// commands/teardown.rs

//! # Teardown Command
//!
//! Implements the `teardown` command. Destroys provisioned resources in reverse order.
//! This is the Rust equivalent of Python's `cmd/teardown.py` `StackQLDeProvisioner`.

use std::time::Instant;

use clap::{ArgMatches, Command};
use log::{debug, info, warn};

use crate::commands::base::CommandRunner;
use crate::commands::common_args::{
    dry_run, env_file, env_var, log_level, on_failure, show_queries, stack_dir, stack_env,
    FailureAction,
};
use crate::core::config::get_resource_type;
use crate::core::utils::{
    has_returning_clause, references_unknown_export, strip_returning_clause,
    UNKNOWN_EXPORT_PLACEHOLDER,
};
use crate::resource::manifest::Resource;
use crate::utils::connection::create_client;
use crate::utils::display::{print_unicode_box, BorderColor};
use crate::utils::server::{check_and_start_server, stop_local_server};

/// Configures the `teardown` command for the CLI application.
pub fn command() -> Command {
    Command::new("teardown")
        .about("Teardown a provisioned stack")
        .arg(stack_dir())
        .arg(stack_env())
        .arg(log_level())
        .arg(env_file())
        .arg(env_var())
        .arg(dry_run())
        .arg(show_queries())
        .arg(on_failure())
}

/// Executes the `teardown` command.
pub fn execute(matches: &ArgMatches) {
    let stack_dir_val = matches.get_one::<String>("stack_dir").unwrap();
    let stack_env_val = matches.get_one::<String>("stack_env").unwrap();
    let env_file_val = matches.get_one::<String>("env-file").unwrap();
    let env_vars: Vec<String> = matches
        .get_many::<String>("env")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let is_dry_run = matches.get_flag("dry-run");
    let is_show_queries = matches.get_flag("show-queries");
    let on_failure_val = matches.get_one::<FailureAction>("on-failure").unwrap();

    check_and_start_server();
    let client = create_client();
    let mut runner = CommandRunner::new(
        client,
        stack_dir_val,
        stack_env_val,
        env_file_val,
        &env_vars,
    );

    let stack_name_display = if runner.stack_name.is_empty() {
        runner.stack_dir.clone()
    } else {
        runner.stack_name.clone()
    };

    print_unicode_box(
        &format!(
            "Tearing down stack: [{}] in environment: [{}]",
            stack_name_display, stack_env_val
        ),
        BorderColor::Yellow,
    );

    run_teardown(&mut runner, is_dry_run, is_show_queries, *on_failure_val);

    if is_dry_run {
        print_unicode_box("dry-run teardown complete", BorderColor::Green);
    } else {
        print_unicode_box("teardown complete", BorderColor::Green);
    }

    stop_local_server();
}

/// Render a query template for teardown.
///
/// Returns `None` when the query cannot or must not be executed:
///
/// - template variables are unresolved (an upstream export was never
///   populated), or
/// - the rendered SQL contains the [`UNKNOWN_EXPORT_PLACEHOLDER`] because an
///   upstream export could not be collected.
///
/// In both cases `consequence` is logged so the operator can see why the
/// resource was skipped.
fn render_for_teardown(
    runner: &CommandRunner,
    resource: &Resource,
    anchor: &str,
    template: &str,
    full_context: &std::collections::HashMap<String, String>,
    consequence: &str,
) -> Option<String> {
    match runner.try_render_query(&resource.name, anchor, template, full_context) {
        Some(rendered) if references_unknown_export(&rendered) => {
            info!(
                "[{}] {} query references an export that could not be collected ({}), {}",
                resource.name, anchor, UNKNOWN_EXPORT_PLACEHOLDER, consequence
            );
            None
        }
        Some(rendered) => Some(rendered),
        None => {
            info!(
                "[{}] {} query has unresolved variables, {}",
                resource.name, anchor, consequence
            );
            None
        }
    }
}

/// Collect exports for all resources before teardown.
///
/// Exports are collected in manifest order so that a downstream resource's
/// `exists`/`delete` queries can reference upstream values. Every resource
/// ends this phase with each of its declared exports either populated or set
/// to [`UNKNOWN_EXPORT_PLACEHOLDER`]; queries that would interpolate the
/// placeholder are skipped by [`render_for_teardown`].
fn collect_exports(runner: &mut CommandRunner, show_queries: bool, dry_run: bool) {
    info!(
        "collecting exports for [{}] in [{}] environment",
        runner.stack_name, runner.stack_env
    );

    let resources = runner.manifest.resources.clone();

    for resource in &resources {
        let res_type = get_resource_type(resource).to_string();
        info!("getting exports for resource [{}]", resource.name);

        // Commands have no exports, and scripts are not executed during
        // teardown; mark script exports unknown so downstream references
        // are skipped rather than left unrenderable.
        if res_type == "command" {
            continue;
        }
        if res_type == "script" {
            runner.set_exports_unknown(resource);
            continue;
        }

        // A query resource opted out of teardown: do not run it, and mark
        // its exports unknown so anything depending on them is skipped.
        if res_type == "query" && resource.skip_on_delete {
            info!(
                "[{}] skip_on_delete is set, query not executed during teardown",
                resource.name
            );
            runner.set_exports_unknown(resource);
            continue;
        }

        let mut full_context = runner.get_full_context(resource);

        let (exports_query, exports_retries, exports_retry_delay) = if let Some(sql_val) =
            resource.sql.as_ref().filter(|_| res_type == "query")
        {
            match runner.try_render_inline_template(&resource.name, sql_val, &full_context) {
                Some(rendered) if references_unknown_export(&rendered) => {
                    info!(
                        "[{}] inline query references an export that could not be collected ({}), skipping exports collection",
                        resource.name, UNKNOWN_EXPORT_PLACEHOLDER
                    );
                    (None, 1u32, 0u32)
                }
                Some(rendered) => (Some(rendered), 1u32, 0u32),
                None => {
                    info!(
                        "[{}] inline query has unresolved variables, skipping exports collection",
                        resource.name
                    );
                    (None, 1u32, 0u32)
                }
            }
        } else {
            let queries = runner.get_queries(resource, &full_context);
            // Run exists query first to capture this.* fields needed by
            // exports (e.g. this.identifier).
            if let Some(eq) = queries.get("exists") {
                if let Some(rendered) = render_for_teardown(
                    runner,
                    resource,
                    "exists",
                    &eq.template,
                    &full_context,
                    "assuming resource does not exist",
                ) {
                    let (_exists, fields) = runner.check_if_resource_exists(
                        resource,
                        &rendered,
                        eq.options.retries,
                        eq.options.retry_delay,
                        dry_run,
                        show_queries,
                        false,
                    );
                    if let Some(ref f) = fields {
                        for (k, v) in f {
                            full_context.insert(format!("{}.{}", resource.name, k), v.clone());
                        }
                    }
                }
            }
            if let Some(eq) = queries.get("exports") {
                // During teardown use minimal retries - the resource may
                // already be partially deleted.
                (
                    render_for_teardown(
                        runner,
                        resource,
                        "exports",
                        &eq.template,
                        &full_context,
                        "skipping exports collection",
                    ),
                    1u32,
                    0u32,
                )
            } else {
                (None, 1u32, 0u32)
            }
        };

        match exports_query {
            Some(ref eq_str) => runner.process_exports(
                resource,
                &full_context,
                eq_str,
                exports_retries,
                exports_retry_delay,
                dry_run,
                show_queries,
                true, // ignore_missing_exports
            ),
            // The exports could not be collected (no query, unresolved, or
            // <unknown> upstream): mark them unknown so downstream queries
            // referencing them are skipped consistently.
            None => runner.set_exports_unknown(resource),
        }
    }
}

/// Main teardown workflow matching Python's StackQLDeProvisioner.run().
///
/// `on_failure` controls what happens when a `delete` statement fails at the
/// provider:
///
/// - `Error` (default): the first failed delete aborts the run.
/// - `Ignore`: the failure is logged, the resource is reported as not
///   confirmed deleted, and the run continues with the next resource. Fatal
///   errors (network, auth, planner) still abort.
/// - `Rollback`: not meaningful for teardown; treated as `Error`.
///
/// A delete that cannot be confirmed within its retry budget never aborts the
/// run on its own; such resources are listed in a summary at the end.
pub fn run_teardown(
    runner: &mut CommandRunner,
    dry_run: bool,
    show_queries: bool,
    on_failure: FailureAction,
) {
    let start_time = Instant::now();

    info!(
        "tearing down [{}] in [{}] environment {}",
        runner.stack_name,
        runner.stack_env,
        if dry_run { "(dry run)" } else { "" }
    );

    let ignore_delete_errors = match on_failure {
        FailureAction::Ignore => {
            info!("on-failure=ignore: a failed delete is logged and the teardown continues with the next resource");
            true
        }
        FailureAction::Rollback => {
            warn!(
                "on-failure=rollback is not supported for teardown, treating as on-failure=error"
            );
            false
        }
        FailureAction::Error => false,
    };

    // Resources whose delete was not confirmed (failed, or still present
    // after the retry budget); reported once at the end of the run.
    let mut unconfirmed: Vec<String> = Vec::new();

    // Collect all exports first
    collect_exports(runner, show_queries, dry_run);

    // Process resources in reverse order
    let resources: Vec<_> = runner
        .manifest
        .resources
        .clone()
        .into_iter()
        .rev()
        .collect();

    for resource in &resources {
        print_unicode_box(
            &format!("Processing resource: [{}]", resource.name),
            BorderColor::Red,
        );

        let res_type = get_resource_type(resource).to_string();

        if res_type != "resource" && res_type != "multi" {
            debug!("skipping resource [{}] (type: {})", resource.name, res_type);
            continue;
        }

        if resource.skip_on_delete {
            info!(
                "[{}] skip_on_delete is set, resource retained (delete not executed)",
                resource.name
            );
            continue;
        }

        info!(
            "de-provisioning resource [{}], type: {}",
            resource.name, res_type
        );

        let full_context = runner.get_full_context(resource);

        // Evaluate condition
        if !runner.evaluate_condition(resource, &full_context) {
            continue;
        }

        // Add reverse export map variables to full context
        let mut full_context = full_context;
        for export in &resource.exports {
            if let Some(map) = export.as_mapping() {
                for (key_val, lookup_val) in map {
                    let key = key_val.as_str().unwrap_or("");
                    let lookup_key = lookup_val.as_str().unwrap_or("");
                    if let Some(value) = full_context.get(lookup_key).cloned() {
                        full_context.insert(key.to_string(), value);
                    }
                }
            }
        }

        // Get resource queries (templates only)
        let resource_queries = runner.get_queries(resource, &full_context);

        // Get exists query (fallback to statecheck) - render JIT
        let (exists_query_str, exists_retries, exists_retry_delay) =
            if let Some(eq) = resource_queries.get("exists") {
                match render_for_teardown(
                    runner,
                    resource,
                    "exists",
                    &eq.template,
                    &full_context,
                    "assuming resource does not exist, skipping...",
                ) {
                    Some(rendered) => (rendered, eq.options.retries, eq.options.retry_delay),
                    None => continue,
                }
            } else if let Some(sq) = resource_queries.get("statecheck") {
                info!(
                    "exists query not defined for [{}], trying statecheck query as exists query.",
                    resource.name
                );
                match render_for_teardown(
                    runner,
                    resource,
                    "statecheck",
                    &sq.template,
                    &full_context,
                    "skipping...",
                ) {
                    Some(rendered) => (rendered, sq.options.retries, sq.options.retry_delay),
                    None => continue,
                }
            } else {
                info!(
                    "No exists or statecheck query for [{}], skipping...",
                    resource.name
                );
                continue;
            };

        // Check if delete query template exists (don't render yet — may need
        // this.* fields from the exists check).
        let has_delete_query = resource_queries.contains_key("delete");
        if !has_delete_query {
            info!(
                "delete query not defined for [{}], skipping...",
                resource.name
            );
            continue;
        }

        // Pre-delete check
        let ignore_errors = res_type == "multi" || ignore_delete_errors;
        let resource_exists = if res_type == "multi" {
            info!("pre-delete check not supported for multi resources, skipping...");
            true
        } else {
            let (exists, fields) = runner.check_if_resource_exists(
                resource,
                &exists_query_str,
                exists_retries,
                exists_retry_delay,
                dry_run,
                show_queries,
                false,
            );
            // If the exists query captured fields, inject them as this.* so
            // the delete query can reference them.
            if let Some(ref f) = fields {
                for (k, v) in f {
                    full_context.insert(format!("{}.{}", resource.name, k), v.clone());
                }
            }
            // A dry run executes no queries, so the exists check above only
            // logged its SQL and reported "not found". Assume the resource
            // exists so the delete statement is rendered and logged too.
            if dry_run {
                info!(
                    "dry run: assuming [{}] exists so the delete can be shown",
                    resource.name
                );
                true
            } else {
                exists
            }
        };

        // Delete
        if resource_exists {
            // Render the delete query now (after exists fields are available).
            let dq = resource_queries.get("delete").unwrap();
            let rendered_delete = match render_for_teardown(
                runner,
                resource,
                "delete",
                &dq.template,
                &full_context,
                "cannot delete, skipping...",
            ) {
                Some(rendered) => rendered,
                None => continue,
            };
            let delete_retries = dq.options.retries;
            let delete_retry_delay = dq.options.retry_delay;

            // Only keep a RETURNING clause when return_vals.delete is configured
            // for this resource. Otherwise strip it — teardown has no use for
            // return values, and some providers reject RETURNING * on DELETE.
            let delete_return_mappings = resource.get_return_val_mappings("delete");
            let delete_query = if delete_return_mappings.is_empty() {
                if has_returning_clause(&rendered_delete) {
                    debug!(
                        "[{}] stripping RETURNING clause from delete query (no return_vals.delete configured)",
                        resource.name
                    );
                    strip_returning_clause(&rendered_delete)
                } else {
                    rendered_delete
                }
            } else if !has_returning_clause(&rendered_delete) {
                warn!(
                    "return_vals.delete specified for [{}] but delete query has no RETURNING clause; capture will be skipped",
                    resource.name
                );
                rendered_delete
            } else {
                rendered_delete
            };

            let (returning_row, delete_confirmed) = runner.delete_and_confirm(
                resource,
                &delete_query,
                &exists_query_str,
                delete_retries,
                delete_retry_delay,
                dry_run,
                show_queries,
                ignore_errors,
            );

            // Capture RETURNING * result.
            if let Some(ref row) = returning_row {
                debug!("RETURNING payload for [{}]: {:?}", resource.name, row);
                runner.store_callback_data(&resource.name, row);

                // Apply return_vals.delete mappings from manifest.
                if !delete_return_mappings.is_empty() {
                    for (src, tgt) in &delete_return_mappings {
                        if let Some(val) = row.get(src.as_str()) {
                            if !val.is_empty() && val != "null" {
                                info!(
                                    "RETURNING [{}] for [{}] captured as [this.{}] = [{}]",
                                    src, resource.name, tgt, val
                                );
                                full_context
                                    .insert(format!("{}.{}", resource.name, tgt), val.clone());
                            } else {
                                warn!(
                                    "return_vals.delete for [{}]: field [{}] in RETURNING result is null or empty",
                                    resource.name, src
                                );
                            }
                        } else {
                            warn!(
                                "return_vals.delete for [{}]: expected field [{}] not found in RETURNING result",
                                resource.name, src
                            );
                        }
                    }
                }
            } else if !delete_return_mappings.is_empty() {
                warn!(
                    "return_vals.delete specified for [{}] but no RETURNING data received",
                    resource.name
                );
            }

            // Run callback:delete block if present. A callback polls the
            // handle returned by RETURNING *, so there is nothing to poll
            // when no row came back: a dry run, a delete without RETURNING,
            // or a delete that failed and was ignored.
            let cb_anchor = if resource_queries.contains_key("callback:delete") {
                Some("callback:delete")
            } else if resource_queries.contains_key("callback") {
                Some("callback")
            } else {
                None
            };
            if let Some(anchor) = cb_anchor {
                if returning_row.is_none() {
                    info!(
                        "[{}] {} not run: the delete returned no RETURNING data{}",
                        resource.name,
                        anchor,
                        if dry_run { " (dry run)" } else { "" }
                    );
                } else if let Some(q) = resource_queries.get(anchor) {
                    let cb_template = q.template.clone();
                    let cb_retries = q.options.retries;
                    let cb_delay = q.options.retry_delay;
                    let cb_sc_field = q.options.short_circuit_field.clone();
                    let cb_sc_value = q.options.short_circuit_value.clone();
                    let cb_ctx = runner.get_full_context(resource);
                    match runner.try_render_query(&resource.name, anchor, &cb_template, &cb_ctx) {
                        Some(rendered_cb) => runner.run_callback(
                            resource,
                            &rendered_cb,
                            cb_retries,
                            cb_delay,
                            cb_sc_field.as_deref(),
                            cb_sc_value.as_deref(),
                            "delete",
                            dry_run,
                            show_queries,
                        ),
                        None => warn!(
                            "[{}] {} has unresolved variables and was not run",
                            resource.name, anchor
                        ),
                    }
                }
            }

            if delete_confirmed {
                info!("successfully deleted {}", resource.name);
            } else {
                runner.run_troubleshoot(
                    resource,
                    &resource_queries,
                    "delete",
                    &full_context,
                    show_queries,
                );
                info!("[{}] delete could not be confirmed", resource.name);
                unconfirmed.push(resource.name.clone());
            }
        } else {
            info!(
                "resource [{}] does not exist, skipping delete",
                resource.name
            );
            continue;
        }
    }

    let elapsed = start_time.elapsed();
    if unconfirmed.is_empty() {
        info!("teardown completed in {:.2?}", elapsed);
    } else {
        warn!(
            "teardown completed in {:.2?} with {} resource(s) whose delete could not be confirmed: {}",
            elapsed,
            unconfirmed.len(),
            unconfirmed
                .iter()
                .map(|n| format!("[{}]", n))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}
