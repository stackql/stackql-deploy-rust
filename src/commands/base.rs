// commands/base.rs

//! # Base Command Module
//!
//! Shared resource processing logic used by build, teardown, and test commands.
//! This is the Rust equivalent of the Python `cmd/base.py` `StackQLBase` class.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process;

use log::{debug, error, info, warn};

use crate::core::config::{get_full_context, render_globals, render_string_value};
use crate::core::env::load_env_vars;
use crate::core::templating::{self, ParsedQuery};
use crate::core::utils::{
    catch_error_and_exit, check_exports_as_statecheck_proxy, check_short_circuit, export_vars,
    flatten_returning_row, has_returning_clause, perform_retries, perform_retries_with_fields,
    pull_providers, run_callback_poll, run_ext_script, run_stackql_command,
    run_stackql_dml_returning, run_stackql_query, show_query, unknown_exports_for,
    DRY_RUN_EXPORT_PLACEHOLDER,
};
use crate::resource::manifest::{Manifest, Resource};
use crate::resource::validation::validate_manifest;
use crate::template::engine::TemplateEngine;
use crate::utils::display::{print_unicode_box, BorderColor};
use crate::utils::pgwire::PgwireLite;

/// Core state for all command operations, equivalent to Python's StackQLBase.
pub struct CommandRunner {
    pub client: PgwireLite,
    pub engine: TemplateEngine,
    pub manifest: Manifest,
    pub global_context: HashMap<String, String>,
    pub stack_dir: String,
    pub stack_env: String,
    pub stack_name: String,
    #[allow(dead_code)]
    pub env_vars: HashMap<String, String>,
    /// Per-resource idempotency tokens (UUID v4), stable for the lifetime of
    /// a single session (invocation).  Keyed by resource name.
    pub idempotency_tokens: HashMap<String, String>,
}

impl CommandRunner {
    /// Create a new CommandRunner, loading manifest, pulling providers, etc.
    pub fn new(
        mut client: PgwireLite,
        stack_dir: &str,
        stack_env: &str,
        env_file: &str,
        env_overrides: &[String],
    ) -> Self {
        let engine = TemplateEngine::new();

        // Load env vars
        let env_vars = load_env_vars(env_file, env_overrides);

        // Load manifest
        let manifest = Manifest::load_from_dir_or_exit(stack_dir);

        // Validate manifest rules
        if let Err(errors) = validate_manifest(&manifest) {
            for err in &errors {
                error!("{}", err);
            }
            catch_error_and_exit(&format!(
                "Manifest validation failed with {} error(s)",
                errors.len()
            ));
        }

        let stack_name = manifest.name.clone();

        // Render globals
        let global_context = render_globals(&engine, &env_vars, &manifest, stack_env, &stack_name);

        // Generate a stable UUID v4 idempotency token for each resource once,
        // at session start.  The same token is reused on every retry within
        // this invocation, allowing providers to distinguish retries from new
        // requests.
        let idempotency_tokens: HashMap<String, String> = manifest
            .resources
            .iter()
            .map(|r| (r.name.clone(), uuid::Uuid::new_v4().to_string()))
            .collect();

        // Pull providers
        pull_providers(&manifest.providers, &mut client);

        Self {
            client,
            engine,
            manifest,
            global_context,
            stack_dir: stack_dir.to_string(),
            stack_env: stack_env.to_string(),
            stack_name,
            env_vars,
            idempotency_tokens,
        }
    }

    /// Get the full context for a resource (global + resource properties).
    pub fn get_full_context(&self, resource: &Resource) -> HashMap<String, String> {
        let token = self
            .idempotency_tokens
            .get(&resource.name)
            .map(|s| s.as_str());
        get_full_context(
            &self.engine,
            &self.global_context,
            resource,
            &self.stack_env,
            token,
        )
    }

    /// Evaluate a resource's `if` condition. Returns true if the resource should be processed.
    pub fn evaluate_condition(
        &self,
        resource: &Resource,
        full_context: &HashMap<String, String>,
    ) -> bool {
        if let Some(ref condition) = resource.r#if {
            let rendered = render_string_value(&self.engine, condition, full_context);

            // Evaluate simple string equality/inequality conditions
            // Python uses eval(), we do simple pattern matching for safety
            match evaluate_simple_condition(&rendered) {
                Some(result) => {
                    if !result {
                        info!(
                            "Skipping resource [{}] due to condition: {}",
                            resource.name, condition
                        );
                    }
                    result
                }
                None => {
                    error!(
                        "Error evaluating condition for resource [{}]: {}",
                        resource.name, rendered
                    );
                    process::exit(1);
                }
            }
        } else {
            true // No condition, always process
        }
    }

    /// Get queries for a resource from its .iql file.
    pub fn get_queries(
        &self,
        resource: &Resource,
        full_context: &HashMap<String, String>,
    ) -> HashMap<String, ParsedQuery> {
        templating::get_queries(&self.engine, &self.stack_dir, resource, full_context)
    }

    /// Render inline SQL template.
    pub fn render_inline_template(
        &self,
        resource_name: &str,
        sql: &str,
        full_context: &HashMap<String, String>,
    ) -> String {
        templating::render_inline_template(&self.engine, resource_name, sql, full_context)
    }

    /// Tolerant variant of [`Self::render_inline_template`]: returns `None`
    /// instead of exiting when the inline SQL references variables that are
    /// not in the context. Used during teardown.
    pub fn try_render_inline_template(
        &self,
        resource_name: &str,
        sql: &str,
        full_context: &HashMap<String, String>,
    ) -> Option<String> {
        templating::try_render_inline_template(&self.engine, resource_name, sql, full_context)
    }

    /// Render a single query template JIT with the current context.
    pub fn render_query(
        &self,
        resource_name: &str,
        anchor: &str,
        template: &str,
        full_context: &HashMap<String, String>,
    ) -> String {
        templating::render_query(&self.engine, resource_name, anchor, template, full_context)
    }

    /// Try to render a query template, returning None if variables are missing.
    /// Used for deferred rendering where this.* fields may not yet be available.
    pub fn try_render_query(
        &self,
        resource_name: &str,
        anchor: &str,
        template: &str,
        full_context: &HashMap<String, String>,
    ) -> Option<String> {
        templating::try_render_query(&self.engine, resource_name, anchor, template, full_context)
    }

    /// Check if a resource exists using the exists query.
    #[allow(clippy::too_many_arguments)]
    /// Check if a resource exists by running the exists query.
    ///
    /// Returns `(bool, Option<HashMap<String, String>>)`:
    /// - The bool indicates whether the resource exists.
    /// - If the exists query returned fields OTHER than `count`, those fields
    ///   are captured and returned.  The caller should inject them into the
    ///   template context scoped to the resource (e.g. `this.identifier`) so
    ///   that subsequent queries (statecheck, exports, delete) can reference
    ///   the discovered identifier without a separate lookup.
    pub fn check_if_resource_exists(
        &mut self,
        resource: &Resource,
        exists_query: &str,
        retries: u32,
        retry_delay: u32,
        dry_run: bool,
        show_queries: bool,
        delete_test: bool,
    ) -> (bool, Option<HashMap<String, String>>) {
        let check_type = if delete_test { "post-delete" } else { "exists" };

        if dry_run {
            info!(
                "dry run {} check for [{}]:\n\n/* exists query */\n{}\n",
                check_type, resource.name, exists_query
            );
            return (false, None);
        }

        info!("running {} check for [{}]...", check_type, resource.name);
        show_query(show_queries, exists_query);

        let (exists, fields) = perform_retries_with_fields(
            &resource.name,
            exists_query,
            retries,
            retry_delay,
            &mut self.client,
            delete_test,
        );

        if delete_test {
            if exists {
                info!("[{}] confirmed deleted", resource.name);
            } else {
                info!("[{}] still exists after post-delete check", resource.name);
            }
        } else if exists {
            info!("[{}] exists", resource.name);
            // Log any captured fields from the exists query
            if let Some(ref f) = fields {
                for (k, v) in f {
                    info!(
                        "exists query for [{}] captured field [this.{}] ({{ {}.{} }}) = [{}]",
                        resource.name, k, resource.name, k, v
                    );
                }
            }
        } else {
            info!("[{}] does not exist", resource.name);
        }

        (exists, fields)
    }

    /// Check if a resource is in the correct state.
    pub fn check_if_resource_is_correct_state(
        &mut self,
        resource: &Resource,
        statecheck_query: &str,
        retries: u32,
        retry_delay: u32,
        dry_run: bool,
        show_queries: bool,
    ) -> bool {
        if dry_run {
            info!(
                "dry run state check for [{}]:\n\n/* state check query */\n{}\n",
                resource.name, statecheck_query
            );
            return true;
        }

        info!("running state check for [{}]...", resource.name);
        show_query(show_queries, statecheck_query);

        let is_correct = perform_retries(
            &resource.name,
            statecheck_query,
            retries,
            retry_delay,
            &mut self.client,
            false,
        );

        if is_correct {
            info!("[{}] is in the desired state", resource.name);
        } else {
            info!("[{}] is not in the desired state", resource.name);
        }

        is_correct
    }

    /// Use exports query as a proxy for state check.
    pub fn check_state_using_exports_proxy(
        &mut self,
        resource: &Resource,
        exports_query: &str,
        retries: u32,
        retry_delay: u32,
        dry_run: bool,
        show_queries: bool,
    ) -> (bool, Option<Vec<HashMap<String, String>>>) {
        if dry_run {
            info!(
                "dry run state check using exports proxy for [{}]:\n\n/* exports as statecheck proxy */\n{}\n",
                resource.name, exports_query
            );
            return (true, None);
        }

        info!(
            "running state check using exports proxy for [{}]...",
            resource.name
        );
        show_query(show_queries, exports_query);

        let result = run_stackql_query(exports_query, &mut self.client, true, retries, retry_delay);

        let is_correct = check_exports_as_statecheck_proxy(&result);

        if is_correct {
            info!(
                "[{}] exports proxy indicates resource is in the desired state",
                resource.name
            );
            (true, Some(result))
        } else {
            info!(
                "[{}] exports proxy indicates resource is not in the desired state",
                resource.name
            );
            (false, None)
        }
    }

    /// Create a resource.
    ///
    /// Returns `(created, returning_row)` where `returning_row` is `Some` when
    /// the create query included `RETURNING *` and the provider returned data.
    #[allow(clippy::too_many_arguments)]
    pub fn create_resource(
        &mut self,
        resource: &Resource,
        create_query: &str,
        retries: u32,
        retry_delay: u32,
        dry_run: bool,
        show_queries: bool,
        ignore_errors: bool,
    ) -> (bool, Option<HashMap<String, String>>) {
        if dry_run {
            if has_returning_clause(create_query) {
                info!(
                    "dry run create for [{}]:\n\n/* insert (create) query with RETURNING */\n{}\n\
                     [dry run: RETURNING * capture skipped]\n",
                    resource.name, create_query
                );
            } else {
                info!(
                    "dry run create for [{}]:\n\n/* insert (create) query */\n{}\n",
                    resource.name, create_query
                );
            }
            return (false, None);
        }

        info!("creating [{}]...", resource.name);
        show_query(show_queries, create_query);

        if has_returning_clause(create_query) {
            let (msg, returning_row) = run_stackql_dml_returning(
                create_query,
                &mut self.client,
                ignore_errors,
                retries,
                retry_delay,
            );
            if msg.is_empty() && returning_row.is_none() {
                debug!("Create response: no response");
            } else {
                debug!("Create response: {}", msg);
            }
            (true, returning_row)
        } else {
            let msg = run_stackql_command(
                create_query,
                &mut self.client,
                ignore_errors,
                retries,
                retry_delay,
            );
            if msg.is_empty() {
                debug!("Create response: no response");
            } else {
                debug!("Create response: {}", msg);
            }
            (true, None)
        }
    }

    /// Update a resource.
    ///
    /// Returns `(updated, returning_row)` where `returning_row` is `Some` when
    /// the update query included `RETURNING *` and the provider returned data.
    #[allow(clippy::too_many_arguments)]
    pub fn update_resource(
        &mut self,
        resource: &Resource,
        update_query: Option<&str>,
        retries: u32,
        retry_delay: u32,
        dry_run: bool,
        show_queries: bool,
        ignore_errors: bool,
    ) -> (bool, Option<HashMap<String, String>>) {
        match update_query {
            Some(query) => {
                if dry_run {
                    if has_returning_clause(query) {
                        info!(
                            "dry run update for [{}]:\n\n/* update query with RETURNING */\n{}\n\
                             [dry run: RETURNING * capture skipped]\n",
                            resource.name, query
                        );
                    } else {
                        info!(
                            "dry run update for [{}]:\n\n/* update query */\n{}\n",
                            resource.name, query
                        );
                    }
                    return (false, None);
                }

                info!("updating [{}]...", resource.name);
                show_query(show_queries, query);

                if has_returning_clause(query) {
                    let (msg, returning_row) = run_stackql_dml_returning(
                        query,
                        &mut self.client,
                        ignore_errors,
                        retries,
                        retry_delay,
                    );
                    if msg.is_empty() && returning_row.is_none() {
                        debug!("Update response: no response");
                    } else {
                        debug!("Update response: {}", msg);
                    }
                    (true, returning_row)
                } else {
                    let msg = run_stackql_command(
                        query,
                        &mut self.client,
                        ignore_errors,
                        retries,
                        retry_delay,
                    );
                    if msg.is_empty() {
                        debug!("Update response: no response");
                    } else {
                        debug!("Update response: {}", msg);
                    }
                    (true, None)
                }
            }
            None => {
                info!(
                    "Update query not configured for [{}], skipping update...",
                    resource.name
                );
                (false, None)
            }
        }
    }

    /// Delete a resource and confirm deletion with an interleaved
    /// delete-check-retry loop.
    ///
    /// When `delete_retries > 0` the loop is:
    ///   1. Execute DELETE
    ///   2. Run exists query — count==0 → done, count==1 → continue, else → error
    ///   3. Wait `delete_retry_delay` seconds
    ///   4. Run exists query again — count==0 → done, count==1 → re-delete
    ///      ... repeat up to `delete_retries` times
    ///
    /// When `delete_retries == 0`: single delete + single check, no retry.
    ///
    /// Returns the RETURNING * row (if any) from the first successful delete.
    #[allow(clippy::too_many_arguments)]
    pub fn delete_and_confirm(
        &mut self,
        resource: &Resource,
        delete_query: &str,
        exists_query: &str,
        delete_retries: u32,
        delete_retry_delay: u32,
        dry_run: bool,
        show_queries: bool,
        ignore_errors: bool,
    ) -> (Option<HashMap<String, String>>, bool) {
        // --- dry run path ---
        if dry_run {
            if has_returning_clause(delete_query) {
                info!(
                    "dry run delete for [{}]:\n\n{}\n[dry run: RETURNING * capture skipped]\n",
                    resource.name, delete_query
                );
            } else {
                info!(
                    "dry run delete for [{}]:\n\n{}\n",
                    resource.name, delete_query
                );
            }
            return (None, true);
        }

        let mut returning_row: Option<HashMap<String, String>> = None;

        // Helper closure: execute the DELETE statement once (no retries on the
        // DML itself — retries are handled by the outer loop).
        let execute_delete = |client: &mut crate::utils::pgwire::PgwireLite,
                              query: &str,
                              res_name: &str,
                              sq: bool,
                              ignore: bool| {
            info!("deleting [{}]...", res_name);
            show_query(sq, query);
            if has_returning_clause(query) {
                let (msg, row) = run_stackql_dml_returning(query, client, ignore, 0, 0);
                debug!("Delete response: {}", msg);
                row
            } else {
                let msg = run_stackql_command(query, client, ignore, 0, 0);
                debug!("Delete response: {}", msg);
                None
            }
        };

        // Helper closure: run the exists query and return the count.
        // Returns Ok(count) or Err(msg) for unexpected results.
        let run_exists_count = |client: &mut crate::utils::pgwire::PgwireLite,
                                query: &str,
                                res_name: &str,
                                sq: bool|
         -> Result<i64, String> {
            info!("running post-delete check for [{}]...", res_name);
            show_query(sq, query);
            let result = run_stackql_query(query, client, true, 0, 5);
            if result.is_empty() {
                return Ok(0); // no rows → resource gone
            }
            if result[0].contains_key("_stackql_deploy_error") || result[0].contains_key("error") {
                return Ok(0); // error querying → treat as gone
            }
            if let Some(count_str) = result[0].get("count") {
                if let Ok(count) = count_str.parse::<i64>() {
                    return Ok(count);
                }
            }
            // No count field — check if all field values are null/empty
            // (resource gone) or any non-null value (resource still exists).
            let row = &result[0];
            let all_null = row.values().all(|v| v == "null" || v.is_empty());
            if all_null {
                Ok(0) // all null/empty → resource gone
            } else {
                Ok(1) // non-null value → resource still exists
            }
        };

        // --- no-retry path: single delete + single check ---
        if delete_retries == 0 {
            let row = execute_delete(
                &mut self.client,
                delete_query,
                &resource.name,
                show_queries,
                ignore_errors,
            );
            if returning_row.is_none() {
                returning_row = row;
            }
            match run_exists_count(&mut self.client, exists_query, &resource.name, show_queries) {
                Ok(0) => {
                    info!("[{}] confirmed deleted", resource.name);
                    return (returning_row, true);
                }
                Ok(1) => {
                    info!(
                        "[{}] delete dispatched (resource may still be deleting asynchronously)",
                        resource.name
                    );
                    return (returning_row, false);
                }
                Ok(n) => {
                    catch_error_and_exit(&format!(
                        "Post-delete exists query for [{}] returned count={} (expected 0 or 1). \
                         This indicates a query or logic error.",
                        resource.name, n
                    ));
                }
                Err(msg) => {
                    catch_error_and_exit(&msg);
                }
            }
        }

        // --- retry path: interleaved delete + check loop ---
        let start = std::time::Instant::now();

        for attempt in 0..delete_retries {
            // Step 1: execute DELETE
            let row = execute_delete(
                &mut self.client,
                delete_query,
                &resource.name,
                show_queries,
                ignore_errors,
            );
            if returning_row.is_none() {
                returning_row = row;
            }

            // Step 2: immediate post-delete check
            match run_exists_count(&mut self.client, exists_query, &resource.name, show_queries) {
                Ok(0) => {
                    info!("[{}] confirmed deleted", resource.name);
                    return (returning_row, true);
                }
                Ok(1) => {
                    let elapsed = start.elapsed().as_secs();
                    info!(
                        "[{}] still exists after delete, attempt {}/{} ({} seconds elapsed)",
                        resource.name,
                        attempt + 1,
                        delete_retries,
                        elapsed
                    );
                }
                Ok(n) => {
                    catch_error_and_exit(&format!(
                        "Post-delete exists query for [{}] returned count={} (expected 0 or 1). \
                         This indicates a query or logic error.",
                        resource.name, n
                    ));
                }
                Err(msg) => {
                    catch_error_and_exit(&msg);
                }
            }

            // Step 3: wait retry_delay
            if delete_retry_delay > 0 {
                info!(
                    "[{}] waiting {} seconds before next attempt...",
                    resource.name, delete_retry_delay
                );
                std::thread::sleep(std::time::Duration::from_secs(delete_retry_delay as u64));
            }

            // Step 4: check again after the delay (maybe it cleaned up)
            match run_exists_count(&mut self.client, exists_query, &resource.name, show_queries) {
                Ok(0) => {
                    info!("[{}] confirmed deleted", resource.name);
                    return (returning_row, true);
                }
                Ok(1) => {
                    let elapsed = start.elapsed().as_secs();
                    info!(
                        "[{}] still exists after delay, attempt {}/{} ({} seconds elapsed), re-issuing delete...",
                        resource.name,
                        attempt + 1,
                        delete_retries,
                        elapsed
                    );
                    // Loop continues → next iteration will re-issue DELETE
                }
                Ok(n) => {
                    catch_error_and_exit(&format!(
                        "Post-delete exists query for [{}] returned count={} (expected 0 or 1). \
                         This indicates a query or logic error.",
                        resource.name, n
                    ));
                }
                Err(msg) => {
                    catch_error_and_exit(&msg);
                }
            }
        }

        // Exhausted all retries
        let elapsed = start.elapsed().as_secs();
        info!(
            "[{}] delete could not be confirmed after {} attempts ({} seconds elapsed)",
            resource.name, delete_retries, elapsed
        );
        (returning_row, false)
    }

    // -----------------------------------------------------------------------
    // RETURNING * capture and callback support
    // -----------------------------------------------------------------------

    /// Store a RETURNING * row for `resource_name` in the global context.
    ///
    /// Flat keys (`callback.{field}`, `{resource_name}.callback.{field}`) are
    /// inserted so they are accessible from subsequent template renders:
    /// - The unscoped `callback.*` form is available to the resource's own
    ///   `.iql` templates (and is overwritten by the next DML that has
    ///   RETURNING *).
    /// - The scoped `{resource_name}.callback.*` form is available to any
    ///   downstream resource (written once, never overwritten).
    pub fn store_callback_data(
        &mut self,
        resource_name: &str,
        returning_row: &HashMap<String, String>,
    ) {
        debug!(
            "storing RETURNING * result for [{}] in callback context",
            resource_name
        );
        flatten_returning_row(returning_row, resource_name, &mut self.global_context);
    }

    /// Execute a callback block associated with a DML operation.
    ///
    /// 1. If `short_circuit_field` is set and the field in the current context
    ///    equals `short_circuit_value`, skip polling.
    /// 2. Otherwise poll the callback query up to `retries` times.
    /// 3. On exhaustion, call `catch_error_and_exit`.
    ///
    /// `operation` is used only for log messages (e.g. `"create"`).
    #[allow(clippy::too_many_arguments)]
    pub fn run_callback(
        &mut self,
        resource: &Resource,
        callback_query: &str,
        retries: u32,
        retry_delay: u32,
        short_circuit_field: Option<&str>,
        short_circuit_value: Option<&str>,
        operation: &str,
        dry_run: bool,
        show_queries: bool,
    ) {
        if dry_run {
            info!(
                "dry run callback ({}) for [{}]:\n\n/* callback query */\n{}\n\
                 [dry run: callback polling skipped]\n",
                operation, resource.name, callback_query
            );
            return;
        }

        // Short-circuit check.
        if let (Some(field), Some(expected)) = (short_circuit_field, short_circuit_value) {
            if check_short_circuit(&self.global_context, field, expected) {
                info!(
                    "[{}] {} callback short-circuited (field '{}' = '{}')",
                    resource.name, operation, field, expected
                );
                return;
            }
        }

        info!("running {} callback for [{}]...", operation, resource.name);
        show_query(show_queries, callback_query);

        let succeeded = run_callback_poll(
            &resource.name,
            callback_query,
            retries,
            retry_delay,
            &mut self.client,
        );

        if !succeeded {
            catch_error_and_exit(&format!(
                "callback timeout for [{}] {} operation after {} retries",
                resource.name, operation, retries
            ));
        }

        info!(
            "[{}] {} callback completed successfully",
            resource.name, operation
        );
    }

    /// Run an optional troubleshoot diagnostic query after a failed operation.
    ///
    /// Looks for a `troubleshoot:{operation}` anchor first, falling back to a
    /// generic `troubleshoot` anchor.  If found, the query is rendered with
    /// `try_render_query` (tolerant of missing variables) and executed once.
    /// Results are logged at `error!()` level so the user sees them alongside
    /// the failure message.
    ///
    /// This method never causes the build/teardown to fail - if the
    /// troubleshoot query itself errors, a warning is logged and execution
    /// continues to the original error path.
    pub fn run_troubleshoot(
        &mut self,
        resource: &Resource,
        resource_queries: &HashMap<String, ParsedQuery>,
        operation: &str,
        full_context: &HashMap<String, String>,
        show_queries: bool,
    ) {
        // Look up operation-specific anchor first, fall back to generic
        let specific = format!("troubleshoot:{}", operation);
        let anchor = if resource_queries.contains_key(&specific) {
            &specific
        } else if resource_queries.contains_key("troubleshoot") {
            "troubleshoot"
        } else {
            return;
        };

        let pq = resource_queries.get(anchor).unwrap();

        let rendered =
            match self.try_render_query(&resource.name, anchor, &pq.template, full_context) {
                Some(q) => q,
                None => {
                    // Extract referenced variables from template to report
                    // which ones are missing
                    let re = regex::Regex::new(r"\{\{\s*([\w.]+)").unwrap();
                    let missing: Vec<&str> = re
                        .captures_iter(&pq.template)
                        .filter_map(|c| c.get(1).map(|m| m.as_str()))
                        .filter(|v| !full_context.contains_key(*v))
                        .collect();
                    warn!(
                        "[{}] troubleshoot query could not be rendered, missing variables: {:?}",
                        resource.name, missing
                    );
                    return;
                }
            };

        info!(
            "running troubleshoot query for [{}] ({})...",
            resource.name, operation
        );
        show_query(show_queries, &rendered);

        let results = run_stackql_query(
            &rendered,
            &mut self.client,
            true,
            pq.options.retries,
            pq.options.retry_delay,
        );

        if results.is_empty() {
            warn!("[{}] troubleshoot query returned no results", resource.name);
            return;
        }

        if let Ok(json) = serde_json::to_string_pretty(&results) {
            info!(
                "[{}] troubleshoot diagnostics ({}):\n\n{}\n",
                resource.name, operation, json
            );
        } else {
            // Fallback if JSON serialization fails
            info!(
                "[{}] troubleshoot diagnostics ({}): {:?}",
                resource.name, operation, results
            );
        }
    }

    /// Run a command-type query.
    pub fn run_command(
        &mut self,
        command_query: &str,
        retries: u32,
        retry_delay: u32,
        dry_run: bool,
        show_queries: bool,
    ) {
        if dry_run {
            info!("dry run command:\n\n{}\n", command_query);
            return;
        }

        info!("running command...");
        show_query(show_queries, command_query);
        let result =
            run_stackql_command(command_query, &mut self.client, false, retries, retry_delay);
        if result.is_empty() {
            debug!("Command response: no response");
        } else {
            debug!("Command response:\n\n{}\n", result);
        }
    }

    /// Process exports for a resource.
    #[allow(clippy::too_many_arguments)]
    pub fn process_exports(
        &mut self,
        resource: &Resource,
        _full_context: &HashMap<String, String>,
        exports_query: &str,
        retries: u32,
        retry_delay: u32,
        dry_run: bool,
        show_queries: bool,
        ignore_missing_exports: bool,
    ) {
        let expected_exports = &resource.exports;
        if expected_exports.is_empty() {
            return;
        }

        let all_dicts = expected_exports.iter().all(|e| e.is_mapping());
        let protected_exports = &resource.protected;

        if dry_run {
            let mut export_data = HashMap::new();
            if all_dicts {
                for item in expected_exports {
                    if let Some(map) = item.as_mapping() {
                        for (_, val) in map {
                            if let Some(v) = val.as_str() {
                                export_data
                                    .insert(v.to_string(), DRY_RUN_EXPORT_PLACEHOLDER.to_string());
                            }
                        }
                    }
                }
            } else {
                for item in expected_exports {
                    if let Some(s) = item.as_str() {
                        export_data.insert(s.to_string(), DRY_RUN_EXPORT_PLACEHOLDER.to_string());
                    }
                }
            }
            export_vars(
                &mut self.global_context,
                &resource.name,
                &export_data,
                protected_exports,
            );
            info!(
                "dry run exports query for [{}]:\n\n/* exports query */\n{}\n",
                resource.name, exports_query
            );
            return;
        }

        info!("exporting variables for [{}]...", resource.name);
        show_query(show_queries, exports_query);

        let exports =
            run_stackql_query(exports_query, &mut self.client, true, retries, retry_delay);

        debug!("Exports result: {:?}", exports);

        if exports.is_empty() {
            if ignore_missing_exports {
                // During teardown the resource may already be partially
                // deleted; mark its exports as unknown and carry on.
                self.set_exports_unknown(resource);
                return;
            }
            show_query(true, exports_query);
            catch_error_and_exit(&format!("Exports query failed for {}", resource.name));
        }

        // Check for errors
        if let Some(err_msg) = exports[0]
            .get("_stackql_deploy_error")
            .or_else(|| exports[0].get("error"))
        {
            if ignore_missing_exports {
                // During teardown a provider error on an exports query is
                // not fatal (fatal network/auth errors already exited in
                // run_stackql_query): the resource may be mid-deletion or
                // unreachable. Mark the exports unknown so dependants are
                // skipped, and carry on.
                warn!(
                    "exports query for [{}] failed during teardown, marking exports as unknown:\n\n{}\n",
                    resource.name, err_msg
                );
                self.set_exports_unknown(resource);
                return;
            }
            show_query(true, exports_query);
            catch_error_and_exit(&format!(
                "Exports query failed for {}\n\nError details:\n{}",
                resource.name, err_msg
            ));
        }

        if exports.len() > 1 {
            catch_error_and_exit(&format!(
                "Exports should include one row only, received {} rows",
                exports.len()
            ));
        }

        self.process_export_data(
            resource,
            &exports,
            expected_exports,
            all_dicts,
            protected_exports,
        );
    }

    /// Set every declared export of `resource` to the teardown
    /// [`crate::core::utils::UNKNOWN_EXPORT_PLACEHOLDER`] in the global
    /// context.
    ///
    /// Called during teardown when the exports could not be collected: the
    /// exports query returned no rows, could not be rendered, or the resource
    /// was skipped via `skip_on_delete`. Downstream queries that interpolate
    /// the placeholder are skipped rather than executed.
    pub fn set_exports_unknown(&mut self, resource: &Resource) {
        if resource.exports.is_empty() {
            return;
        }
        let fallback = unknown_exports_for(&resource.exports);
        export_vars(
            &mut self.global_context,
            &resource.name,
            &fallback,
            &resource.protected,
        );
    }

    /// Process exports from an already-obtained result (e.g., from exports proxy).
    pub fn process_exports_from_result(
        &mut self,
        resource: &Resource,
        exports_result: &[HashMap<String, String>],
    ) {
        let expected_exports = &resource.exports;
        if expected_exports.is_empty() || exports_result.is_empty() {
            return;
        }

        let all_dicts = expected_exports.iter().all(|e| e.is_mapping());
        let protected_exports = &resource.protected;

        if exports_result.len() > 1 {
            catch_error_and_exit(&format!(
                "Exports should include one row only, received {} rows",
                exports_result.len()
            ));
        }

        self.process_export_data(
            resource,
            exports_result,
            expected_exports,
            all_dicts,
            protected_exports,
        );
    }

    /// Internal helper to extract export data from query results.
    fn process_export_data(
        &mut self,
        resource: &Resource,
        exports: &[HashMap<String, String>],
        expected_exports: &[serde_yaml::Value],
        all_dicts: bool,
        protected_exports: &[String],
    ) {
        let export_row = if exports.is_empty() {
            HashMap::new()
        } else {
            exports[0].clone()
        };

        let mut export_data = HashMap::new();

        for item in expected_exports {
            if all_dicts {
                if let Some(map) = item.as_mapping() {
                    for (key_val, val_val) in map {
                        let key = key_val.as_str().unwrap_or("");
                        let val = val_val.as_str().unwrap_or("");
                        // key in expected_exports maps to key in export_row
                        // val becomes the key in export_data
                        let exported_value = export_row.get(key).cloned().unwrap_or_default();
                        export_data.insert(val.to_string(), exported_value);
                    }
                }
            } else {
                let item_name = item.as_str().unwrap_or("");
                if !item_name.is_empty() {
                    let exported_value = export_row.get(item_name).cloned().unwrap_or_default();
                    export_data.insert(item_name.to_string(), exported_value);
                }
            }
        }

        export_vars(
            &mut self.global_context,
            &resource.name,
            &export_data,
            protected_exports,
        );
    }

    /// Process a script resource type.
    pub fn process_script_resource(
        &mut self,
        resource: &Resource,
        dry_run: bool,
        full_context: &HashMap<String, String>,
    ) {
        info!("Running script for {}...", resource.name);

        let script_template = match &resource.run {
            Some(s) => s.clone(),
            None => {
                catch_error_and_exit("Script resource must include 'run' key");
            }
        };

        let script = render_string_value(&self.engine, &script_template, full_context);

        if dry_run {
            let dry_run_script = script.replace("\"\"", "\"<evaluated>\"");
            info!(
                "dry run script for [{}]:\n\n{}\n",
                resource.name, dry_run_script
            );
        } else {
            info!("running script for [{}]...", resource.name);

            let export_names: Vec<String> = resource
                .exports
                .iter()
                .filter_map(|e| e.as_str().map(|s| s.to_string()))
                .collect();

            let export_names_opt = if export_names.is_empty() {
                None
            } else {
                Some(export_names.as_slice())
            };

            if let Some(ret_vars) = run_ext_script(&script, export_names_opt) {
                if !resource.exports.is_empty() {
                    info!("Exported variables from script: {:?}", ret_vars);
                    export_vars(
                        &mut self.global_context,
                        &resource.name,
                        &ret_vars,
                        &resource.protected,
                    );
                }
            }
        }
    }

    /// Process stack-level exports to a JSON output file.
    pub fn process_stack_exports(
        &self,
        dry_run: bool,
        output_file: Option<&str>,
        elapsed_time: &str,
    ) {
        let manifest_exports = &self.manifest.exports;

        if manifest_exports.is_empty() {
            return;
        }

        if dry_run {
            let total_vars = manifest_exports.len() + 3;
            info!(
                "dry run: would export {} variables (including automatic stack_name, stack_env, and elapsed_time)",
                total_vars
            );
            return;
        }

        let mut export_data = serde_json::Map::new();
        let mut missing_vars = Vec::new();

        // Always include stack metadata
        export_data.insert(
            "stack_name".to_string(),
            serde_json::Value::String(self.stack_name.clone()),
        );
        export_data.insert(
            "stack_env".to_string(),
            serde_json::Value::String(self.stack_env.clone()),
        );

        for var_name in manifest_exports {
            if var_name == "stack_name" || var_name == "stack_env" {
                continue;
            }

            if let Some(value) = self.global_context.get(var_name) {
                if value.starts_with('[') || value.starts_with('{') {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value) {
                        export_data.insert(var_name.clone(), parsed);
                        continue;
                    }
                }
                export_data.insert(var_name.clone(), serde_json::Value::String(value.clone()));
            } else {
                missing_vars.push(var_name.clone());
            }
        }

        if !missing_vars.is_empty() {
            catch_error_and_exit(&format!(
                "Exports failed: variables not found in context: {:?}",
                missing_vars
            ));
        }

        // Add elapsed_time
        export_data.insert(
            "elapsed_time".to_string(),
            serde_json::Value::String(elapsed_time.to_string()),
        );

        // Display stack exports table
        print_unicode_box("stack exports", BorderColor::Cyan);

        // Build ASCII table
        // Env var names: STACKQL_DEPLOY__<stack>__<env>__<var> (hyphens -> underscores)
        let sanitize = |s: &str| s.replace('-', "_");
        let prefix = format!(
            "STACKQL_DEPLOY__{}__{}__",
            sanitize(&self.stack_name),
            sanitize(&self.stack_env)
        );
        let mut rows: Vec<(String, String)> = Vec::new();
        let mut max_name_len = 8usize; // "variable" header
        for (key, val) in &export_data {
            let fq_name = format!("{}{}", prefix, sanitize(key));
            let val_str = match val {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            max_name_len = max_name_len.max(fq_name.len());
            rows.push((fq_name, val_str));
        }
        let max_val_len = rows
            .iter()
            .map(|(_, v)| v.len())
            .max()
            .unwrap_or(5)
            .clamp(5, 80); // cap value display width

        let sep = format!(
            "+-{}-+-{}-+",
            "-".repeat(max_name_len),
            "-".repeat(max_val_len)
        );
        println!("{}", sep);
        println!(
            "| {:<width_n$} | {:<width_v$} |",
            "variable",
            "value",
            width_n = max_name_len,
            width_v = max_val_len
        );
        println!("{}", sep);
        for (name, val) in &rows {
            // Mask protected values in the displayed table (files keep real values)
            let val = crate::core::secrets::redact(val);
            let display_val = if val.len() > max_val_len {
                format!("{}...", &val[..max_val_len - 3])
            } else {
                val.clone()
            };
            println!(
                "| {:<width_n$} | {:<width_v$} |",
                name,
                display_val,
                width_n = max_name_len,
                width_v = max_val_len
            );
        }
        println!("{}", sep);

        // Write sourceable exports file
        let exports_file = ".stackql-deploy-exports";
        let mut export_lines = Vec::new();
        for (name, val) in &rows {
            // Escape single quotes in values
            let escaped = val.replace('\'', "'\\''");
            export_lines.push(format!("export {}='{}'", name, escaped));
        }
        match fs::write(exports_file, export_lines.join("\n") + "\n") {
            Ok(_) => {
                info!("{} variables written to {}", rows.len(), exports_file);
                println!();
                println!("To load these variables into your shell:");
                if cfg!(target_os = "windows") {
                    println!(
                        "  PowerShell:  Get-Content {} | ForEach-Object {{ Invoke-Expression $_ }}",
                        exports_file
                    );
                    println!("  Git Bash:    source {}", exports_file);
                } else {
                    println!("  source {}", exports_file);
                }
                println!();
            }
            Err(e) => {
                error!("Failed to write exports file {}: {}", exports_file, e);
            }
        }

        // Write JSON file if --output-file was specified
        if let Some(output_file) = output_file {
            if let Some(parent) = Path::new(output_file).parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    if let Err(e) = fs::create_dir_all(parent) {
                        catch_error_and_exit(&format!(
                            "Failed to create directory for output file: {}",
                            e
                        ));
                    }
                }
            }

            let json = serde_json::Value::Object(export_data);
            match fs::write(output_file, serde_json::to_string_pretty(&json).unwrap()) {
                Ok(_) => info!("Exports also written to {}", output_file),
                Err(e) => catch_error_and_exit(&format!(
                    "Failed to write exports file {}: {}",
                    output_file, e
                )),
            }
        }
    }
}

/// Evaluate a simple condition expression.
/// Supports: 'value1' == 'value2', 'value1' != 'value2', true, false
fn evaluate_simple_condition(condition: &str) -> Option<bool> {
    let trimmed = condition.trim();

    // Direct boolean values
    if trimmed == "true" || trimmed == "True" {
        return Some(true);
    }
    if trimmed == "false" || trimmed == "False" {
        return Some(false);
    }

    // Equality check: 'a' == 'b'
    if let Some((left, right)) = trimmed.split_once("==") {
        let l = left.trim().trim_matches('\'').trim_matches('"');
        let r = right.trim().trim_matches('\'').trim_matches('"');
        return Some(l == r);
    }

    // Inequality check: 'a' != 'b'
    if let Some((left, right)) = trimmed.split_once("!=") {
        let l = left.trim().trim_matches('\'').trim_matches('"');
        let r = right.trim().trim_matches('\'').trim_matches('"');
        return Some(l != r);
    }

    // `in` check: 'a' in ['a', 'b']
    if trimmed.contains(" in ") {
        let parts: Vec<&str> = trimmed.splitn(2, " in ").collect();
        if parts.len() == 2 {
            let needle = parts[0].trim().trim_matches('\'').trim_matches('"');
            let haystack = parts[1].trim();
            // Simple list check
            if haystack.starts_with('[') && haystack.ends_with(']') {
                let items: Vec<&str> = haystack[1..haystack.len() - 1]
                    .split(',')
                    .map(|s| s.trim().trim_matches('\'').trim_matches('"'))
                    .collect();
                return Some(items.contains(&needle));
            }
        }
    }

    // `not in` check
    if trimmed.contains(" not in ") {
        let parts: Vec<&str> = trimmed.splitn(2, " not in ").collect();
        if parts.len() == 2 {
            let needle = parts[0].trim().trim_matches('\'').trim_matches('"');
            let haystack = parts[1].trim();
            if haystack.starts_with('[') && haystack.ends_with(']') {
                let items: Vec<&str> = haystack[1..haystack.len() - 1]
                    .split(',')
                    .map(|s| s.trim().trim_matches('\'').trim_matches('"'))
                    .collect();
                return Some(!items.contains(&needle));
            }
        }
    }

    None
}
