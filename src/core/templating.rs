// lib/templating.rs

//! # Templating Module
//!
//! Handles loading, parsing, and rendering SQL query templates from .iql files.
//! Matches the Python `lib/templating.py` implementation.
//!
//! Queries are loaded and parsed eagerly, but rendered lazily (JIT) when
//! actually needed. This avoids errors from templates that reference variables
//! not yet available in the context (e.g., delete queries referencing exports
//! that haven't been computed yet during a build operation).

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process;

use log::{debug, error};
use regex::Regex;

use crate::core::config::prepare_query_context;
use crate::resource::manifest::Resource;
use crate::template::engine::TemplateEngine;

/// Parsed query with its raw template and options.
/// Rendering is deferred until the query is actually needed.
#[derive(Debug, Clone)]
pub struct ParsedQuery {
    pub template: String,
    pub options: QueryOptions,
}

/// Options for a query anchor.
#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    pub retries: u32,
    pub retry_delay: u32,
    pub postdelete_retries: u32,
    pub postdelete_retry_delay: u32,
    /// Dot-path into the RETURNING * result to check before polling
    /// (e.g. `"ProgressEvent.OperationStatus"`).  Only used on `callback`
    /// anchors.
    pub short_circuit_field: Option<String>,
    /// Value of `short_circuit_field` that means polling can be skipped.
    /// Only used on `callback` anchors.
    pub short_circuit_value: Option<String>,
}

/// Parse an anchor line to extract key, numeric options, and string options.
/// Matches Python's `parse_anchor`, extended for callback string params.
///
/// Returns `(key, uint_options, str_options)`.  Numeric-valued params go into
/// `uint_options`; all other params (e.g. `short_circuit_field`,
/// `short_circuit_value`) go into `str_options`.
fn parse_anchor(anchor: &str) -> (String, HashMap<String, u32>, HashMap<String, String>) {
    let parts: Vec<&str> = anchor.split(',').collect();
    let key = parts[0].trim().to_lowercase();
    let mut uint_options: HashMap<String, u32> = HashMap::new();
    let mut str_options: HashMap<String, String> = HashMap::new();

    for part in &parts[1..] {
        if let Some((option_key, option_value)) = part.split_once('=') {
            let k = option_key.trim().to_string();
            let v = option_value.trim().to_string();
            if let Ok(uint_val) = v.parse::<u32>() {
                uint_options.insert(k, uint_val);
            } else {
                str_options.insert(k, v);
            }
        }
    }

    (key, uint_options, str_options)
}

/// Return type of `load_sql_queries`: (templates, uint_options, str_options).
type SqlQueriesResult = (
    HashMap<String, String>,
    HashMap<String, HashMap<String, u32>>,
    HashMap<String, HashMap<String, String>>,
);

/// Load SQL queries from a .iql file, split by anchors.
/// Matches Python's `load_sql_queries`.
fn load_sql_queries(file_path: &Path) -> SqlQueriesResult {
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to read query file {:?}: {}", file_path, e);
            process::exit(1);
        }
    };

    let mut queries: HashMap<String, String> = HashMap::new();
    let mut uint_options: HashMap<String, HashMap<String, u32>> = HashMap::new();
    let mut str_options: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_anchor: Option<String> = None;
    let mut query_buffer: Vec<String> = Vec::new();

    for line in content.lines() {
        if line.trim_start().starts_with("/*+") && line.contains("*/") {
            // Store the current query under the last anchor
            if let Some(ref anchor) = current_anchor {
                if !query_buffer.is_empty() {
                    let (anchor_key, anchor_uint_opts, anchor_str_opts) = parse_anchor(anchor);
                    queries.insert(
                        anchor_key.clone(),
                        query_buffer.join("\n").trim().to_string(),
                    );
                    uint_options.insert(anchor_key.clone(), anchor_uint_opts);
                    str_options.insert(anchor_key, anchor_str_opts);
                    query_buffer.clear();
                }
            }
            // Extract new anchor
            let start = line.find("/*+").unwrap() + 3;
            let end = line.find("*/").unwrap();
            current_anchor = Some(line[start..end].trim().to_string());
        } else {
            query_buffer.push(line.to_string());
        }
    }

    // Store the last query
    if let Some(ref anchor) = current_anchor {
        if !query_buffer.is_empty() {
            let (anchor_key, anchor_uint_opts, anchor_str_opts) = parse_anchor(anchor);
            queries.insert(
                anchor_key.clone(),
                query_buffer.join("\n").trim().to_string(),
            );
            uint_options.insert(anchor_key.clone(), anchor_uint_opts);
            str_options.insert(anchor_key, anchor_str_opts);
        }
    }

    (queries, uint_options, str_options)
}

/// Pre-process Jinja2 inline dict expressions that Tera doesn't support.
///
/// Converts patterns like `{{ { "Key": var, ... } | filter }}` into
/// Tera-compatible form by resolving the dict from context variables
/// and injecting the result as a temporary context variable.
///
/// For example:
///   `{{ { "Description": description, "Path": path } | generate_patch_document }}`
/// becomes:
///   `{{ __inline_dict_0 | generate_patch_document }}`
/// with `__inline_dict_0` set to the constructed JSON object in context.
pub fn preprocess_inline_dicts(template: &str, context: &mut HashMap<String, String>) -> String {
    // Match {{ { ... } | filter_name }}
    // This regex captures the dict body and the filter expression
    let re = Regex::new(r"\{\{\s*\{([^}]*(?:\{[^}]*\}[^}]*)*)\}\s*\|\s*(\w+)\s*\}\}").unwrap();

    let mut result = template.to_string();
    let mut counter = 0;

    // We need to iterate carefully since we're modifying the string
    loop {
        let captures = re.captures(&result);
        if captures.is_none() {
            break;
        }
        let caps = captures.unwrap();
        let full_match = caps.get(0).unwrap();
        let dict_body = caps.get(1).unwrap().as_str().trim();
        let filter_name = caps.get(2).unwrap().as_str();

        // Parse the dict body: "Key": var, "Key2": var2
        let mut obj = serde_json::Map::new();
        for entry in split_dict_entries(dict_body) {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            if let Some((key_part, val_part)) = entry.split_once(':') {
                let key = key_part
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                let var_name = val_part.trim();

                // Look up the variable in context
                let value = context.get(var_name).cloned().unwrap_or_default();

                // Try to parse as JSON, otherwise use as string
                let json_val = match serde_json::from_str::<serde_json::Value>(&value) {
                    Ok(v) => v,
                    Err(_) => serde_json::Value::String(value),
                };
                obj.insert(key, json_val);
            }
        }

        let var_name = format!("__inline_dict_{}", counter);
        let json_str = serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_default();
        context.insert(var_name.clone(), json_str);

        let replacement = format!("{{{{ {} | {} }}}}", var_name, filter_name);
        result = format!(
            "{}{}{}",
            &result[..full_match.start()],
            replacement,
            &result[full_match.end()..]
        );
        counter += 1;
    }

    result
}

/// Split dict entries by commas, but respect nested braces and quoted strings.
fn split_dict_entries(s: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut brace_depth = 0;
    let mut in_quote = false;
    let mut quote_char = ' ';

    for ch in s.chars() {
        match ch {
            '"' | '\'' if !in_quote => {
                in_quote = true;
                quote_char = ch;
                current.push(ch);
            }
            c if in_quote && c == quote_char => {
                in_quote = false;
                current.push(ch);
            }
            '{' if !in_quote => {
                brace_depth += 1;
                current.push(ch);
            }
            '}' if !in_quote => {
                brace_depth -= 1;
                current.push(ch);
            }
            ',' if !in_quote && brace_depth == 0 => {
                entries.push(current.trim().to_string());
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.trim().is_empty() {
        entries.push(current.trim().to_string());
    }
    entries
}

/// Pre-process Jinja2-specific syntax into Tera-compatible equivalents.
/// Handles:
/// - `{{ uuid() }}` -> `{{ uuid }}` (function call to variable)
/// - `replace('x', 'y')` -> `replace(from="x", to="y")` (positional to named args)
fn preprocess_jinja2_compat(template: &str) -> String {
    let mut result = template.to_string();

    // Convert {{ uuid() }} to {{ uuid }}
    let uuid_re = Regex::new(r"\{\{\s*uuid\(\)\s*\}\}").unwrap();
    result = uuid_re.replace_all(&result, "{{ uuid }}").to_string();

    // Convert Jinja2 replace('from', 'to') to Tera replace(from="from", to="to")
    // Matches: replace('x', 'y') or replace("x", "y") with any quoting combo
    let replace_re =
        Regex::new(r#"replace\(\s*['"]([^'"]*)['"]\s*,\s*['"]([^'"]*)['"]\s*\)"#).unwrap();
    result = replace_re
        .replace_all(&result, r#"replace(from="$1", to="$2")"#)
        .to_string();

    result
}

/// Render a single query template with the given context.
/// This is the JIT rendering function called when a query is actually needed.
pub fn render_query(
    engine: &TemplateEngine,
    res_name: &str,
    anchor: &str,
    template: &str,
    context: &HashMap<String, String>,
) -> String {
    let temp_context = prepare_query_context(context);

    let expanded = match preprocess_this_prefix(template, res_name) {
        Ok(t) => t,
        Err(e) => {
            crate::core::utils::catch_error_and_exit(&format!("[{}] [{}] {}", res_name, anchor, e));
        }
    };

    let mut ctx = temp_context;
    let compat_query = preprocess_jinja2_compat(&expanded);
    let processed_query = preprocess_inline_dicts(&compat_query, &mut ctx);

    let template_name = format!("{}__{}", res_name, anchor);
    match engine.render_with_filters(&template_name, &processed_query, &ctx) {
        Ok(rendered) => {
            // Check for unresolved template variables in the final rendered output
            let unresolved_re = Regex::new(r"\{\{[^}]+\}\}").unwrap();
            if let Some(m) = unresolved_re.find(&rendered) {
                crate::core::utils::catch_error_and_exit(&format!(
                    "Unresolved template variable in [{}] [{}]: '{}'\n\nRendered query:\n{}\n",
                    res_name,
                    anchor,
                    m.as_str(),
                    rendered
                ));
            }
            debug!(
                "Rendered [{}] [{}] query:\n\n{}\n",
                res_name, anchor, rendered
            );
            rendered
        }
        Err(e) => {
            error!(
                "Error rendering query for [{}] [{}]: {}",
                res_name, anchor, e
            );

            // Extract template variable references for diagnostics
            let re = Regex::new(r"\{\{\s*(\w+)").unwrap();
            let referenced_vars: Vec<&str> = re
                .captures_iter(&processed_query)
                .filter_map(|c| c.get(1).map(|m| m.as_str()))
                .collect();
            let missing: Vec<&&str> = referenced_vars
                .iter()
                .filter(|v| !ctx.contains_key(**v))
                .collect();

            if !missing.is_empty() {
                error!(
                    "Missing variables in context for [{}] [{}]: {:?}",
                    res_name, anchor, missing
                );
                error!(
                    "Hint: ensure these properties are defined in the manifest for resource [{}], \
                     or that the .iql template only references variables provided by the manifest.",
                    res_name
                );
            }

            debug!(
                "[{}] [{}] available context keys: {:?}",
                res_name,
                anchor,
                ctx.keys().collect::<Vec<_>>()
            );

            crate::core::utils::catch_error_and_exit(&format!(
                "Failed to render query for [{}] [{}]",
                res_name, anchor
            ));
        }
    }
}

/// Try to render a query template, returning None if variables are missing.
/// Used for deferred rendering where this.* fields may not yet be available.
pub fn try_render_query(
    engine: &TemplateEngine,
    res_name: &str,
    anchor: &str,
    template: &str,
    context: &HashMap<String, String>,
) -> Option<String> {
    let temp_context = prepare_query_context(context);

    let expanded = match preprocess_this_prefix(template, res_name) {
        Ok(t) => t,
        Err(_) => return None,
    };

    let mut ctx = temp_context;
    let compat_query = preprocess_jinja2_compat(&expanded);
    let processed_query = preprocess_inline_dicts(&compat_query, &mut ctx);

    let template_name = format!("{}__{}", res_name, anchor);
    match engine.render_with_filters(&template_name, &processed_query, &ctx) {
        Ok(rendered) => {
            // Check for unresolved template variables
            let unresolved_re = Regex::new(r"\{\{[^}]+\}\}").unwrap();
            if unresolved_re.is_match(&rendered) {
                debug!(
                    "Unresolved variables in [{}] [{}], deferring render",
                    res_name, anchor
                );
                return None;
            }
            debug!(
                "Rendered [{}] [{}] query:\n\n{}\n",
                res_name, anchor, rendered
            );
            Some(rendered)
        }
        Err(_) => None,
    }
}

/// Get queries for a resource: load from file, parse anchors.
/// Templates are NOT rendered here — rendering is deferred to when
/// each query is actually needed (JIT rendering).
/// Matches Python's `get_queries`.
///
/// Callback anchors (e.g. `callback:create`, `callback:delete`) are stored
/// under the key `"callback:create"`, `"callback:delete"`, etc.  A bare
/// `callback` anchor (no operation qualifier) is stored under `"callback"`.
pub fn get_queries(
    _engine: &TemplateEngine,
    stack_dir: &str,
    resource: &Resource,
    _full_context: &HashMap<String, String>,
) -> HashMap<String, ParsedQuery> {
    let mut result = HashMap::new();

    let template_path = if let Some(ref file) = resource.file {
        Path::new(stack_dir).join("resources").join(file)
    } else {
        Path::new(stack_dir)
            .join("resources")
            .join(format!("{}.iql", resource.name))
    };

    if !template_path.exists() {
        error!("Query file not found: {:?}", template_path);
        process::exit(1);
    }

    let (query_templates, query_uint_options, query_str_options) = load_sql_queries(&template_path);

    for (anchor, template) in &query_templates {
        // Fix backward compatibility for preflight and postdeploy.
        // Callback anchors (callback:create, callback:delete, callback:update,
        // callback) are passed through unchanged.
        let normalized_anchor = match anchor.as_str() {
            "preflight" => "exists".to_string(),
            "postdeploy" => "statecheck".to_string(),
            other => other.to_string(),
        };

        let uint_opts = query_uint_options.get(anchor).cloned().unwrap_or_default();
        let str_opts = query_str_options.get(anchor).cloned().unwrap_or_default();

        result.insert(
            normalized_anchor.clone(),
            ParsedQuery {
                template: template.clone(),
                options: QueryOptions {
                    retries: *uint_opts.get("retries").unwrap_or(&1),
                    retry_delay: *uint_opts.get("retry_delay").unwrap_or(&0),
                    postdelete_retries: *uint_opts.get("postdelete_retries").unwrap_or(&10),
                    postdelete_retry_delay: *uint_opts.get("postdelete_retry_delay").unwrap_or(&5),
                    short_circuit_field: str_opts.get("short_circuit_field").cloned(),
                    short_circuit_value: str_opts.get("short_circuit_value").cloned(),
                },
            },
        );
    }

    debug!(
        "Queries for [{}]: {:?}",
        resource.name,
        result.keys().collect::<Vec<_>>()
    );
    result
}

/// Pre-process `this.` prefix inside Tera template blocks.
///
/// Within every `{{ ... }}` and `{% ... %}` block, replaces `this.` with
/// `{resource_name}.`, allowing resource-scoped variables to be referenced
/// unambiguously inside a resource's own `.iql` file.
///
/// Returns `Err` with a diagnostic if `this.` appears but `resource_name`
/// is empty (i.e. no active resource context, such as a global template).
pub fn preprocess_this_prefix(template: &str, resource_name: &str) -> Result<String, String> {
    if !template.contains("this.") {
        return Ok(template.to_string());
    }

    if resource_name.is_empty() {
        return Err(
            "Template uses 'this.' prefix but no resource context is active; \
             'this.' is only valid inside a resource's .iql file."
                .to_string(),
        );
    }

    let replacement = format!("{}.", resource_name);

    // Replace 'this.' with '{resource_name}.' inside {{ ... }} blocks.
    let var_re = Regex::new(r"(?s)\{\{(.*?)\}\}").unwrap();
    let with_vars = var_re.replace_all(template, |caps: &regex::Captures| {
        let inner = caps[1].replace("this.", &replacement);
        format!("{{{{{}}}}}", inner)
    });

    // Also handle {% ... %} tag blocks (conditionals, loops).
    let tag_re = Regex::new(r"(?s)\{%(.*?)%\}").unwrap();
    let with_tags = tag_re.replace_all(&with_vars, |caps: &regex::Captures| {
        let inner = caps[1].replace("this.", &replacement);
        format!("{{%{}%}}", inner)
    });

    Ok(with_tags.to_string())
}

/// Render an inline SQL template string.
/// Matches Python's `render_inline_template`.
pub fn render_inline_template(
    engine: &TemplateEngine,
    resource_name: &str,
    template_string: &str,
    full_context: &HashMap<String, String>,
) -> String {
    debug!(
        "[{}] inline template:\n\n{}\n",
        resource_name, template_string
    );

    let mut temp_context = prepare_query_context(full_context);

    let expanded = match preprocess_this_prefix(template_string, resource_name) {
        Ok(t) => t,
        Err(e) => {
            error!("[{}] inline template: {}", resource_name, e);
            process::exit(1);
        }
    };

    let compat = preprocess_jinja2_compat(&expanded);
    let processed = preprocess_inline_dicts(&compat, &mut temp_context);
    let template_name = format!("{}__inline", resource_name);

    match engine.render_with_filters(&template_name, &processed, &temp_context) {
        Ok(rendered) => {
            debug!(
                "[{}] rendered inline template:\n\n{}\n",
                resource_name, rendered
            );
            rendered
        }
        Err(e) => {
            error!(
                "Error rendering inline template for [{}]: {}",
                resource_name, e
            );

            let re = Regex::new(r"\{\{\s*(\w+)").unwrap();
            let referenced_vars: Vec<&str> = re
                .captures_iter(&processed)
                .filter_map(|c| c.get(1).map(|m| m.as_str()))
                .collect();
            let missing: Vec<&&str> = referenced_vars
                .iter()
                .filter(|v| !temp_context.contains_key(**v))
                .collect();

            if !missing.is_empty() {
                error!(
                    "Missing variables in context for [{}]: {:?}",
                    resource_name, missing
                );
                error!(
                    "Hint: ensure these properties are defined in the manifest for resource [{}], \
                     or that the inline SQL only references variables provided by the manifest.",
                    resource_name
                );
            }

            debug!(
                "[{}] available context keys: {:?}",
                resource_name,
                temp_context.keys().collect::<Vec<_>>()
            );

            process::exit(1);
        }
    }
}

/// Tolerant variant of [`render_inline_template`]: returns `None` when the
/// inline SQL references variables that are not in the context, instead of
/// exiting the process.
///
/// Used by `teardown`, where an upstream export may legitimately be missing
/// (the resource that produced it is already gone, or was skipped via
/// `skip_on_delete`) and the correct behaviour is to skip the query.
pub fn try_render_inline_template(
    engine: &TemplateEngine,
    resource_name: &str,
    template_string: &str,
    full_context: &HashMap<String, String>,
) -> Option<String> {
    let mut temp_context = prepare_query_context(full_context);

    let expanded = preprocess_this_prefix(template_string, resource_name).ok()?;
    let compat = preprocess_jinja2_compat(&expanded);
    let processed = preprocess_inline_dicts(&compat, &mut temp_context);
    let template_name = format!("{}__inline", resource_name);

    match engine.render_with_filters(&template_name, &processed, &temp_context) {
        Ok(rendered) => {
            let unresolved_re = Regex::new(r"\{\{[^}]+\}\}").unwrap();
            if unresolved_re.is_match(&rendered) {
                debug!(
                    "Unresolved variables in [{}] inline template, deferring render",
                    resource_name
                );
                return None;
            }
            debug!(
                "[{}] rendered inline template:\n\n{}\n",
                resource_name, rendered
            );
            Some(rendered)
        }
        Err(e) => {
            debug!(
                "[{}] inline template could not be rendered: {}",
                resource_name, e
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::engine::TemplateEngine;

    // ── try_render_inline_template unit tests ─────────────────────────────

    #[test]
    fn test_try_render_inline_template_renders_when_context_complete() {
        let engine = TemplateEngine::new();
        let mut ctx = HashMap::new();
        ctx.insert("deployment_name".to_string(), "dbc-1234".to_string());
        let rendered = try_render_inline_template(
            &engine,
            "workspace_ready",
            "SELECT userName FROM databricks_workspace.iam.current_user \
             WHERE deployment_name = '{{ deployment_name }}'",
            &ctx,
        );
        assert_eq!(
            rendered.as_deref(),
            Some(
                "SELECT userName FROM databricks_workspace.iam.current_user \
                 WHERE deployment_name = 'dbc-1234'"
            )
        );
    }

    #[test]
    fn test_try_render_inline_template_returns_none_when_variable_missing() {
        let engine = TemplateEngine::new();
        let ctx: HashMap<String, String> = HashMap::new();
        let rendered = try_render_inline_template(
            &engine,
            "workspace_ready",
            "SELECT 1 WHERE deployment_name = '{{ deployment_name }}'",
            &ctx,
        );
        assert!(rendered.is_none());
    }

    // ── preprocess_this_prefix unit tests ─────────────────────────────────

    #[test]
    fn test_preprocess_this_prefix_basic_rewrite() {
        let result = preprocess_this_prefix("{{ this.fred }}", "resource_name_x").unwrap();
        assert_eq!(result, "{{ resource_name_x.fred }}");
    }

    #[test]
    fn test_preprocess_this_prefix_noop_when_no_this() {
        let template = "{{ fred }}";
        let result = preprocess_this_prefix(template, "resource_name_x").unwrap();
        assert_eq!(
            result, template,
            "template without 'this.' should be unchanged"
        );
    }

    #[test]
    fn test_preprocess_this_prefix_error_when_no_resource_name() {
        let result = preprocess_this_prefix("{{ this.fred }}", "");
        assert!(result.is_err(), "empty resource_name should return Err");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("this.") || msg.contains("resource context"),
            "error message should mention 'this.' or resource context, got: {}",
            msg
        );
    }

    #[test]
    fn test_preprocess_this_prefix_multiple_occurrences() {
        let template = "{{ this.a }} and {{ this.b }}";
        let result = preprocess_this_prefix(template, "my_res").unwrap();
        assert_eq!(result, "{{ my_res.a }} and {{ my_res.b }}");
    }

    #[test]
    fn test_preprocess_this_prefix_deep_path() {
        let template = "{{ this.callback.ProgressEvent.RequestToken }}";
        let result = preprocess_this_prefix(template, "resource_name_x").unwrap();
        assert_eq!(
            result,
            "{{ resource_name_x.callback.ProgressEvent.RequestToken }}"
        );
    }

    #[test]
    fn test_preprocess_this_prefix_in_tag_block() {
        let template = "{% if this.flag %}yes{% endif %}";
        let result = preprocess_this_prefix(template, "res").unwrap();
        assert_eq!(result, "{% if res.flag %}yes{% endif %}");
    }

    #[test]
    fn test_preprocess_this_prefix_with_filter() {
        let template = "{{ this.tags | from_json }}";
        let result = preprocess_this_prefix(template, "my_vpc").unwrap();
        assert_eq!(result, "{{ my_vpc.tags | from_json }}");
    }

    // ── End-to-end rendering tests via TemplateEngine ─────────────────────

    #[test]
    fn test_this_resolves_resource_scoped_over_global() {
        // When both a global 'fred' and a resource-scoped 'resource_name_x.fred'
        // exist, {{ this.fred }} must resolve to the resource-scoped value.
        let engine = TemplateEngine::new();
        let mut context = std::collections::HashMap::new();
        context.insert("fred".to_string(), "global_fred".to_string());
        context.insert(
            "resource_name_x.fred".to_string(),
            "scoped_fred".to_string(),
        );

        let expanded = preprocess_this_prefix("{{ this.fred }}", "resource_name_x").unwrap();
        let result = engine
            .render_with_filters("t", &expanded, &context)
            .unwrap();
        assert_eq!(
            result, "scoped_fred",
            "this.fred should resolve to the resource-scoped value, not the global"
        );
    }

    #[test]
    fn test_this_resolves_when_only_resource_scoped_exists() {
        // No global 'fred' - only the resource-scoped one.
        let engine = TemplateEngine::new();
        let mut context = std::collections::HashMap::new();
        context.insert(
            "resource_name_x.fred".to_string(),
            "scoped_only".to_string(),
        );

        let expanded = preprocess_this_prefix("{{ this.fred }}", "resource_name_x").unwrap();
        let result = engine
            .render_with_filters("t", &expanded, &context)
            .unwrap();
        assert_eq!(result, "scoped_only");
    }

    #[test]
    fn test_this_errors_when_only_global_exists_not_resource_scoped() {
        // this.fred expands to resource_name_x.fred; if only a global 'fred'
        // exists the render should fail rather than silently using the global.
        let engine = TemplateEngine::new();
        let mut context = std::collections::HashMap::new();
        context.insert("fred".to_string(), "global_fred".to_string());
        // No resource_name_x.fred in context

        let expanded = preprocess_this_prefix("{{ this.fred }}", "resource_name_x").unwrap();
        let result = engine.render_with_filters("t", &expanded, &context);
        assert!(
            result.is_err(),
            "this.fred should error when resource_name_x.fred is not in context"
        );
    }

    #[test]
    fn test_this_callback_resolves_same_as_scoped_and_shorthand() {
        // {{ this.callback.ProgressEvent.RequestToken }} inside resource_name_x
        // should resolve identically to:
        //   {{ resource_name_x.callback.ProgressEvent.RequestToken }}  (explicit)
        //   {{ callback.ProgressEvent.RequestToken }}                  (shorthand)
        let engine = TemplateEngine::new();
        let mut context = std::collections::HashMap::new();
        context.insert(
            "resource_name_x.callback.ProgressEvent.RequestToken".to_string(),
            "token-abc".to_string(),
        );
        context.insert(
            "callback.ProgressEvent.RequestToken".to_string(),
            "token-abc".to_string(),
        );

        let expanded = preprocess_this_prefix(
            "{{ this.callback.ProgressEvent.RequestToken }}",
            "resource_name_x",
        )
        .unwrap();

        let via_this = engine
            .render_with_filters("t1", &expanded, &context)
            .unwrap();
        let via_explicit = engine
            .render_with_filters(
                "t2",
                "{{ resource_name_x.callback.ProgressEvent.RequestToken }}",
                &context,
            )
            .unwrap();
        let via_shorthand = engine
            .render_with_filters("t3", "{{ callback.ProgressEvent.RequestToken }}", &context)
            .unwrap();

        assert_eq!(via_this, "token-abc");
        assert_eq!(
            via_this, via_explicit,
            "this.callback should equal resource_name_x.callback"
        );
        assert_eq!(
            via_this, via_shorthand,
            "this.callback should equal shorthand callback"
        );
    }
}
