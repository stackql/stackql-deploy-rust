// lib/utils.rs

//! # Utility Functions
//!
//! Low-level StackQL execution utilities, retry logic, export handling,
//! provider management, and script execution.
//! Matches the Python `lib/utils.py` implementation.

use std::collections::HashMap;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};

use crate::core::errors::check_fatal_error;
use crate::utils::pgwire::PgwireLite;
use crate::utils::query::{execute_query, QueryResult};

/// Exit with error message. Matches Python's `catch_error_and_exit`.
pub fn catch_error_and_exit(msg: &str) -> ! {
    error!("{}", msg);
    // Stop the local server before exiting to avoid stale sessions
    crate::utils::server::stop_local_server();
    crate::utils::display::print_unicode_box(
        "stackql-deploy operation failed",
        crate::utils::display::BorderColor::Red,
    );
    process::exit(1);
}

/// Execute a StackQL SELECT query with retry logic.
/// Returns rows as Vec<HashMap<String, String>>.
/// Matches Python's `run_stackql_query`.
pub fn run_stackql_query(
    query: &str,
    client: &mut PgwireLite,
    suppress_errors: bool,
    retries: u32,
    delay: u32,
) -> Vec<HashMap<String, String>> {
    let mut attempt = 0;
    let mut last_error: Option<String> = None;

    while attempt <= retries {
        match execute_query(query, client) {
            Ok(result) => match result {
                QueryResult::Data {
                    columns,
                    rows,
                    notices,
                } => {
                    // Check for error notices
                    for notice in &notices {
                        if notice.contains("error") || notice.starts_with("ERROR") {
                            last_error = Some(notice.clone());
                            if !suppress_errors && attempt == retries {
                                catch_error_and_exit(&format!(
                                    "Error during stackql query execution:\n\n{}\n",
                                    notice
                                ));
                            }
                        }
                    }

                    if rows.is_empty() {
                        debug!("Query returned no results");
                        if attempt < retries {
                            thread::sleep(Duration::from_secs(delay as u64));
                            attempt += 1;
                            continue;
                        }
                        return Vec::new();
                    }

                    // Convert to Vec<HashMap>
                    let col_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();

                    let result_maps: Vec<HashMap<String, String>> = rows
                        .iter()
                        .map(|row| {
                            let mut map = HashMap::new();
                            for (i, col_name) in col_names.iter().enumerate() {
                                let value = row
                                    .values
                                    .get(i)
                                    .cloned()
                                    .unwrap_or_else(|| "NULL".to_string());
                                map.insert(col_name.clone(), value);
                            }
                            map
                        })
                        .collect();

                    // Check for error in results
                    if !result_maps.is_empty() {
                        if let Some(err) = result_maps[0].get("error") {
                            last_error = Some(err.clone());
                            // Check for fatal errors even when suppressing
                            if let Some(pattern) = check_fatal_error(err) {
                                catch_error_and_exit(&format!(
                                    "Fatal error (matched '{}'):\n\n{}\n",
                                    pattern, err
                                ));
                            }
                            if !suppress_errors {
                                if attempt == retries {
                                    catch_error_and_exit(&format!(
                                        "Error during stackql query execution:\n\n{}\n",
                                        err
                                    ));
                                } else {
                                    error!("Attempt {} failed:\n\n{}\n", attempt + 1, err);
                                }
                            }
                            thread::sleep(Duration::from_secs(delay as u64));
                            attempt += 1;
                            continue;
                        }

                        // Check for count query
                        if let Some(count_str) = result_maps[0].get("count") {
                            if let Ok(json) = serde_json::to_string_pretty(&result_maps) {
                                debug!(
                                    "Stackql query executed successfully, count: {}\n\nresults:\n\n{}\n",
                                    count_str, json
                                );
                            }
                            if let Ok(count) = count_str.parse::<i64>() {
                                if count > 1 {
                                    catch_error_and_exit(&format!(
                                        "Detected more than one resource matching query criteria, expected 0 or 1, got {}",
                                        count
                                    ));
                                }
                            }
                            return result_maps;
                        }
                    }

                    if let Ok(json) = serde_json::to_string_pretty(&result_maps) {
                        debug!(
                            "Stackql query executed successfully, retrieved {} items.\n\nresults:\n\n{}\n",
                            result_maps.len(), json
                        );
                    }
                    return result_maps;
                }
                QueryResult::Command(msg) => {
                    debug!("Command result: {}", msg);
                    return Vec::new();
                }
                QueryResult::Empty => {
                    debug!("Query returned no results");
                    if attempt < retries {
                        thread::sleep(Duration::from_secs(delay as u64));
                        attempt += 1;
                        continue;
                    }
                    return Vec::new();
                }
            },
            Err(e) => {
                last_error = Some(e.clone());
                debug!("Query error on attempt {}: {}", attempt + 1, e);
                // Check for fatal errors (network, auth) that should not be retried
                if let Some(pattern) = check_fatal_error(&e) {
                    catch_error_and_exit(&format!(
                        "Fatal error (matched '{}'):\n\n{}\n",
                        pattern, e
                    ));
                }
                if attempt == retries && !suppress_errors {
                    catch_error_and_exit(&format!(
                        "Exception during stackql query execution:\n\n{}\n",
                        e
                    ));
                }
            }
        }

        thread::sleep(Duration::from_secs(delay as u64));
        attempt += 1;
    }

    // If suppress_errors and we have an error, return error marker
    if suppress_errors {
        if let Some(err) = last_error {
            let mut error_map = HashMap::new();
            error_map.insert("_stackql_deploy_error".to_string(), err);
            return vec![error_map];
        }
    }

    Vec::new()
}

/// Execute a StackQL DML/DDL command with retry logic.
/// Matches Python's `run_stackql_command`.
pub fn run_stackql_command(
    command: &str,
    client: &mut PgwireLite,
    ignore_errors: bool,
    retries: u32,
    retry_delay: u32,
) -> String {
    let mut attempt = 0;

    // Handle REGISTRY PULL command format
    let processed_command = if command.starts_with("REGISTRY PULL") {
        let re = regex::Regex::new(r"(REGISTRY PULL \w+)(::v[\d\.]+)?").unwrap();
        if let Some(caps) = re.captures(command) {
            let provider = caps.get(1).map_or("", |m| m.as_str());
            if let Some(version) = caps.get(2) {
                format!("{} {}", provider, &version.as_str()[2..])
            } else {
                command.to_string()
            }
        } else {
            command.to_string()
        }
    } else {
        command.to_string()
    };

    while attempt <= retries {
        match execute_query(&processed_command, client) {
            Ok(result) => {
                match result {
                    QueryResult::Data {
                        notices,
                        columns,
                        rows,
                    } => {
                        // Check for errors in notices
                        for notice in &notices {
                            if error_detected_in_notice(notice) && ignore_errors {
                                warn!(
                                    "Command returned an error notice (ignored):\n\n{}\n",
                                    notice
                                );
                                continue;
                            }
                            if error_detected_in_notice(notice) && !ignore_errors {
                                if attempt < retries {
                                    debug!(
                                        "Command notice on attempt {}/{}, retrying in {} seconds: {}",
                                        attempt + 1, retries + 1, retry_delay, notice
                                    );
                                    thread::sleep(Duration::from_secs(retry_delay as u64));
                                    attempt += 1;
                                    continue;
                                } else {
                                    catch_error_and_exit(&format!(
                                        "Error during stackql command execution:\n\n{}\n\nlast rendered query:\n\n{}\n",
                                        notice, processed_command
                                    ));
                                }
                            }
                        }
                        // Log returned data (e.g. from RETURNING clause) at debug level
                        if !rows.is_empty() {
                            let col_names: Vec<&str> =
                                columns.iter().map(|c| c.name.as_str()).collect();
                            let result_maps: Vec<HashMap<String, String>> = rows
                                .iter()
                                .map(|row| {
                                    col_names
                                        .iter()
                                        .enumerate()
                                        .map(|(i, &name)| {
                                            let val =
                                                row.values.get(i).cloned().unwrap_or_default();
                                            (name.to_string(), val)
                                        })
                                        .collect()
                                })
                                .collect();
                            if let Ok(json) = serde_json::to_string_pretty(&result_maps) {
                                debug!("Command returned data:\n\n{}\n", json);
                            }
                        }
                        let msg = notices.join("\n");
                        if !msg.is_empty() {
                            debug!("Command notices:\n\n{}\n", msg);
                        }
                        return msg;
                    }
                    QueryResult::Command(msg) => {
                        debug!("Stackql command executed successfully:\n\n{}\n", msg);
                        return msg;
                    }
                    QueryResult::Empty => {
                        debug!("Command executed with empty result");
                        return String::new();
                    }
                }
            }
            Err(e) => {
                // Check for fatal errors (network, auth) before retrying
                if let Some(pattern) = check_fatal_error(&e) {
                    catch_error_and_exit(&format!(
                        "Fatal error (matched '{}'):\n\n{}\n",
                        pattern, e
                    ));
                }
                if !ignore_errors {
                    if attempt < retries {
                        debug!(
                            "Command returned error on attempt {}/{}, retrying in {} seconds: {}",
                            attempt + 1,
                            retries + 1,
                            retry_delay,
                            e
                        );
                        thread::sleep(Duration::from_secs(retry_delay as u64));
                        attempt += 1;
                        continue;
                    }
                    catch_error_and_exit(&format!(
                        "Exception during stackql command execution:\n\n{}\n",
                        e
                    ));
                } else {
                    warn!("Command failed (ignored):\n\n{}\n", e);
                    return String::new();
                }
            }
        }
    }

    String::new()
}

/// Check if a notice/message indicates an error.
///
/// Patterns can appear either at the start of the notice message or inside
/// the `DETAIL:` payload (stackql wraps provider errors as a generic "a
/// notice level event has occurred" message with the real HTTP status in
/// the detail), so match against the whole notice string.
fn error_detected_in_notice(msg: &str) -> bool {
    msg.contains("http response status code: 4")
        || msg.contains("http response status code: 5")
        || msg.starts_with("error:")
        || msg.contains("\nDETAIL: error:")
        || msg.starts_with("disparity in fields to insert")
        || msg.starts_with("cannot find matching operation")
}

/// Run a test query and check if count == 1 (exists) or count == 0 (deleted).
/// Matches Python's `run_test`.
pub fn run_test(
    resource_name: &str,
    query: &str,
    client: &mut PgwireLite,
    delete_test: bool,
) -> bool {
    run_test_with_fields(resource_name, query, client, delete_test).0
}

/// Run a test query and capture any non-count fields from the result.
///
/// Returns `(bool, Option<HashMap<String, String>>)`:
/// - The bool indicates whether the test passed (resource exists / is deleted).
/// - If the exists query returns fields OTHER than `count`, those fields are
///   captured and returned so the caller can inject them into the template
///   context (e.g. as `{{ this.identifier }}`).
pub fn run_test_with_fields(
    resource_name: &str,
    query: &str,
    client: &mut PgwireLite,
    delete_test: bool,
) -> (bool, Option<HashMap<String, String>>) {
    let result = run_stackql_query(query, client, true, 0, 5);

    if result.is_empty() {
        if delete_test {
            debug!("Delete test result true for [{}]", resource_name);
            return (true, None);
        } else {
            debug!("Test result false for [{}]", resource_name);
            return (false, None);
        }
    }

    // Check for error markers
    if result[0].contains_key("_stackql_deploy_error") || result[0].contains_key("error") {
        if delete_test {
            return (true, None);
        }
        return (false, None);
    }

    if let Some(count_str) = result[0].get("count") {
        if let Ok(count) = count_str.parse::<i64>() {
            if delete_test {
                if count == 0 {
                    debug!("Delete test result true for [{}]", resource_name);
                    return (true, None);
                } else {
                    debug!(
                        "Delete test result false for [{}], expected 0 got {}",
                        resource_name, count
                    );
                    return (false, None);
                }
            } else if count == 1 {
                debug!("Test result true for [{}]", resource_name);
                // Capture any extra fields beyond "count"
                let extra = extract_non_count_fields(&result[0]);
                return (true, extra);
            } else {
                debug!(
                    "Test result false for [{}], expected 1 got {}",
                    resource_name, count
                );
                return (false, None);
            }
        }
    }

    // If no count field, for non-delete test consider any result as exists
    // and capture all returned fields.
    // However, if multiple rows are returned this is a fatal error — the
    // exists (identifier) query must return exactly 0 or 1 rows.
    if !delete_test && result.len() > 1 {
        catch_error_and_exit(&format!(
            "Exists query for [{}] returned {} rows (expected 0 or 1). \
             This indicates an ambiguous resource identifier — fix the \
             exists query or tag configuration so it returns a single row.",
            resource_name,
            result.len()
        ));
    }

    // However, if all non-trivial field values are "null" or empty, treat
    // as "does not exist" (e.g. a CASE WHEN that returned NULL).
    if !delete_test && !result.is_empty() {
        let row = &result[0];
        let all_null = row.values().all(|v| v == "null" || v.is_empty());
        if all_null {
            debug!(
                "Test result false for [{}]: all field values are null/empty",
                resource_name
            );
            return (false, None);
        }
        let fields = Some(row.clone());
        return (true, fields);
    }

    (false, None)
}

/// Extract fields from an exists query result row, excluding the `count` field.
/// Returns `Some(map)` if there are non-count fields, `None` otherwise.
fn extract_non_count_fields(row: &HashMap<String, String>) -> Option<HashMap<String, String>> {
    let extra: HashMap<String, String> = row
        .iter()
        .filter(|(k, _)| k.as_str() != "count")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if extra.is_empty() {
        None
    } else {
        Some(extra)
    }
}

/// Perform retries on a test query.
/// Matches Python's `perform_retries`.
pub fn perform_retries(
    resource_name: &str,
    query: &str,
    retries: u32,
    delay: u32,
    client: &mut PgwireLite,
    delete_test: bool,
) -> bool {
    perform_retries_with_fields(resource_name, query, retries, delay, client, delete_test).0
}

/// Perform retries on a test query, capturing any non-count fields from the result.
pub fn perform_retries_with_fields(
    resource_name: &str,
    query: &str,
    retries: u32,
    delay: u32,
    client: &mut PgwireLite,
    delete_test: bool,
) -> (bool, Option<HashMap<String, String>>) {
    let start = Instant::now();
    let mut attempt = 0;

    while attempt < retries {
        let (result, fields) = run_test_with_fields(resource_name, query, client, delete_test);
        if result {
            return (true, fields);
        }
        let elapsed = start.elapsed().as_secs();
        info!(
            "attempt {}/{}: retrying in {} seconds ({} seconds elapsed).",
            attempt + 1,
            retries,
            delay,
            elapsed
        );
        thread::sleep(Duration::from_secs(delay as u64));
        attempt += 1;
    }

    // Pre-create exists checks run with retries == 1 as a "fast fail" where
    // a negative result is the expected outcome (the resource does not yet
    // exist and needs to be created). Only surface the query when the
    // caller configured real retries — i.e. a statecheck, exports proxy,
    // or post-deploy exists check where exhaustion signals a stack failure.
    if retries > 1 {
        warn!(
            "retries exhausted for [{}], last rendered query:\n\n{}\n",
            resource_name, query
        );
    }
    (false, None)
}

/// Show a query in logs if show_queries is enabled.
pub fn show_query(show_queries: bool, query: &str) {
    if show_queries {
        info!("query:\n\n{}\n", query);
    }
}

/// Pull providers using the StackQL server.
/// Matches Python's `pull_providers`.
pub fn pull_providers(providers: &[String], client: &mut PgwireLite) {
    let installed = run_stackql_query("SHOW PROVIDERS", client, false, 0, 5);

    for provider in providers {
        if provider.contains("::") {
            // Versioned provider
            let parts: Vec<&str> = provider.splitn(2, "::").collect();
            let name = parts[0];
            let version = parts[1];

            let found = installed.iter().any(|p| {
                p.get("name").is_some_and(|n| n == name)
                    && p.get("version").is_some_and(|v| v == version)
            });

            if found {
                info!("Provider '{}' is already installed.", provider);
            } else {
                // Check if a higher version is installed
                let higher_installed = installed.iter().any(|p| {
                    p.get("name").is_some_and(|n| n == name)
                        && p.get("version")
                            .is_some_and(|v| is_version_higher(v, version))
                });

                if higher_installed {
                    info!(
                        "Provider '{}' - a higher version is already installed.",
                        provider
                    );
                } else {
                    info!("Pulling provider '{}'...", provider);
                    let cmd = format!("REGISTRY PULL {}", provider);
                    let msg = run_stackql_command(&cmd, client, false, 0, 5);
                    if !msg.is_empty() {
                        info!("{}", msg);
                    }
                }
            }
        } else {
            let found = installed.iter().any(|p| p.get("name") == Some(provider));

            if found {
                info!("Provider '{}' is already installed.", provider);
            } else {
                info!("Pulling provider '{}'...", provider);
                let cmd = format!("REGISTRY PULL {}", provider);
                let msg = run_stackql_command(&cmd, client, false, 0, 5);
                if !msg.is_empty() {
                    info!("{}", msg);
                }
            }
        }
    }
}

/// Compare version strings. Returns true if installed > requested.
fn is_version_higher(installed: &str, requested: &str) -> bool {
    let parse = |v: &str| -> u64 { v.replace(['v', '.'], "").parse::<u64>().unwrap_or(0) };
    parse(installed) > parse(requested)
}

/// Placeholder assigned to an export during `teardown` when its value could
/// not be collected: the exports query returned no rows (the upstream
/// resource may already be gone), the query could not be rendered, or the
/// resource was skipped via `skip_on_delete`.
///
/// Downstream queries that would interpolate this placeholder must not be
/// executed - see [`references_unknown_export`].
pub const UNKNOWN_EXPORT_PLACEHOLDER: &str = "<unknown>";

/// Placeholder assigned to an export during `--dry-run`, where no query is
/// executed and the real value is not known.
pub const DRY_RUN_EXPORT_PLACEHOLDER: &str = "<evaluated>";

/// True for the framework's own export placeholders. These are never real
/// values and must not be registered for log redaction: doing so would mask
/// every other placeholder in the run as if it were a secret.
pub fn is_export_placeholder(value: &str) -> bool {
    value == UNKNOWN_EXPORT_PLACEHOLDER || value == DRY_RUN_EXPORT_PLACEHOLDER
}

/// Returns true when a rendered query contains the teardown
/// [`UNKNOWN_EXPORT_PLACEHOLDER`], i.e. it references an export whose value
/// could not be collected.
///
/// Such a query is never useful to run: at best it matches nothing, at worst
/// the placeholder lands in a hostname or identifier and the provider call
/// fails with a fatal, non-retryable error (for example
/// `dial tcp: lookup <unknown>.cloud.databricks.com: no such host`), which
/// aborts the whole teardown.
pub fn references_unknown_export(rendered_query: &str) -> bool {
    rendered_query.contains(UNKNOWN_EXPORT_PLACEHOLDER)
}

/// Build the fallback export map for a resource during teardown: every
/// declared export (plain `name` or `{ column: name }` mapping) is set to
/// [`UNKNOWN_EXPORT_PLACEHOLDER`].
pub fn unknown_exports_for(expected_exports: &[serde_yaml::Value]) -> HashMap<String, String> {
    let mut fallback = HashMap::new();
    for item in expected_exports {
        if let Some(s) = item.as_str() {
            fallback.insert(s.to_string(), UNKNOWN_EXPORT_PLACEHOLDER.to_string());
        } else if let Some(map) = item.as_mapping() {
            for (_, val) in map {
                if let Some(v) = val.as_str() {
                    fallback.insert(v.to_string(), UNKNOWN_EXPORT_PLACEHOLDER.to_string());
                }
            }
        }
    }
    fallback
}

/// Update global context with exported values.
///
/// Each export is stored under two keys:
///
/// - **`{key}`** — the global (unscoped) key.  This can be overridden by a
///   subsequent resource that exports a variable with the same name, so it
///   always reflects the *most recent* export value.
///
/// - **`{resource_name}.{key}`** — the resource-scoped (fully qualified) key.
///   This is written **once** and never overwritten, so it is immutable once
///   set.  Consumers that need an unambiguous reference should use this form.
///
/// Matches Python's `export_vars`.
pub fn export_vars(
    global_context: &mut HashMap<String, String>,
    resource_name: &str,
    export_data: &HashMap<String, String>,
    protected_exports: &[String],
) {
    for (key, value) in export_data {
        let is_protected = protected_exports.contains(key);
        if is_protected && !is_export_placeholder(value) {
            // Register for global log redaction so the value is also masked
            // anywhere else it surfaces (e.g. interpolated into a downstream
            // resource's query shown via --dry-run or --show-queries).
            crate::core::secrets::register_secret(value);
        }
        let display_value = if is_protected {
            "*".repeat(value.len())
        } else {
            value.clone()
        };

        // --- resource-scoped key (immutable: only written if not already set) ---
        let scoped_key = format!("{}.{}", resource_name, key);
        global_context.entry(scoped_key.clone()).or_insert_with(|| {
            debug!(
                "set {} [{}] to [{}] in exports",
                if is_protected {
                    "protected variable"
                } else {
                    "variable"
                },
                scoped_key,
                display_value,
            );
            value.clone()
        });

        // --- global (unscoped) key (can be overridden by later resources) ---
        info!(
            "set {} [{}] to [{}] in exports",
            if is_protected {
                "protected variable"
            } else {
                "variable"
            },
            key,
            display_value,
        );
        global_context.insert(key.clone(), value.clone());
    }
}

/// Check if exports result can serve as a statecheck proxy.
/// Returns true if result is non-empty and has no errors.
/// Matches Python's `check_exports_as_statecheck_proxy`.
pub fn check_exports_as_statecheck_proxy(exports_result: &[HashMap<String, String>]) -> bool {
    debug!(
        "Checking exports result as statecheck proxy: {} rows",
        exports_result.len()
    );

    if exports_result.is_empty() {
        debug!("Empty exports result, treating as statecheck failure");
        return false;
    }

    // Check for error conditions
    if exports_result[0].contains_key("_stackql_deploy_error") {
        debug!("Error in exports result, treating as statecheck failure");
        return false;
    }
    if exports_result[0].contains_key("error") {
        debug!("Error in exports result, treating as statecheck failure");
        return false;
    }

    debug!("Valid exports result, treating as statecheck success");
    true
}

/// Check if all items in exports list are dicts (HashMap-like).
/// In Rust, exports from YAML can be strings or maps.
/// Matches Python's `check_all_dicts`.
pub fn check_all_dicts(items: &[serde_yaml::Value]) -> bool {
    if items.is_empty() {
        return false;
    }
    items.iter().all(|item| item.is_mapping())
}

/// Run an external script and capture output.
/// Matches Python's `run_ext_script`.
pub fn run_ext_script(
    cmd: &str,
    expected_exports: Option<&[String]>,
) -> Option<HashMap<String, String>> {
    debug!("Running external script: {}", cmd);

    let output = match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(output) => output,
        Err(e) => {
            catch_error_and_exit(&format!("Script failed: {}", e));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    debug!("Script output: {}", stdout);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        catch_error_and_exit(&format!(
            "Script failed with status {:?}: {}",
            output.status.code(),
            stderr
        ));
    }

    match expected_exports {
        Some(exports) if !exports.is_empty() => {
            match serde_json::from_str::<HashMap<String, String>>(&stdout) {
                Ok(exported_vars) => {
                    for export in exports {
                        if !exported_vars.contains_key(export) {
                            catch_error_and_exit(&format!(
                                "Exported variable '{}' not found in script output",
                                export
                            ));
                        }
                    }
                    Some(exported_vars)
                }
                Err(_) => {
                    catch_error_and_exit(&format!(
                        "External scripts must return valid JSON: {}",
                        stdout
                    ));
                }
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// RETURNING * capture helpers
// ---------------------------------------------------------------------------

/// Return `true` if the rendered query string contains a `RETURNING` clause.
/// Case-insensitive match; used to decide whether to capture a DML result.
pub fn has_returning_clause(query: &str) -> bool {
    query.to_uppercase().contains("RETURNING")
}

/// Remove a trailing `RETURNING ...` clause from a DML query.
///
/// Matches case-insensitively on the last `RETURNING` keyword occurrence and
/// truncates from there, preserving any trailing `;`.
pub fn strip_returning_clause(query: &str) -> String {
    let upper = query.to_uppercase();
    if let Some(idx) = upper.rfind("RETURNING") {
        let trailing_semi = query.trim_end().ends_with(';');
        let mut out = query[..idx].trim_end().to_string();
        if trailing_semi {
            out.push(';');
        }
        out
    } else {
        query.to_string()
    }
}

/// Execute a DML command (INSERT / UPDATE / DELETE), optionally capturing
/// the `RETURNING *` result as the first row.
///
/// Returns `(command_message, Option<first_row>)`.  When the DML includes
/// `RETURNING *` and the provider returns rows, the first row is captured.
/// If no rows are returned (provider returned no body), `None` is returned –
/// this is **not** an error.
pub fn run_stackql_dml_returning(
    command: &str,
    client: &mut PgwireLite,
    ignore_errors: bool,
    retries: u32,
    retry_delay: u32,
) -> (String, Option<HashMap<String, String>>) {
    let mut attempt = 0u32;

    while attempt <= retries {
        match execute_query(command, client) {
            Ok(result) => match result {
                QueryResult::Data {
                    columns,
                    rows,
                    notices,
                } => {
                    // Check for errors in notices before accepting the result.
                    let mut error_noticed = false;
                    for notice in &notices {
                        if error_detected_in_notice(notice) && !ignore_errors {
                            if attempt < retries {
                                debug!(
                                    "DML notice on attempt {}/{}, retrying in {} seconds: {}",
                                    attempt + 1,
                                    retries + 1,
                                    retry_delay,
                                    notice
                                );
                                thread::sleep(Duration::from_secs(retry_delay as u64));
                                attempt += 1;
                                error_noticed = true;
                                break;
                            } else {
                                catch_error_and_exit(&format!(
                                    "Error during stackql DML execution:\n\n{}\n\nlast rendered query:\n\n{}\n",
                                    notice, command
                                ));
                            }
                        }
                    }
                    if error_noticed {
                        continue;
                    }

                    // Capture RETURNING * first row (if any).
                    let first_row = if !rows.is_empty() {
                        let col_names: Vec<String> =
                            columns.iter().map(|c| c.name.clone()).collect();
                        let row = &rows[0];
                        let mut map = HashMap::new();
                        for (i, col_name) in col_names.iter().enumerate() {
                            let value = row.values.get(i).cloned().unwrap_or_default();
                            map.insert(col_name.clone(), value);
                        }
                        Some(map)
                    } else {
                        None
                    };

                    let msg = notices.join("\n");
                    return (msg, first_row);
                }
                QueryResult::Command(msg) => {
                    return (msg, None);
                }
                QueryResult::Empty => {
                    return (String::new(), None);
                }
            },
            Err(e) => {
                // Check for fatal errors (network, auth) before retrying
                if let Some(pattern) = check_fatal_error(&e) {
                    catch_error_and_exit(&format!(
                        "Fatal error (matched '{}'):\n\n{}\n",
                        pattern, e
                    ));
                }
                if !ignore_errors {
                    if attempt < retries {
                        debug!(
                            "DML error on attempt {}/{}, retrying in {} seconds: {}",
                            attempt + 1,
                            retries + 1,
                            retry_delay,
                            e
                        );
                        thread::sleep(Duration::from_secs(retry_delay as u64));
                        attempt += 1;
                        continue;
                    }
                    catch_error_and_exit(&format!(
                        "Exception during stackql DML execution:\n\n{}\n",
                        e
                    ));
                } else {
                    debug!("DML failed (ignored): {}", e);
                    return (String::new(), None);
                }
            }
        }
    }

    (String::new(), None)
}

/// Flatten a single RETURNING * row into dotted context keys and insert them
/// into `context`.
///
/// For each column `col` in `row`:
/// - `callback.{col}` is set (shorthand for the current resource's own `.iql`
///   templates).
/// - `{resource_name}.callback.{col}` is set (fully-qualified key accessible
///   by downstream resources).
///
/// If a column value is a JSON object it is recursively expanded:
/// `"ProgressEvent" = {"OperationStatus":"SUCCESS","RequestToken":"abc"}`
/// produces:
/// ```text
/// callback.ProgressEvent.OperationStatus = SUCCESS
/// callback.ProgressEvent.RequestToken    = abc
/// ```
pub fn flatten_returning_row(
    row: &HashMap<String, String>,
    resource_name: &str,
    context: &mut HashMap<String, String>,
) {
    for (col, val) in row {
        let short_prefix = format!("callback.{}", col);
        let full_prefix = format!("{}.callback.{}", resource_name, col);
        flatten_value_into_context(&short_prefix, &full_prefix, val, context);
    }
}

/// Recursively expand a string value (possibly JSON) into dotted context keys.
fn flatten_value_into_context(
    short_prefix: &str,
    full_prefix: &str,
    value: &str,
    context: &mut HashMap<String, String>,
) {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(value) {
        if json.is_object() {
            flatten_json_into_context(short_prefix, full_prefix, &json, context);
            return;
        }
    }
    context.insert(short_prefix.to_string(), value.to_string());
    context.insert(full_prefix.to_string(), value.to_string());
}

fn flatten_json_into_context(
    short_prefix: &str,
    full_prefix: &str,
    value: &serde_json::Value,
    context: &mut HashMap<String, String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let new_short = format!("{}.{}", short_prefix, k);
                let new_full = format!("{}.{}", full_prefix, k);
                flatten_json_into_context(&new_short, &new_full, v, context);
            }
        }
        serde_json::Value::String(s) => {
            context.insert(short_prefix.to_string(), s.clone());
            context.insert(full_prefix.to_string(), s.clone());
        }
        other => {
            let s = other.to_string();
            context.insert(short_prefix.to_string(), s.clone());
            context.insert(full_prefix.to_string(), s);
        }
    }
}

/// Check whether a short-circuit condition is met using already-captured
/// callback data.
///
/// `field` is a dot-path into the captured result (e.g.
/// `"ProgressEvent.OperationStatus"`), looked up as `callback.{field}` in
/// `context`.  Returns `true` if the value equals `expected_value`.
/// Returns `false` (no short-circuit) if the field is absent.
pub fn check_short_circuit(
    context: &HashMap<String, String>,
    field: &str,
    expected_value: &str,
) -> bool {
    let lookup_key = format!("callback.{}", field);
    match context.get(&lookup_key) {
        Some(val) => {
            let result = val == expected_value;
            if result {
                info!(
                    "short-circuit condition met: {} = {} (skipping callback poll)",
                    lookup_key, expected_value
                );
            }
            result
        }
        None => {
            debug!(
                "short-circuit field '{}' not found in context, proceeding with callback poll",
                lookup_key
            );
            false
        }
    }
}

/// Poll a callback query until the `success` (or `count`) column returns a
/// truthy value, or `retries` are exhausted.
///
/// Returns `true` on success, `false` when retries are exhausted (the caller
/// is responsible for treating exhaustion as an error).
pub fn run_callback_poll(
    resource_name: &str,
    query: &str,
    retries: u32,
    retry_delay: u32,
    client: &mut PgwireLite,
) -> bool {
    let mut attempt = 0u32;

    while attempt <= retries {
        debug!(
            "Callback poll for [{}] attempt {}:\n\n{}\n",
            resource_name,
            attempt + 1,
            query
        );

        let result = run_stackql_query(query, client, true, 0, 0);

        if !result.is_empty() {
            let row = &result[0];

            // Check `success` column (primary).
            if let Some(success_val) = row.get("success") {
                if success_val == "1" || success_val.to_lowercase() == "true" {
                    info!(
                        "[{}] callback poll succeeded on attempt {}",
                        resource_name,
                        attempt + 1
                    );
                    return true;
                }
            }

            // Check `count` column (alternative).
            if let Some(count_val) = row.get("count") {
                if count_val == "1" {
                    info!(
                        "[{}] callback poll succeeded (count=1) on attempt {}",
                        resource_name,
                        attempt + 1
                    );
                    return true;
                }
            }
        }

        if attempt < retries {
            info!(
                "[{}] callback poll attempt {}/{}: retrying in {} seconds...",
                resource_name,
                attempt + 1,
                retries + 1,
                retry_delay
            );
            thread::sleep(Duration::from_secs(retry_delay as u64));
        }
        attempt += 1;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // export_vars
    // ------------------------------------------------------------------

    #[test]
    fn test_export_vars_sets_global_and_scoped_key() {
        let mut ctx: HashMap<String, String> = HashMap::new();
        let mut data: HashMap<String, String> = HashMap::new();
        data.insert("role_name".to_string(), "my-role".to_string());

        export_vars(&mut ctx, "aws_cross_account_role", &data, &[]);

        // Global key
        assert_eq!(ctx.get("role_name").map(|s| s.as_str()), Some("my-role"));
        // Resource-scoped key
        assert_eq!(
            ctx.get("aws_cross_account_role.role_name")
                .map(|s| s.as_str()),
            Some("my-role"),
        );
    }

    #[test]
    fn test_export_vars_global_key_is_overridable() {
        let mut ctx: HashMap<String, String> = HashMap::new();

        // First resource exports role_name
        let mut data1 = HashMap::new();
        data1.insert("role_name".to_string(), "first-role".to_string());
        export_vars(&mut ctx, "resource_a", &data1, &[]);

        // Second resource exports role_name with a different value
        let mut data2 = HashMap::new();
        data2.insert("role_name".to_string(), "second-role".to_string());
        export_vars(&mut ctx, "resource_b", &data2, &[]);

        // Global key reflects the most recent export
        assert_eq!(
            ctx.get("role_name").map(|s| s.as_str()),
            Some("second-role")
        );
    }

    #[test]
    fn test_export_vars_scoped_key_is_immutable() {
        let mut ctx: HashMap<String, String> = HashMap::new();

        // First resource exports role_name
        let mut data1 = HashMap::new();
        data1.insert("role_name".to_string(), "original-role".to_string());
        export_vars(&mut ctx, "resource_a", &data1, &[]);

        // Simulate an accidental re-export of the same resource (e.g. called
        // twice): the scoped key must not be overwritten.
        let mut data2 = HashMap::new();
        data2.insert("role_name".to_string(), "should-not-overwrite".to_string());
        export_vars(&mut ctx, "resource_a", &data2, &[]);

        // Scoped key is unchanged
        assert_eq!(
            ctx.get("resource_a.role_name").map(|s| s.as_str()),
            Some("original-role"),
        );
        // Global key reflects the latest call (expected)
        assert_eq!(
            ctx.get("role_name").map(|s| s.as_str()),
            Some("should-not-overwrite"),
        );
    }

    #[test]
    fn test_export_vars_protected_values_are_stored_normally() {
        // Protection only affects log-masking, not what is stored
        let mut ctx: HashMap<String, String> = HashMap::new();
        let mut data = HashMap::new();
        data.insert("secret_key".to_string(), "super-secret".to_string());

        export_vars(&mut ctx, "vault", &data, &["secret_key".to_string()]);

        assert_eq!(
            ctx.get("secret_key").map(|s| s.as_str()),
            Some("super-secret")
        );
        assert_eq!(
            ctx.get("vault.secret_key").map(|s| s.as_str()),
            Some("super-secret"),
        );
    }

    #[test]
    fn test_export_vars_protected_values_registered_for_redaction() {
        // Protected export values must be masked anywhere they later surface
        // in log output (e.g. interpolated into a downstream query)
        let mut ctx: HashMap<String, String> = HashMap::new();
        let mut data = HashMap::new();
        data.insert(
            "generated_password".to_string(),
            "Utils-Exported-S3cret-1".to_string(),
        );

        export_vars(
            &mut ctx,
            "vault",
            &data,
            &["generated_password".to_string()],
        );

        let redacted = crate::core::secrets::redact("SELECT 'Utils-Exported-S3cret-1' AS password");
        assert!(
            !redacted.contains("Utils-Exported-S3cret-1"),
            "protected export value leaked: {}",
            redacted
        );
    }

    // ------------------------------------------------------------------
    // has_returning_clause
    // ------------------------------------------------------------------

    #[test]
    fn test_has_returning_clause_positive() {
        assert!(has_returning_clause(
            "INSERT INTO awscc.s3.buckets(BucketName, region) SELECT 'my-bucket', 'us-east-1' RETURNING *"
        ));
    }

    #[test]
    fn test_has_returning_clause_case_insensitive() {
        assert!(has_returning_clause("DELETE FROM t WHERE id=1 returning *"));
    }

    #[test]
    fn test_has_returning_clause_negative() {
        assert!(!has_returning_clause("INSERT INTO t(col) SELECT 'val'"));
    }

    // ------------------------------------------------------------------
    // flatten_returning_row
    // ------------------------------------------------------------------

    #[test]
    fn test_flatten_returning_row_simple_string_values() {
        let mut row = HashMap::new();
        row.insert("RequestToken".to_string(), "tok-123".to_string());
        row.insert("OperationStatus".to_string(), "SUCCESS".to_string());

        let mut ctx: HashMap<String, String> = HashMap::new();
        flatten_returning_row(&row, "my_resource", &mut ctx);

        assert_eq!(
            ctx.get("callback.RequestToken").map(|s| s.as_str()),
            Some("tok-123")
        );
        assert_eq!(
            ctx.get("my_resource.callback.RequestToken")
                .map(|s| s.as_str()),
            Some("tok-123")
        );
        assert_eq!(
            ctx.get("callback.OperationStatus").map(|s| s.as_str()),
            Some("SUCCESS")
        );
        assert_eq!(
            ctx.get("my_resource.callback.OperationStatus")
                .map(|s| s.as_str()),
            Some("SUCCESS")
        );
    }

    #[test]
    fn test_flatten_returning_row_nested_json() {
        // Provider returns ProgressEvent as a JSON object string.
        let mut row = HashMap::new();
        row.insert(
            "ProgressEvent".to_string(),
            r#"{"OperationStatus":"SUCCESS","RequestToken":"abc"}"#.to_string(),
        );

        let mut ctx: HashMap<String, String> = HashMap::new();
        flatten_returning_row(&row, "aws_s3_bucket", &mut ctx);

        assert_eq!(
            ctx.get("callback.ProgressEvent.OperationStatus")
                .map(|s| s.as_str()),
            Some("SUCCESS")
        );
        assert_eq!(
            ctx.get("callback.ProgressEvent.RequestToken")
                .map(|s| s.as_str()),
            Some("abc")
        );
        assert_eq!(
            ctx.get("aws_s3_bucket.callback.ProgressEvent.OperationStatus")
                .map(|s| s.as_str()),
            Some("SUCCESS")
        );
        assert_eq!(
            ctx.get("aws_s3_bucket.callback.ProgressEvent.RequestToken")
                .map(|s| s.as_str()),
            Some("abc")
        );
    }

    #[test]
    fn test_flatten_returning_row_empty_row_is_noop() {
        let row: HashMap<String, String> = HashMap::new();
        let mut ctx: HashMap<String, String> = HashMap::new();
        flatten_returning_row(&row, "res", &mut ctx);
        assert!(ctx.is_empty());
    }

    // ------------------------------------------------------------------
    // check_short_circuit
    // ------------------------------------------------------------------

    #[test]
    fn test_check_short_circuit_matches() {
        let mut ctx: HashMap<String, String> = HashMap::new();
        ctx.insert(
            "callback.ProgressEvent.OperationStatus".to_string(),
            "SUCCESS".to_string(),
        );
        assert!(check_short_circuit(
            &ctx,
            "ProgressEvent.OperationStatus",
            "SUCCESS"
        ));
    }

    #[test]
    fn test_check_short_circuit_no_match() {
        let mut ctx: HashMap<String, String> = HashMap::new();
        ctx.insert(
            "callback.ProgressEvent.OperationStatus".to_string(),
            "IN_PROGRESS".to_string(),
        );
        assert!(!check_short_circuit(
            &ctx,
            "ProgressEvent.OperationStatus",
            "SUCCESS"
        ));
    }

    #[test]
    fn test_check_short_circuit_missing_field() {
        let ctx: HashMap<String, String> = HashMap::new();
        // Field not present in context → no short-circuit.
        assert!(!check_short_circuit(
            &ctx,
            "ProgressEvent.OperationStatus",
            "SUCCESS"
        ));
    }

    #[test]
    fn test_references_unknown_export_detects_placeholder() {
        let rendered = "SELECT userName FROM databricks_workspace.iam.current_user                         WHERE deployment_name = '<unknown>'";
        assert!(references_unknown_export(rendered));
        assert!(!references_unknown_export(
            "SELECT userName FROM databricks_workspace.iam.current_user              WHERE deployment_name = 'dbc-1234'"
        ));
    }

    #[test]
    fn test_unknown_exports_for_handles_plain_and_mapped_exports() {
        let expected = vec![
            serde_yaml::Value::String("workspace_id".to_string()),
            serde_yaml::from_str::<serde_yaml::Value>("arn: role_arn").unwrap(),
        ];
        let fallback = unknown_exports_for(&expected);
        assert_eq!(fallback.len(), 2);
        assert_eq!(
            fallback.get("workspace_id").map(String::as_str),
            Some(UNKNOWN_EXPORT_PLACEHOLDER)
        );
        // Mapped exports use the target (value) name, not the source column.
        assert_eq!(
            fallback.get("role_arn").map(String::as_str),
            Some(UNKNOWN_EXPORT_PLACEHOLDER)
        );
        assert!(!fallback.contains_key("arn"));
    }
}
