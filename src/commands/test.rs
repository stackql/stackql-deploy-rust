// commands/test.rs

//! # Test Command
//!
//! Implements the `test` command. Validates that deployed resources are in
//! the correct desired state.
//! This is the Rust equivalent of Python's `cmd/test.py` `StackQLTestRunner`.

use std::collections::HashMap;
use std::time::Instant;

use clap::{Arg, ArgMatches, Command};
use log::info;

use crate::commands::base::CommandRunner;
use crate::commands::common_args::{
    dry_run, env_file, env_var, log_level, on_failure, show_queries, stack_dir, stack_env,
    FailureAction,
};
use crate::core::config::get_resource_type;
use crate::core::utils::catch_error_and_exit;
use crate::utils::connection::create_client;
use crate::utils::display::{print_unicode_box, BorderColor};
use crate::utils::server::{check_and_start_server, stop_local_server};

/// Configures the `test` command for the CLI application.
pub fn command() -> Command {
    Command::new("test")
        .about("Run test queries for the stack")
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

/// Executes the `test` command.
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
            "Testing stack: [{}] in environment: [{}]",
            stack_name_display, stack_env_val
        ),
        BorderColor::Yellow,
    );

    run_test(
        &mut runner,
        is_dry_run,
        is_show_queries,
        &format!("{:?}", on_failure_val),
        output_file.map(|s| s.as_str()),
    );

    if is_dry_run {
        print_unicode_box("dry-run tests complete", BorderColor::Green);
    } else {
        print_unicode_box("tests complete", BorderColor::Green);
    }

    stop_local_server();
}

/// Main test workflow matching Python's StackQLTestRunner.run().
pub fn run_test(
    runner: &mut CommandRunner,
    dry_run: bool,
    show_queries: bool,
    _on_failure: &str,
    output_file: Option<&str>,
) {
    let start_time = Instant::now();

    info!(
        "testing [{}] in [{}] environment {}",
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

        let mut full_context = runner.get_full_context(resource);

        // Evaluate condition (same semantics as build and teardown)
        if !runner.evaluate_condition(resource, &full_context) {
            continue;
        }

        if res_type == "query" {
            info!("exporting variables for [{}]", resource.name);
        } else if res_type == "resource" || res_type == "multi" {
            info!("testing resource [{}], type: {}", resource.name, res_type);
        } else if res_type == "command" {
            // Commands are mutations with no state to test.
            continue;
        } else if res_type == "script" {
            // Scripts are the only source of their exports, which later
            // resources may reference; run them exactly as build does.
            runner.process_script_resource(resource, dry_run, &full_context);
            continue;
        } else {
            catch_error_and_exit(&format!("unknown resource type: {}", res_type));
        }

        // Get test queries (templates only, not yet rendered)
        let (test_queries, inline_query) =
            if let Some(sql_val) = resource.sql.as_ref().filter(|_| res_type == "query") {
                let iq = runner.render_inline_template(&resource.name, sql_val, &full_context);
                (HashMap::new(), Some(iq))
            } else {
                (runner.get_queries(resource, &full_context), None)
            };

        // Run the exists query first if present to capture this.* fields
        // (e.g. identifier) before rendering statecheck/exports.
        let mut exists_is_statecheck = false;
        if let Some(eq) = test_queries.get("exists") {
            let rendered =
                runner.render_query(&resource.name, "exists", &eq.template, &full_context);
            let (exists, fields) = runner.check_if_resource_exists(
                resource,
                &rendered,
                eq.options.retries,
                eq.options.retry_delay,
                dry_run,
                show_queries,
                false,
            );
            let has_fields = fields.is_some();
            if let Some(ref f) = fields {
                for (k, v) in f {
                    full_context.insert(format!("{}.{}", resource.name, k), v.clone());
                }
            }
            // If exists exports a variable and there is no statecheck or
            // exports query, the exists query IS the statecheck.
            if exists
                && has_fields
                && !test_queries.contains_key("statecheck")
                && !test_queries.contains_key("exports")
            {
                exists_is_statecheck = true;
            }
        }

        // Render statecheck JIT (after exists fields are available)
        let statecheck_rendered = test_queries.get("statecheck").map(|q| {
            let rendered =
                runner.render_query(&resource.name, "statecheck", &q.template, &full_context);
            (rendered, q.options.clone())
        });
        let statecheck_retries = test_queries
            .get("statecheck")
            .map_or(1, |q| q.options.retries);
        let statecheck_retry_delay = test_queries
            .get("statecheck")
            .map_or(0, |q| q.options.retry_delay);

        // Render exports JIT (after exists fields are available)
        let mut exports_query_str = test_queries
            .get("exports")
            .map(|q| runner.render_query(&resource.name, "exports", &q.template, &full_context));
        let exports_opts = test_queries.get("exports");
        let exports_retries = exports_opts.map_or(1, |q| q.options.retries);
        let exports_retry_delay = exports_opts.map_or(0, |q| q.options.retry_delay);

        if res_type == "query" && exports_query_str.is_none() {
            if let Some(ref iq) = inline_query {
                exports_query_str = Some(iq.clone());
            } else {
                catch_error_and_exit(
                    "Inline sql must be supplied or an iql file must be present with an 'exports' anchor for query type resources.",
                );
            }
        }

        // Statecheck with optimizations
        let mut exports_result_from_proxy: Option<Vec<HashMap<String, String>>> = None;

        if res_type == "resource" || res_type == "multi" {
            let is_correct_state;

            if resource.skip_validation.unwrap_or(false) {
                info!("Skipping statecheck for {}", resource.name);
                is_correct_state = true;
            } else if let Some(ref sq) = statecheck_rendered {
                is_correct_state = runner.check_if_resource_is_correct_state(
                    resource,
                    &sq.0,
                    sq.1.retries,
                    sq.1.retry_delay,
                    dry_run,
                    show_queries,
                );
            } else if let Some(ref eq_str) = exports_query_str {
                // OPTIMIZATION: Use exports as statecheck proxy
                info!(
                    "using exports query as proxy for statecheck test for [{}]",
                    resource.name
                );
                let (state, proxy) = runner.check_state_using_exports_proxy(
                    resource,
                    eq_str,
                    statecheck_retries,
                    statecheck_retry_delay,
                    dry_run,
                    show_queries,
                );
                is_correct_state = state;
                exports_result_from_proxy = proxy;
            } else if exists_is_statecheck {
                // Exists query exported a variable and there is no statecheck
                // or exports; the successful exists check confirms the state.
                info!(
                    "exists query with captured fields confirms state for [{}]",
                    resource.name
                );
                is_correct_state = true;
            } else {
                catch_error_and_exit(
                    "iql file must include either 'statecheck' or 'exports' anchor for validation.",
                );
            }

            if !is_correct_state && !dry_run {
                catch_error_and_exit(&format!("test failed for {}.", resource.name));
            }
        }

        // Exports with optimization
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

        if res_type == "resource" && !dry_run {
            info!("test passed for {}", resource.name);
        }
    }

    let elapsed = start_time.elapsed();
    let elapsed_str = format!("{:.2?}", elapsed);
    info!("test completed in {}", elapsed_str);

    runner.process_stack_exports(dry_run, output_file, &elapsed_str);
}
