# Changelog

## 2.2.0 (2026-09-10)

### Features

- `teardown --on-failure ignore` is now honoured. A `delete` statement the provider rejects is logged at `warn` level, the resource is reported as not confirmed deleted, and the teardown continues with the next resource; the default (`error`) still aborts at the first failure. Fatal errors (network, auth, planner) abort in both modes. Every teardown now ends with a summary of resources whose delete could not be confirmed. `rollback` is not meaningful for teardown and is treated as `error`.
- Added a live integration test suite (`tests/live.rs`, `tests/live_stacks/`) that runs the real binary against real providers using free resources only: AWS SSM Parameter Store parameters (Cloud Control and native API) and GitHub repository labels. It covers create with `RETURNING *`, `return_vals`, `callback`, `statecheck`, `PatchDocument` updates, `createorupdate`, `query`/`command`/`script` resources, conditions, `file()`, `merge`, per-environment values, protected masking, stack exports, `test` pass and fail, `teardown` with `skip_on_delete`, `--on-failure`, and idempotent re-runs. The tests are `#[ignore]`d for `cargo test`; `ci-scripts/integration-test.sh` runs them, and the new `Integration Tests` workflow runs them on pull requests to `main` as a merge gate (replacing the `Test Demo` placeholder workflow). See `tests/README.md`.
- Added `skip_on_delete: true` for resources ([#56](https://github.com/stackql/stackql-deploy-rs/issues/56)). A `query` resource with this flag is not executed during `teardown` and its declared exports are set to `<unknown>`; a `resource` or `multi` resource with this flag still has its exports collected (a downstream `delete` may need them) but its `delete` query is not executed, so the resource is retained. The flag has no effect on `build` or `test`.
- Added an integration test suite (`tests/`) that drives the `build` and `teardown` flows against an in-process mock stackql server speaking the PostgreSQL wire protocol, with fixture stacks under `tests/fixtures/`. Tests assert on the exact statements sent and need no stackql binary, provider registry, network access, or cloud credentials. `ci-scripts/test.sh` now runs `cargo test`, and the CI workflow triggers on `tests/**` and `ci-scripts/**` changes. To support this, the crate now has a library target alongside the binary; it is not a public API.

### Fixes

- Teardown no longer executes queries that interpolate the `<unknown>` export placeholder. When an upstream resource had already been deleted (for example by an earlier, partially successful teardown), its exports were set to `<unknown>` and then substituted into downstream `exists`, `exports`, `delete`, and inline `sql` queries. For a Databricks workspace this produced `https://<unknown>.cloud.databricks.com/...`, a fatal `dial tcp ... no such host` error, and an aborted teardown that could never complete. Such queries are now skipped with a log line naming the resource and anchor, and the dependent resource's exports are themselves marked `<unknown>` so the skip propagates consistently.
- Teardown now tolerates a non-fatal provider error on an `exports` query (fatal network and auth errors still abort). The resource's exports are marked `<unknown>` with a warning and the teardown continues; previously the error aborted the run.
- Teardown no longer aborts on stacks that contain a `script` resource. Export collection tried to load a `.iql` file for the script and exited when it was not found; script exports are now marked `<unknown>` (scripts are never executed on teardown).
- Inline `sql` on `query` resources is rendered tolerantly during teardown: a missing template variable skips the query instead of exiting the process.
- The `test` command now evaluates `if` conditions (it previously processed every resource regardless) and runs `script` resources the same way `build` does (it previously exited with `unknown resource type: script`).
- A `--dry-run` teardown now renders and logs each `delete` statement. Previously the dry-run exists check reported every resource as not found, so the run only showed "skipping delete".
- The `<evaluated>` and `<unknown>` export placeholders are no longer registered for log redaction when the export is `protected`. Previously a protected export in a dry run registered `<evaluated>` as a secret, which then masked every other placeholder in the run as `********`.
- Command failures that are ignored (`multi` resources, and now `--on-failure ignore`) are logged at `warn` level instead of `debug`.
- A `callback:delete` (or generic `callback`) anchor no longer aborts a `--dry-run` teardown, or a teardown whose delete returned no `RETURNING *` row. Callbacks poll the handle returned by `RETURNING *`, so they are now skipped with a log line when there is nothing to poll, matching `build`.
- The `postdelete_retries` and `postdelete_retry_delay` options on the `exists` anchor are now honoured, as documented (defaults 10 and 5). They were parsed but never used: the post-delete check ran once immediately and once more after the `delete` anchor's `retry_delay`, which defaults to 0, so an asynchronous delete (Cloud Control, most SaaS APIs) could only be confirmed by luck of timing. After each delete attempt the exists query now polls until the resource is gone, up to `postdelete_retries` times, `postdelete_retry_delay` seconds apart; only then is the delete re-issued, up to the `delete` anchor's `retries`.
- The `callback:delete` anchor now runs before the post-delete check rather than after it, so a provider's asynchronous delete is polled to completion before the resource is checked for absence.

## 2.1.1 (2026-08-24)

### Fixes

- StackQL planner/compiler errors (`could not locate symbol ...`, `cannot find matching operation ...`, `syntax error at position ...`, `disparity in fields to insert ...`) are now treated as fatal and abort the operation immediately. These errors are deterministic - the same query fails the same way on every attempt - but they were previously swallowed by the `exists`/`statecheck` retry loop and retried until the retry budget was exhausted.

## 2.1.0 (2026-08-24)

### Features

- Added `protected: true` for global variables and resource properties. The rendered value of a protected global or prop is masked (`********`) in all log output - including rendered queries shown with `--dry-run` or `--show-queries`, `DEBUG` level logging, and `RETURNING *` response logging - while the real value is still sent to the provider. Log scrubbing is value-based and applied at the logger sink, so a protected value is masked wherever it surfaces.

### Fixes

- Resource-level `protected` export values are now masked everywhere in log output, not just in the `set variable` export log lines. Previously a protected export interpolated into a downstream resource's query appeared in cleartext under `--dry-run` or `--show-queries`.
- The stack exports summary table now masks protected values in the terminal display. The `.stackql-deploy-exports` file and `--output-file` JSON retain real values.

## 2.0.9 (2026-08-21)

### Fixes

- Suppressed stray output from the stackql binary PATH lookup. The `which`/`where` check inherited the terminal's stdout, so the resolved binary path was printed once per lookup (four times during `info`). The check now captures the child process output and only inspects the exit status.

## 2.0.8 (2026-06-29)

### Features

- Set a `stackql-deploy/{version}` User-Agent on the stackql binary download client (and the template-scaffolding client used by `init`).

## 2.0.7 (2026-04-19)

### Fixes

- Fixed post-deploy failure for `createorupdate` resources whose `exports` anchor is a literal `SELECT` with no `FROM` clause (a supported and common pattern when the values to export are already known and an extra API round-trip is wasteful). Previously the exports query was executed as a statecheck proxy, and a FROM-less result caused the proxy check to report the resource was not in the desired state, aborting the run. With `createorupdate`, the DML is authoritative, so the exports-as-statecheck proxy is now skipped entirely - `exports` still runs to populate the global context for downstream resources.
- Fixed `stackql-deploy upgrade` downloading the stackql binary twice when the binary was missing: the pre-command binary check triggered a download, and the subcommand dispatch then triggered a second download. `upgrade` is now exempt from the pre-command binary check.
- Server-side notices from provider HTTP 4xx/5xx responses are now detected even when stackql wraps them as a generic `a notice level event has occurred` message with the real status code in the `DETAIL:` payload. Previously these escaped the error check and the `create`/`update`/`delete` operation silently appeared to succeed while the post-deploy `exists` check spun through its retries.
- Collapsed duplicate lines within a single notice's `DETAIL:` payload so repeated provider error bodies are printed once.
- Teardown now tolerates resources with unresolved template variables. If an `exists`, `exports`, or `delete` query references a variable that was never populated (because an upstream resource doesn't exist), the resource is treated as already torn down and skipped, instead of aborting the run. Stacks in a half-baked state can now be torn down cleanly.
- During teardown, `RETURNING` clauses are stripped from rendered `delete` DML when `return_vals.delete` is not configured for the resource. Some providers reject `RETURNING *` on `DELETE`, and teardown has no consumer for the returned data unless the manifest explicitly opts in via `return_vals.delete`. When `return_vals.delete` is configured the `RETURNING` clause is preserved and mapped fields are captured as `this.*` with non-fatal warnings if a mapping cannot be satisfied.
- Stale provider notices are no longer re-surfaced on subsequent queries. stackql emits a cumulative `NoticeResponse` on every query that includes every provider notice observed earlier in the session; the pgwire client now tracks each notice line already surfaced and drops byte-identical re-emissions. Dedup is exact-match (no canonicalization) so two distinct provider errors — which always differ in their embedded request/serving IDs — are never conflated. Fixes spurious `create`/`update`/`delete` failures where a 4xx provider response from an earlier `exists` SELECT was attributed to a later DML.

### Features

- When retries are exhausted on a `statecheck`, `exports` proxy, or post-deploy `exists` check, the last rendered query is now logged at `warn` level so the failing SQL is visible without needing `--show-queries` or `--log-level debug`. Pre-create exists checks (which fast-fail by design) stay silent.

## 2.0.6 (2026-03-28)

### Fixes

- Fixed eager rendering of `statecheck` queries that caused hard failures when `this.*` variables were not yet available (e.g. post-create exists re-run fails due to eventual consistency). `statecheck` now uses JIT rendering like `exports`, deferring gracefully when template variables are unresolved.
- When a deferred `statecheck` cannot be rendered post-deploy, the build falls through to `exports`-as-proxy validation or accepts the create/update based on successful execution.
- Applied the same fix to `teardown`, where `statecheck` used as an exists fallback would crash on unresolved variables instead of skipping the resource.
- Fixed `--dry-run` failures for resources that depend on exports from upstream resources. `create` and `update` query rendering now defers gracefully in dry-run mode when upstream exports are unavailable, and placeholder (`<evaluated>`) values are injected for unresolved exports so downstream resources can still render.
- When a post-create exists re-run fails to find a newly created resource (eventual consistency), the exists query is automatically retried using the `statecheck` retry settings if available, giving async providers time to make the resource discoverable.

### Features

- New optional `troubleshoot` IQL anchor for post-failure diagnostics. When a `build` post-deploy check fails or a `teardown` delete cannot be confirmed, a user-defined diagnostic query is automatically rendered and executed, with results logged as pretty-printed JSON. Supports operation-specific variants (`troubleshoot:create`, `troubleshoot:update`, `troubleshoot:delete`) with fallback to a generic `troubleshoot` anchor. Typically used with `return_vals` to capture an async operation handle (e.g. `RequestToken`) from `RETURNING *` and query its status via `{{ this.<field> }}`. See [resource query files documentation](https://stackql-deploy.io/docs/resource-query-files#troubleshoot) for details.
- The `RETURNING *` log message (`storing RETURNING * result...`) is now logged at `debug` level instead of `info`.

## 2.0.5 (2026-03-24)

### Fixes

- Network and authentication errors (DNS failures, 401/403 responses) are now detected early and surfaced as fatal errors instead of being silently retried.
- Unresolved template variables are caught at render time with a clear error message identifying the missing variable and source template.
- `command` type resources now log query output when using `RETURNING` clauses, matching the behavior of `resource` types.
- Stack level exports (`stack_name`, `stack_env`) are now set as scoped environment variables on the host system for use by external tooling.

## 2.0.4 (2026-03-18)

### Identifier capture from `exists` queries

The `exists` query can now return a named field (e.g. `vpc_id`) instead of `count`. The returned value is automatically captured as a resource-scoped variable (`{{ this.<field> }}`) and made available to all subsequent queries (`statecheck`, `exports`, `delete`) for that resource. This enables a two-step workflow where `exists` discovers the resource identifier and `statecheck` verifies its properties.

- When `exists` returns `null` or empty for the captured field, the resource is treated as non-existent
- Multiple rows from an `exists` (identifier pattern) or `exports` query is now a fatal error
- After a `create`, the `exists` query is automatically re-run to capture the identifier for use in post-deploy `statecheck` and `exports` queries

### `RETURNING *` identifier capture

When a `create` statement includes `RETURNING *` and the response contains an `Identifier` field, it is automatically injected as `this.identifier` — skipping the post-create `exists` re-run and saving an API call per resource.

### `return_vals` manifest field

New optional `return_vals` field on resources to explicitly map fields from `RETURNING *` responses to resource-scoped variables:

```yaml
return_vals:
  create:
    - Identifier: identifier   # rename pattern
    - ErrorCode                 # direct capture
```

If `return_vals` is specified but the field is missing from the response, the build fails.

### `to_aws_tag_filters` template filter

New AWS-specific Tera filter that converts `global_tags` (list of `Key`/`Value` pairs) to the AWS Resource Groups Tagging API `TagFilters` format:

```sql
AND TagFilters = '{{ global_tags | to_aws_tag_filters }}'
```

### YAML type preservation fix

Fixed an issue where YAML string values that look like numbers (e.g. `IpProtocol: "-1"`) were being coerced to integers during JSON serialization. String types declared in YAML are now preserved through to the rendered query.

### Teardown improvements

- Teardown no longer retries exports queries that return empty results — missing exports are set to `<unknown>` and teardown continues best-effort
- Post-delete existence checks accept the first empty response instead of retrying, reducing teardown time significantly

### AWS starter template updated

The `stackql-deploy init --provider aws` starter template now uses:
- `awscc` (Cloud Control) provider instead of `aws`
- CTE + INNER JOIN exists pattern with `to_aws_tag_filters`
- `AWS_POLICY_EQUAL` for statecheck tag comparison
- `this.<field>` identifier capture pattern
- `RETURNING *` on create statements
- `stackql:stack-name` / `stackql:stack-env` / `stackql:resource-name` tag taxonomy

### AWS VPC Web Server example

Complete rewrite of the `examples/aws/aws-vpc-webserver` stack (renamed from `aws-stack`) using the `awscc` provider exclusively. Includes 10 resources demonstrating all query patterns: tag-based discovery, identifier capture, property-level statechecks, PatchDocument updates, and the `to_aws_tag_filters` filter.

### Patch Document Test example

New `examples/aws/patch-doc-test` example demonstrating the Cloud Control API `UPDATE` workflow with `PatchDocument` — deploy an S3 bucket, modify its versioning config in the manifest, and re-deploy to apply the update.

### Other changes

- Fixed `init` command missing `--env` argument (defaulting to `dev`)
- Added `debug` log import to build command
- Debug logging now shows full `RETURNING *` payloads
- Documentation updates: `resource-query-files.md`, `template-filters.md`, `manifest-file.md`, and AWS template library

## 2.0.0 (2026-03-14)

### Initial Rust Release

This is the first release of **stackql-deploy** as a native Rust binary, replacing the Python implementation.

**Key changes from v1.x (Python):**
- Complete rewrite in Rust — single static binary, no Python runtime required
- Same CLI interface: `build`, `test`, `teardown`, `init`, `info`, `shell`, `upgrade`, `plan`
- Multi-platform binaries: Linux x86_64/ARM64, macOS Apple Silicon/Intel, Windows x86_64
- Available on [crates.io](https://crates.io/crates/stackql-deploy) via `cargo install stackql-deploy`

**The Python package (v1.x) is now archived.** See the [Python package changelog](https://github.com/stackql/stackql-deploy/blob/main/CHANGELOG.md) for the v1.x release history (last Python release: v1.9.4).
