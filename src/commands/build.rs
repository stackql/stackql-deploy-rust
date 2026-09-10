// commands/build.rs

//! # Build Command
//!
//! Implements the `build` (deploy) command. Creates or updates infrastructure
//! resources defined in a stack manifest.
//! This is the Rust equivalent of Python's `cmd/build.py` `StackQLProvisioner`.

use std::collections::HashMap;
use std::time::Instant;

use clap::{Arg, ArgMatches, Command};
use log::{debug, info, warn};

use crate::commands::base::CommandRunner;
use crate::commands::common_args::{
    dry_run, env_file, env_var, log_level, on_failure, show_queries, stack_dir, stack_env,
    FailureAction,
};
use crate::core::config::get_resource_type;
use crate::core::utils::{catch_error_and_exit, export_vars, DRY_RUN_EXPORT_PLACEHOLDER};
use crate::utils::connection::create_client;
use crate::utils::display::{print_unicode_box, BorderColor};
use crate::utils::server::{check_and_start_server, stop_local_server};

/// Defines the `build` command for the CLI application.
pub fn command() -> Command {
    Command::new("build")
        .about("Create or update resources")
        .arg(stack_dir())
        .arg(stack_env())
        .arg(log_level())
        .arg(env_file())
        .arg(env_var())
        .arg(dry_run())
        .arg(show_queries())
        .arg(on_failure())
        .arg(
            Arg::new("output-file")
                .long("output-file")
                .help("File path to write deployment outputs as JSON")
                .num_args(1),
        )
}

/// Executes the `build` command.
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
    let output_file = matches.get_one::<String>("output-file");

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
            "Deploying stack: [{}] to environment: [{}]",
            stack_name_display, stack_env_val
        ),
        BorderColor::Yellow,
    );

    run_build(
        &mut runner,
        is_dry_run,
        is_show_queries,
        &format!("{:?}", on_failure_val),
        output_file.map(|s| s.as_str()),
    );

    if is_dry_run {
        print_unicode_box("dry-run build complete", BorderColor::Green);
    } else {
        print_unicode_box("build complete", BorderColor::Green);
    }

    stop_local_server();
}

/// Render the statecheck query template with the given context.
/// Uses try_render_query so that unresolved variables (e.g. this.* fields
/// not yet captured) return None instead of a hard error.
macro_rules! render_statecheck {
    ($runner:expr, $resource_queries:expr, $resource:expr, $ctx:expr) => {
        $resource_queries.get("statecheck").and_then(|q| {
            match $runner.try_render_query(&$resource.name, "statecheck", &q.template, $ctx) {
                Some(rendered) => Some((rendered, q.options.clone())),
                None => {
                    debug!(
                        "statecheck query for [{}] deferred (unresolved variables)",
                        $resource.name
                    );
                    None
                }
            }
        })
    };
}

/// Render the exports query template with the given context.
macro_rules! render_exports {
    ($runner:expr, $resource_queries:expr, $resource:expr, $ctx:expr) => {
        $resource_queries.get("exports").and_then(|q| {
            match $runner.try_render_query(&$resource.name, "exports", &q.template, $ctx) {
                Some(rendered) => Some(rendered),
                None => {
                    debug!(
                        "exports query for [{}] deferred (unresolved variables)",
                        $resource.name
                    );
                    None
                }
            }
        })
    };
}

/// Main build workflow matching Python's StackQLProvisioner.run().
pub fn run_build(
    runner: &mut CommandRunner,
    dry_run: bool,
    show_queries: bool,
    _on_failure: &str,
    output_file: Option<&str>,
) {
    let start_time = Instant::now();

    info!(
        "deploying [{}] in [{}] environment {}",
        runner.stack_name,
        runner.stack_env,
        if dry_run { "(dry run)" } else { "" }
    );

    let resources = runner.manifest.resources.clone();

    for resource in &resources {
        print_unicode_box(
            &format!("Processing resource: [{}]", resource.name),
            BorderColor::Blue,
        );

        let res_type = get_resource_type(resource).to_string();
        info!(
            "processing resource [{}], type: {}",
            resource.name, res_type
        );

        let full_context = runner.get_full_context(resource);

        // Evaluate condition
        if !runner.evaluate_condition(resource, &full_context) {
            continue;
        }

        // Handle script type
        if res_type == "script" {
            runner.process_script_resource(resource, dry_run, &full_context);
            continue;
        }

        // Get resource queries (templates only, not yet rendered)
        let (resource_queries, inline_query) = if let Some(sql_val) = resource
            .sql
            .as_ref()
            .filter(|_| res_type == "command" || res_type == "query")
        {
            let iq = runner.render_inline_template(&resource.name, sql_val, &full_context);
            (HashMap::new(), Some(iq))
        } else {
            (runner.get_queries(resource, &full_context), None)
        };

        // Detect anchor presence and extract retry options (no rendering yet).
        // All query rendering is deferred to the point of use (JIT) because
        // exists may capture this.* fields needed by downstream queries.
        let has_createorupdate = resource_queries.contains_key("createorupdate");
        let create_retries;
        let create_retry_delay;
        let update_retries;
        let update_retry_delay;

        if res_type == "resource" || res_type == "multi" {
            if has_createorupdate {
                let cou = resource_queries.get("createorupdate").unwrap();
                create_retries = cou.options.retries;
                create_retry_delay = cou.options.retry_delay;
                update_retries = cou.options.retries;
                update_retry_delay = cou.options.retry_delay;
            } else {
                if let Some(cq) = resource_queries.get("create") {
                    create_retries = cq.options.retries;
                    create_retry_delay = cq.options.retry_delay;
                } else {
                    catch_error_and_exit(
                        "iql file must include either 'create' or 'createorupdate' anchor.",
                    );
                }
                if let Some(uq) = resource_queries.get("update") {
                    update_retries = uq.options.retries;
                    update_retry_delay = uq.options.retry_delay;
                } else {
                    update_retries = 1;
                    update_retry_delay = 0;
                }
            }
        } else {
            create_retries = 1;
            create_retry_delay = 0;
            update_retries = 1;
            update_retry_delay = 0;
        }

        // Render exists eagerly (it never depends on this.* fields)
        let exists_query = resource_queries.get("exists").map(|q| {
            let rendered =
                runner.render_query(&resource.name, "exists", &q.template, &full_context);
            (rendered, q.options.clone())
        });

        let mut full_context = full_context;
        let exports_opts = resource_queries.get("exports");
        let exports_retries = exports_opts.map_or(1, |q| q.options.retries);
        let exports_retry_delay = exports_opts.map_or(0, |q| q.options.retry_delay);

        // All other queries (create, update, statecheck, exports) are rendered
        // JIT at the point of use, after exists has had a chance to capture
        // this.* fields into full_context.
        let mut exports_query_str: Option<String> = None;

        // Handle query type: render exports eagerly (query types don't
        // have exists/statecheck so there's no this.* deferral needed).
        if res_type == "query" {
            if let Some(ref iq) = inline_query {
                exports_query_str = Some(iq.clone());
            } else {
                exports_query_str =
                    render_exports!(runner, resource_queries, resource, &full_context);
                if exports_query_str.is_none() {
                    catch_error_and_exit(
                        "Inline sql must be supplied or an iql file must be present with an 'exports' anchor for query type resources.",
                    );
                }
            }
        }

        let mut exports_result_from_proxy: Option<Vec<HashMap<String, String>>> = None;

        if res_type == "resource" || res_type == "multi" {
            let ignore_errors = res_type == "multi";
            let mut resource_exists = false;
            let mut is_correct_state = false;

            /// Inject fields captured by the exists query into the context as
            /// `this.<field>` variables (scoped to the resource name), so that
            /// statecheck / exports / delete templates can reference the
            /// discovered identifiers.
            fn apply_exists_fields(
                fields: Option<HashMap<String, String>>,
                resource_name: &str,
                full_context: &mut HashMap<String, String>,
            ) {
                if let Some(ref f) = fields {
                    for (k, v) in f {
                        full_context.insert(format!("{}.{}", resource_name, k), v.clone());
                    }
                }
            }

            // State checking logic
            if has_createorupdate {
                // Skip all existence and state checks for createorupdate
            } else if resource_queries.contains_key("statecheck") {
                // Flow 1: Traditional flow when statecheck exists
                if let Some(ref eq) = exists_query {
                    // Pre-create: fast fail (1 attempt, no delay)
                    let (exists, fields) = runner.check_if_resource_exists(
                        resource,
                        &eq.0,
                        1,
                        0,
                        dry_run,
                        show_queries,
                        false,
                    );
                    resource_exists = exists;

                    // If the exists query captured fields, inject them and
                    // re-render downstream queries.
                    if fields.is_some() {
                        apply_exists_fields(fields, &resource.name, &mut full_context);
                    }
                } else {
                    // Use statecheck as exists check (render with current ctx).
                    // If the statecheck template has unresolved variables (e.g.
                    // this.* fields not yet captured), the resource cannot exist
                    // yet - treat as not-found.
                    if let Some(sq) =
                        render_statecheck!(runner, resource_queries, resource, &full_context)
                    {
                        let sq_opts = resource_queries.get("statecheck").unwrap();
                        is_correct_state = runner.check_if_resource_is_correct_state(
                            resource,
                            &sq.0,
                            sq_opts.options.retries,
                            sq_opts.options.retry_delay,
                            dry_run,
                            show_queries,
                        );
                        resource_exists = is_correct_state;
                    } else {
                        info!(
                            "[{}] statecheck has unresolved variables, treating as not found",
                            resource.name
                        );
                        resource_exists = false;
                    }
                }

                // Pre-deployment state check for existing resources
                if resource_exists && !is_correct_state {
                    if resource.skip_validation.unwrap_or(false) {
                        info!(
                            "skipping validation for [{}] as skip_validation is set to true.",
                            resource.name
                        );
                        is_correct_state = true;
                    } else {
                        // Re-render statecheck with (possibly enriched) context
                        if let Some(sq) =
                            render_statecheck!(runner, resource_queries, resource, &full_context)
                        {
                            let sq_opts = resource_queries.get("statecheck").unwrap();
                            is_correct_state = runner.check_if_resource_is_correct_state(
                                resource,
                                &sq.0,
                                sq_opts.options.retries,
                                sq_opts.options.retry_delay,
                                dry_run,
                                show_queries,
                            );
                        } else {
                            warn!(
                                "[{}] statecheck has unresolved variables during pre-deploy validation",
                                resource.name
                            );
                        }
                    }
                }

                // Re-render exports with enriched context (only if exists
                // captured fields; otherwise defer until post-create).
                if resource_exists {
                    exports_query_str =
                        render_exports!(runner, resource_queries, resource, &full_context);
                }
            } else if exports_query_str.is_some() {
                // Flow 2: Optimized flow using exports as proxy
                info!(
                    "trying exports query first (fast-fail) for optimal validation for [{}]",
                    resource.name
                );
                let (state, proxy_result) = runner.check_state_using_exports_proxy(
                    resource,
                    exports_query_str.as_ref().unwrap(),
                    1,
                    0,
                    dry_run,
                    show_queries,
                );
                is_correct_state = state;
                resource_exists = is_correct_state;

                if is_correct_state {
                    info!(
                        "[{}] validated successfully with fast exports query",
                        resource.name
                    );
                    exports_result_from_proxy = proxy_result;
                } else {
                    info!(
                        "fast exports validation failed, falling back to exists check for [{}]",
                        resource.name
                    );
                    exports_result_from_proxy = None;

                    if let Some(ref eq) = exists_query {
                        // Pre-create: fast fail (1 attempt, no delay)
                        let (exists, fields) = runner.check_if_resource_exists(
                            resource,
                            &eq.0,
                            1,
                            0,
                            dry_run,
                            show_queries,
                            false,
                        );
                        resource_exists = exists;

                        if fields.is_some() {
                            apply_exists_fields(fields, &resource.name, &mut full_context);
                        }
                        // Always try to render exports after fallback exists
                        // (needed for count-based exists where exports doesn't
                        // depend on this.* fields).
                        exports_query_str =
                            render_exports!(runner, resource_queries, resource, &full_context);
                    } else {
                        resource_exists = false;
                    }
                }
            } else if let Some(ref eq) = exists_query {
                // Flow 3: exists query only (no statecheck rendered yet)
                // Pre-create: fast fail (1 attempt, no delay)
                let (exists, fields) = runner.check_if_resource_exists(
                    resource,
                    &eq.0,
                    1,
                    0,
                    dry_run,
                    show_queries,
                    false,
                );
                resource_exists = exists;
                let has_fields = fields.is_some();

                if has_fields {
                    apply_exists_fields(fields, &resource.name, &mut full_context);
                }
                // Always try to render exports after exists
                exports_query_str =
                    render_exports!(runner, resource_queries, resource, &full_context);

                // Determine correctness based on what's available:
                if exists {
                    if let Some(ref eq_str) = exports_query_str {
                        // Use exports as statecheck proxy
                        info!(
                            "using exports query as statecheck proxy for [{}]",
                            resource.name
                        );
                        let (state, proxy) = runner.check_state_using_exports_proxy(
                            resource,
                            eq_str,
                            exports_retries,
                            exports_retry_delay,
                            dry_run,
                            show_queries,
                        );
                        is_correct_state = state;
                        if proxy.is_some() {
                            exports_result_from_proxy = proxy;
                        }
                    } else {
                        // No statecheck and no exports: exists IS the statecheck
                        is_correct_state = true;
                    }
                }
            } else {
                catch_error_and_exit(
                    "iql file must include either 'exists', 'statecheck', or 'exports' anchor.",
                );
            }

            // Create or update
            let mut is_created_or_updated = false;

            if !resource_exists {
                // JIT render create/createorupdate query.
                // In dry-run mode, use try_render_query so that unresolved
                // variables (from exports not yet available) produce a
                // deferral instead of a hard error.
                let create_query = if has_createorupdate {
                    let cou = resource_queries.get("createorupdate").unwrap();
                    if dry_run {
                        runner.try_render_query(
                            &resource.name,
                            "createorupdate",
                            &cou.template,
                            &full_context,
                        )
                    } else {
                        Some(runner.render_query(
                            &resource.name,
                            "createorupdate",
                            &cou.template,
                            &full_context,
                        ))
                    }
                } else {
                    let cq = resource_queries.get("create").unwrap();
                    if dry_run {
                        runner.try_render_query(
                            &resource.name,
                            "create",
                            &cq.template,
                            &full_context,
                        )
                    } else {
                        Some(runner.render_query(
                            &resource.name,
                            "create",
                            &cq.template,
                            &full_context,
                        ))
                    }
                };

                if create_query.is_none() {
                    info!(
                        "dry run create for [{}]: query has unresolved variables \
                         (upstream exports not yet available), skipping render",
                        resource.name
                    );
                }

                let (created, returning_row) = if let Some(ref cq) = create_query {
                    runner.create_resource(
                        resource,
                        cq,
                        create_retries,
                        create_retry_delay,
                        dry_run,
                        show_queries,
                        ignore_errors,
                    )
                } else {
                    (false, None)
                };
                is_created_or_updated = created;

                // Capture RETURNING * result.
                if let Some(ref row) = returning_row {
                    debug!("RETURNING payload for [{}]: {:?}", resource.name, row);
                    runner.store_callback_data(&resource.name, row);

                    // Apply return_vals mappings from manifest.
                    let mappings = resource.get_return_val_mappings("create");
                    if !mappings.is_empty() {
                        let mut fields = HashMap::new();
                        for (src, tgt) in &mappings {
                            if let Some(val) = row.get(src.as_str()) {
                                if !val.is_empty() && val != "null" {
                                    info!(
                                        "RETURNING [{}] for [{}] captured as [this.{}] = [{}]",
                                        src, resource.name, tgt, val
                                    );
                                    fields.insert(tgt.clone(), val.clone());
                                } else {
                                    catch_error_and_exit(&format!(
                                        "return_vals for [{}]: field [{}] in RETURNING result \
                                         is null or empty.",
                                        resource.name, src
                                    ));
                                }
                            } else {
                                catch_error_and_exit(&format!(
                                    "return_vals for [{}]: expected field [{}] not found in \
                                     RETURNING result. Ensure the create query includes \
                                     'RETURNING *' or 'RETURNING {}'.",
                                    resource.name, src, src
                                ));
                            }
                        }
                        apply_exists_fields(Some(fields), &resource.name, &mut full_context);
                        // Re-render exports/statecheck with the captured values
                        exports_query_str =
                            render_exports!(runner, resource_queries, resource, &full_context);
                    }
                } else if !resource.get_return_val_mappings("create").is_empty() {
                    warn!(
                        "return_vals specified for [{}] create but no RETURNING data received. \
                         Will fall back to post-create exists query.",
                        resource.name
                    );
                }

                // Run callback:create block if present.
                if is_created_or_updated {
                    let cb_anchor = if resource_queries.contains_key("callback:create") {
                        Some("callback:create")
                    } else if resource_queries.contains_key("callback") {
                        Some("callback")
                    } else {
                        None
                    };
                    if let Some(anchor) = cb_anchor {
                        // Pre-extract before the mutable borrow of runner.
                        if let Some(q) = resource_queries.get(anchor) {
                            let cb_template = q.template.clone();
                            let cb_retries = q.options.retries;
                            let cb_delay = q.options.retry_delay;
                            let cb_sc_field = q.options.short_circuit_field.clone();
                            let cb_sc_value = q.options.short_circuit_value.clone();
                            let cb_ctx = runner.get_full_context(resource);
                            let rendered_cb =
                                runner.render_query(&resource.name, anchor, &cb_template, &cb_ctx);
                            runner.run_callback(
                                resource,
                                &rendered_cb,
                                cb_retries,
                                cb_delay,
                                cb_sc_field.as_deref(),
                                cb_sc_value.as_deref(),
                                "create",
                                dry_run,
                                show_queries,
                            );
                        }
                    }
                }
            }

            if resource_exists && !is_correct_state {
                // JIT render update/createorupdate query.
                // In dry-run mode, use try_render_query for tolerance.
                let update_query: Option<String> = if has_createorupdate {
                    let cou = resource_queries.get("createorupdate").unwrap();
                    if dry_run {
                        runner.try_render_query(
                            &resource.name,
                            "createorupdate",
                            &cou.template,
                            &full_context,
                        )
                    } else {
                        Some(runner.render_query(
                            &resource.name,
                            "createorupdate",
                            &cou.template,
                            &full_context,
                        ))
                    }
                } else {
                    resource_queries.get("update").and_then(|uq| {
                        if dry_run {
                            runner.try_render_query(
                                &resource.name,
                                "update",
                                &uq.template,
                                &full_context,
                            )
                        } else {
                            Some(runner.render_query(
                                &resource.name,
                                "update",
                                &uq.template,
                                &full_context,
                            ))
                        }
                    })
                };

                if update_query.is_none() && dry_run {
                    info!(
                        "dry run update for [{}]: query has unresolved variables \
                         (upstream exports not yet available), skipping render",
                        resource.name
                    );
                }

                let (updated, returning_row) = runner.update_resource(
                    resource,
                    update_query.as_deref(),
                    update_retries,
                    update_retry_delay,
                    dry_run,
                    show_queries,
                    ignore_errors,
                );
                is_created_or_updated = updated;

                // Capture RETURNING * result.
                if let Some(ref row) = returning_row {
                    debug!(
                        "RETURNING payload for [{}] (update): {:?}",
                        resource.name, row
                    );
                    runner.store_callback_data(&resource.name, row);

                    // Apply return_vals mappings from manifest.
                    let mappings = resource.get_return_val_mappings("update");
                    if !mappings.is_empty() {
                        let mut fields = HashMap::new();
                        for (src, tgt) in &mappings {
                            if let Some(val) = row.get(src.as_str()) {
                                if !val.is_empty() && val != "null" {
                                    info!(
                                        "RETURNING [{}] for [{}] captured as [this.{}] = [{}]",
                                        src, resource.name, tgt, val
                                    );
                                    fields.insert(tgt.clone(), val.clone());
                                } else {
                                    catch_error_and_exit(&format!(
                                        "return_vals for [{}]: field [{}] in RETURNING result \
                                         is null or empty.",
                                        resource.name, src
                                    ));
                                }
                            } else {
                                catch_error_and_exit(&format!(
                                    "return_vals for [{}]: expected field [{}] not found in \
                                     RETURNING result. Ensure the update query includes \
                                     'RETURNING *' or 'RETURNING {}'.",
                                    resource.name, src, src
                                ));
                            }
                        }
                        apply_exists_fields(Some(fields), &resource.name, &mut full_context);
                        exports_query_str =
                            render_exports!(runner, resource_queries, resource, &full_context);
                    }
                } else if !resource.get_return_val_mappings("update").is_empty()
                    && is_created_or_updated
                {
                    warn!(
                        "return_vals specified for [{}] update but no RETURNING data received. \
                         Will fall back to post-update exists query.",
                        resource.name
                    );
                }

                // Run callback:update block if present.
                if is_created_or_updated {
                    let cb_anchor = if resource_queries.contains_key("callback:update") {
                        Some("callback:update")
                    } else if resource_queries.contains_key("callback") {
                        Some("callback")
                    } else {
                        None
                    };
                    if let Some(anchor) = cb_anchor {
                        if let Some(q) = resource_queries.get(anchor) {
                            let cb_template = q.template.clone();
                            let cb_retries = q.options.retries;
                            let cb_delay = q.options.retry_delay;
                            let cb_sc_field = q.options.short_circuit_field.clone();
                            let cb_sc_value = q.options.short_circuit_value.clone();
                            let cb_ctx = runner.get_full_context(resource);
                            let rendered_cb =
                                runner.render_query(&resource.name, anchor, &cb_template, &cb_ctx);
                            runner.run_callback(
                                resource,
                                &rendered_cb,
                                cb_retries,
                                cb_delay,
                                cb_sc_field.as_deref(),
                                cb_sc_value.as_deref(),
                                "update",
                                dry_run,
                                show_queries,
                            );
                        }
                    }
                }
            }

            // Post-deploy state check
            if is_created_or_updated {
                let op = if !resource_exists { "create" } else { "update" };

                // After create/update, re-run the exists query to capture
                // this.* fields (e.g. identifier) needed by statecheck and
                // exports queries.  This always runs even when return_vals
                // captured some fields, because the exists query discovers
                // the resource identifier and waits for the resource to
                // become available (async/eventual consistency).
                if let Some(ref eq) = exists_query {
                    // Use statecheck retry settings for the post-create
                    // exists check when available (async providers need
                    // time for the resource to become discoverable).
                    let (post_retries, post_delay) =
                        if let Some(sc_opts) = resource_queries.get("statecheck") {
                            (sc_opts.options.retries, sc_opts.options.retry_delay)
                        } else {
                            let eq_opts = resource_queries.get("exists").unwrap();
                            (eq_opts.options.retries, eq_opts.options.retry_delay)
                        };

                    let (post_exists, fields) = runner.check_if_resource_exists(
                        resource,
                        &eq.0,
                        post_retries,
                        post_delay,
                        dry_run,
                        show_queries,
                        false,
                    );

                    // If exists retries are exhausted and resource still
                    // not found, run troubleshoot and exit immediately -
                    // don't attempt statecheck/exports.
                    if !post_exists && !dry_run {
                        runner.run_troubleshoot(
                            resource,
                            &resource_queries,
                            op,
                            &full_context,
                            show_queries,
                        );
                        catch_error_and_exit(&format!(
                            "[{}] not found after {} post-deploy check, {} operation may have failed.",
                            resource.name, op, op
                        ));
                    }

                    apply_exists_fields(fields, &resource.name, &mut full_context);

                    // Always try to render exports after post-create exists
                    exports_query_str =
                        render_exports!(runner, resource_queries, resource, &full_context);

                    // If exists confirms the resource is present and there is
                    // no statecheck or exports query, the exists query IS
                    // the statecheck: a successful re-run confirms the
                    // resource was created/updated successfully.
                    if post_exists
                        && !resource_queries.contains_key("statecheck")
                        && exports_query_str.is_none()
                    {
                        is_correct_state = true;
                    }
                }

                // If exports wasn't rendered yet (e.g. no exists query to
                // trigger it), try now — the context may already contain all
                // the variables the exports template needs.
                if exports_query_str.is_none() {
                    exports_query_str =
                        render_exports!(runner, resource_queries, resource, &full_context);
                }

                debug!(
                    "post-deploy for [{}]: is_correct_state={}, has_statecheck={}, exports_query_str={}",
                    resource.name,
                    is_correct_state,
                    resource_queries.contains_key("statecheck"),
                    if exports_query_str.is_some() { "Some" } else { "None" }
                );

                if let Some(sq) =
                    render_statecheck!(runner, resource_queries, resource, &full_context)
                {
                    let sq_opts = resource_queries.get("statecheck").unwrap();
                    is_correct_state = runner.check_if_resource_is_correct_state(
                        resource,
                        &sq.0,
                        sq_opts.options.retries,
                        sq_opts.options.retry_delay,
                        dry_run,
                        show_queries,
                    );
                } else if resource_queries.contains_key("statecheck") {
                    // Statecheck anchor exists but could not be rendered (unresolved
                    // this.* variables). Fall through to exports-as-proxy if available,
                    // otherwise treat as correct (the resource was just created and
                    // the post-create exists query did not return identifier fields).
                    if let Some(ref eq_str) = exports_query_str {
                        info!(
                            "statecheck deferred for [{}], using exports query as post-deploy statecheck",
                            resource.name
                        );
                        let post_retries = exports_retries;
                        let post_delay = exports_retry_delay;

                        let (state, proxy) = runner.check_state_using_exports_proxy(
                            resource,
                            eq_str,
                            post_retries,
                            post_delay,
                            dry_run,
                            show_queries,
                        );
                        is_correct_state = state;
                        if proxy.is_some() {
                            exports_result_from_proxy = proxy;
                        }
                    } else {
                        info!(
                            "statecheck deferred for [{}] and no exports available, \
                             accepting post-deploy state based on successful create/update",
                            resource.name
                        );
                        is_correct_state = true;
                    }
                } else if has_createorupdate {
                    info!(
                        "createorupdate for [{}] is authoritative, skipping exports-as-statecheck proxy",
                        resource.name
                    );
                    is_correct_state = true;
                } else if let Some(ref eq_str) = exports_query_str {
                    info!(
                        "using exports query as post-deploy statecheck for [{}]",
                        resource.name
                    );
                    let post_retries = exports_retries;
                    let post_delay = exports_retry_delay;

                    let (state, proxy) = runner.check_state_using_exports_proxy(
                        resource,
                        eq_str,
                        post_retries,
                        post_delay,
                        dry_run,
                        show_queries,
                    );
                    is_correct_state = state;
                    if proxy.is_some() {
                        exports_result_from_proxy = proxy;
                    }
                }
            }

            if !is_correct_state && !dry_run {
                let op = if !resource_exists { "create" } else { "update" };
                runner.run_troubleshoot(
                    resource,
                    &resource_queries,
                    op,
                    &full_context,
                    show_queries,
                );
                catch_error_and_exit(&format!(
                    "deployment failed for {} after post-deploy checks.",
                    resource.name
                ));
            }
        }

        // Handle command type
        if res_type == "command" {
            let (command_query, command_retries, command_retry_delay) = if let Some(ref iq) =
                inline_query
            {
                (iq.clone(), 1u32, 0u32)
            } else if let Some(cq) = resource_queries.get("command") {
                let rendered =
                    runner.render_query(&resource.name, "command", &cq.template, &full_context);
                (rendered, cq.options.retries, cq.options.retry_delay)
            } else {
                catch_error_and_exit(
                        "'sql' should be defined in the resource or the 'command' anchor needs to be supplied in the corresponding iql file for command type resources.",
                    );
            };

            runner.run_command(
                &command_query,
                command_retries,
                command_retry_delay,
                dry_run,
                show_queries,
            );
        }

        // Process exports with optimization
        if let Some(ref eq_str) = exports_query_str {
            if let Some(ref proxy_result) = exports_result_from_proxy {
                if res_type == "resource" || res_type == "multi" {
                    info!(
                        "reusing exports result from proxy for [{}]...",
                        resource.name
                    );
                    if !resource.exports.is_empty() {
                        runner.process_exports_from_result(resource, proxy_result);
                    }
                }
            } else {
                runner.process_exports(
                    resource,
                    &full_context,
                    eq_str,
                    exports_retries,
                    exports_retry_delay,
                    dry_run,
                    show_queries,
                    false,
                );
            }
        }

        // If the resource has an exports anchor but we never resolved the query,
        // that's a fatal error - variables that can't be resolved at this point
        // indicate a missing dependency or misconfigured template.
        if exports_query_str.is_none()
            && resource_queries.contains_key("exports")
            && !resource.exports.is_empty()
        {
            if dry_run {
                // In dry-run mode, exports may not render because this.*
                // fields are unavailable (no actual API calls).  Inject
                // placeholder values so downstream resources can still
                // render their templates.
                let mut placeholder_data = HashMap::new();
                for item in &resource.exports {
                    if let Some(map) = item.as_mapping() {
                        for (_, val) in map {
                            if let Some(v) = val.as_str() {
                                placeholder_data
                                    .insert(v.to_string(), DRY_RUN_EXPORT_PLACEHOLDER.to_string());
                            }
                        }
                    } else if let Some(s) = item.as_str() {
                        placeholder_data
                            .insert(s.to_string(), DRY_RUN_EXPORT_PLACEHOLDER.to_string());
                    }
                }
                info!(
                    "dry run: injecting placeholder exports for [{}]: {:?}",
                    resource.name,
                    placeholder_data.keys().collect::<Vec<_>>()
                );
                export_vars(
                    &mut runner.global_context,
                    &resource.name,
                    &placeholder_data,
                    &resource.protected,
                );
            } else {
                runner.run_troubleshoot(
                    resource,
                    &resource_queries,
                    "create",
                    &full_context,
                    show_queries,
                );
                catch_error_and_exit(&format!(
                    "exports query for [{}] could not be rendered - unresolved template variables. \
                     Check that all referenced variables are defined in the manifest or exported by prior resources.",
                    resource.name
                ));
            }
        }

        if !dry_run {
            if res_type == "resource" {
                info!("successfully deployed {}", resource.name);
            } else if res_type == "query" {
                info!(
                    "successfully exported variables for query in {}",
                    resource.name
                );
            }
        }
    }

    let elapsed = start_time.elapsed();
    let elapsed_str = format!("{:.2?}", elapsed);
    info!("deployment completed in {}", elapsed_str);

    runner.process_stack_exports(dry_run, output_file, &elapsed_str);
}
