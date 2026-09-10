# Tests

Two layers, both driven by `cargo test`:

| Layer | Files | Needs | Runs on |
|-------|-------|-------|---------|
| Mock | `build.rs`, `teardown.rs`, `test_command.rs`, `fixtures/` | nothing | every `cargo test`, `ci-scripts/test.sh`, the `CI` workflow |
| Live | `live.rs`, `live_stacks/` | stackql binary, AWS and GitHub credentials | `ci-scripts/integration-test.sh`, the `Integration Tests` workflow on pull requests |

Unit tests live next to the code in `src/` (`#[cfg(test)]` modules).

## Mock layer

The crate exposes a library target so tests can build a `CommandRunner` in
process and call `run_build`, `run_test`, and `run_teardown` directly.
`common/mock_server.rs` is a small in-process server speaking the PostgreSQL
simple-query protocol, exactly what the stackql server speaks, and it records
every statement it receives. `common/fake_cloud.rs` answers those statements
from a table of present or absent provider resources, so a test can assert on
the precise SQL that was (or was not) sent and on the exports left behind.

Fixtures under `fixtures/<stack>/` are ordinary stacks. A fixture can carry
several `manifest_*.yml` variants sharing one `resources/` directory;
`TestStack::with_manifest` installs the chosen one as `stackql_manifest.yml`
in a temporary copy.

```sh
cargo test                 # unit + mock tests, a few seconds, no network
```

## Live layer

`live.rs` runs the real binary (`CARGO_BIN_EXE_stackql-deploy`) against the
stacks in `live_stacks/`. The binary only reads the stack directory; it runs
with `target/live/` as its working directory, which is where the stackql
server keeps its provider cache (`.stackql/`) and where
`.stackql-deploy-exports`, `stackql.log` and `--output-file` documents are
written. That directory is gitignored and persists between runs, so provider
documents are pulled once. Each stack gets its own server port (5451 to
5453), so a developer's default server on 5444 is never touched. Resource
names carry a per-run suffix (`STACKQL_DEPLOY_LIVE_RUN_ID`, or a local
timestamp), so concurrent runs do not collide, and every test tears its stack
down, including on failure.

All resources are free: AWS Systems Manager Parameter Store standard
parameters and GitHub repository labels.

| Stack | Provider | What it exercises |
|-------|----------|-------------------|
| `aws_ssm` | `awscc`, `aws` | The full lifecycle: dry run, create, update and delete with `RETURNING *`, `return_vals`, `callback:create` / `callback:update` / `callback:delete` polling, `troubleshoot`, `statecheck`, `PatchDocument` update via `generate_patch_document`, `createorupdate` (`REPLACE`), `query` (file and inline), `command`, `script`, `if` conditions, `file()` directives, `merge`, per-environment `values`, `protected` masking with `--show-queries`, stack `exports` with `--output-file`, `test` pass and fail, teardown with `skip_on_delete`, idempotent re-build and re-teardown |
| `aws_ssm_onfailure` | `aws`, `awscc` | Two deletes that can never succeed. S3 `DeleteBucket` on a bucket that never existed is rejected synchronously: `--on-failure error` aborts, `--on-failure ignore` logs and continues. Cloud Control `DeleteResource` on a parameter that never existed fails asynchronously: the `troubleshoot:delete` anchor surfaces the ProgressEvent error through the `RequestToken` captured by `return_vals.delete` |
| `github_labels` | `github` | Plain REST provider without `RETURNING`: 404-as-not-found exists check, create, idempotent re-build, update, `test`, teardown |

### Running locally

```sh
# credentials in the environment (never in the repo)
export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... AWS_REGION=us-east-1
export STACKQL_GITHUB_USERNAME=<user> STACKQL_GITHUB_PASSWORD=<token with issues:write>
export GITHUB_OWNER=<owner> GITHUB_REPO=<repo>      # where the test label is created

bash ci-scripts/integration-test.sh                # everything
STACKQL_DEPLOY_LIVE_FILTER=aws_ssm bash ci-scripts/integration-test.sh   # one stack
cargo test --test live live_teardown_on_failure_modes -- --ignored --nocapture
```

The `stackql` binary must be on `PATH`. Script resources run through
`sh -c`, so on Windows run from Git Bash (or another shell with `sh` on
`PATH`). Expect around four minutes for `aws_ssm`; the statecheck retry
budget before an update is most of it.

### In CI

`.github/workflows/integration-tests.yml` runs the live layer on every pull
request to `main` that touches `src/`, `tests/`, `ci-scripts/` or the Cargo
files, and on `workflow_dispatch`. It needs two repository secrets,
`AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, for an IAM principal that
may manage SSM parameters under `/stackql-deploy/` and call `s3:DeleteBucket`
(the call is expected to fail; no bucket is ever created). The GitHub stack
uses the workflow's own `GITHUB_TOKEN` with `issues: write`. Pull requests
from forks cannot see secrets, so the job skips itself for them.

To make it a merge gate, add the `Live provider tests` job to the required
status checks of the `main` branch protection rule.

### Adding a live stack

1. Create `live_stacks/<name>/` with a `stackql_manifest.yml` and
   `resources/`. Use only free resources, include the run id in every
   resource name, and make sure `teardown` removes everything the stack
   creates. Take manifest inputs with `-e` (globals are rendered from `-e`
   flags and a `.env` file, not from the process environment).
2. Add a `#[test] #[ignore]` function in `live.rs` using `LiveStack`,
   `TeardownGuard`, and a new port.
3. Document the credentials it needs here and in
   `ci-scripts/integration-test.sh`.
