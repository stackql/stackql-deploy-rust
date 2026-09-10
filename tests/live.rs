//! Live integration tests: the real `stackql-deploy` binary against real
//! providers, using only free resources (SSM Parameter Store standard
//! parameters, GitHub repository labels) and a unique name suffix per run.
//!
//! These tests are `#[ignore]`d so that a plain `cargo test` never touches a
//! cloud account. Run them with credentials in the environment:
//!
//! ```text
//! cargo test --test live -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! or via `bash ci-scripts/integration-test.sh`, which is what the
//! pre-merge CI workflow runs. See `tests/README.md` for the environment
//! variables each stack needs.

mod common;

use common::cli::{
    env_or, export_str, read_exports_json, require_env, run_id, LiveStack, TeardownGuard,
};

const ENV: &str = "ci";

/// Every stackql-deploy code path reachable from a manifest, on AWS SSM
/// Parameter Store: create with RETURNING / return_vals / callback, statecheck,
/// PatchDocument update, createorupdate, query / command / script resources,
/// conditions, file() and merge, protected values, stack exports, dry runs,
/// test pass and fail, teardown with skip_on_delete, idempotent re-runs.
#[test]
#[ignore = "live AWS test; run with --ignored and AWS credentials in the environment"]
fn live_aws_ssm_full_lifecycle() {
    require_env("AWS_ACCESS_KEY_ID", "the AWS SSM live test");
    require_env("AWS_SECRET_ACCESS_KEY", "the AWS SSM live test");
    let region = env_or("AWS_REGION", "us-east-1");
    let run = run_id();
    let secret_v1 = format!("sqld-secret-{}-v1", run);
    let secret_v2 = format!("sqld-secret-{}-v2", run);

    let stack = LiveStack::new("aws_ssm", 5451);
    stack
        .input("AWS_REGION", &region)
        .input("RUN_ID", &run)
        .input("APP_CONFIG_VALUE", &secret_v1);
    let mut guard = TeardownGuard::new(&stack, ENV);

    let app_config_name = format!("/stackql-deploy/{}/{}/app-config", ENV, run);
    let feature_flags_name = format!("/stackql-deploy/{}/{}/feature-flags", ENV, run);
    let deployed_by = format!("stackql-deploy-live-{}", run);

    // 1. Dry run: every query rendered, nothing executed.
    stack
        .run("build", ENV, &["--dry-run"])
        .assert_success()
        .assert_contains("dry run create for [app_config]")
        .assert_contains("dry run script for [deploy_marker]")
        .assert_contains("dry-run build complete")
        .assert_not_contains("creating [app_config]...");

    // 2. Build from nothing.
    let exports_json = stack.scratch("exports.json");
    let out = stack.run(
        "build",
        ENV,
        &[
            "--show-queries",
            "--output-file",
            exports_json.to_str().unwrap(),
        ],
    );
    out.assert_success()
        // Cloud Control create with RETURNING *, return_vals and callback
        .assert_contains("creating [app_config]...")
        .assert_contains("RETURNING [Identifier] for [app_config] captured as [this.identifier]")
        .assert_contains("[app_config] create callback completed successfully")
        .assert_contains("[app_config] is in the desired state")
        .assert_contains("successfully deployed app_config")
        // query resource with skip_on_delete runs normally on build
        .assert_contains("successfully exported variables for query in app_config_lookup")
        // createorupdate resource
        .assert_contains("creating [feature_flags]...")
        .assert_contains("createorupdate for [feature_flags] is authoritative")
        // conditions
        .assert_contains("Skipping resource [prd_only_check] due to condition")
        // command and script resources
        .assert_contains("running command...")
        .assert_contains("Exported variables from script")
        // protected input is masked everywhere, including --show-queries
        .assert_not_contains(&secret_v1)
        .assert_contains("set protected variable [app_config_value]");
    assert!(stack.exports_file().is_file(), "exports file written");
    let exports = read_exports_json(&exports_json);
    assert_eq!(export_str(&exports, "app_config_name"), app_config_name);
    assert_eq!(export_str(&exports, "app_config_version"), "1");
    assert!(
        export_str(&exports, "app_config_arn").ends_with(&format!(":parameter{}", app_config_name))
    );
    assert_eq!(
        export_str(&exports, "feature_flags_name"),
        feature_flags_name
    );
    assert_eq!(
        export_str(&exports, "feature_flags_deployed_by"),
        deployed_by
    );
    assert_eq!(export_str(&exports, "param_count"), "2");
    assert_eq!(export_str(&exports, "deployed_by"), deployed_by);
    assert_eq!(export_str(&exports, "stack_env"), ENV);

    // 3. Test: everything is in the desired state.
    stack
        .run("test", ENV, &[])
        .assert_success()
        .assert_contains("test passed for app_config")
        .assert_contains("test passed for feature_flags")
        .assert_contains("Skipping resource [prd_only_check] due to condition")
        .assert_contains("Exported variables from script")
        .assert_not_contains(&secret_v1);

    // 4. Build again: idempotent, no create or update.
    stack
        .run("build", ENV, &[])
        .assert_success()
        .assert_contains("[app_config] is in the desired state")
        .assert_not_contains("creating [app_config]...")
        .assert_not_contains("updating [app_config]...");

    // 5. Change an input: statecheck fails, PatchDocument update, statecheck passes.
    stack.input("APP_CONFIG_VALUE", &secret_v2);
    let out = stack.run(
        "build",
        ENV,
        &["--output-file", exports_json.to_str().unwrap()],
    );
    out.assert_success()
        .assert_contains("[app_config] is not in the desired state")
        .assert_contains("updating [app_config]...")
        .assert_contains("[app_config] update callback completed successfully")
        .assert_contains("successfully deployed app_config")
        .assert_not_contains(&secret_v2);
    let exports = read_exports_json(&exports_json);
    assert_eq!(export_str(&exports, "app_config_version"), "2");

    // 6. Test with a stale input fails.
    stack.input("APP_CONFIG_VALUE", &secret_v1);
    stack
        .run("test", ENV, &[])
        .assert_failure()
        .assert_contains("[app_config] is not in the desired state")
        .assert_contains("test failed for app_config");
    stack.input("APP_CONFIG_VALUE", &secret_v2);

    // 7. Teardown dry run shows the deletes and the skipped query.
    stack
        .run("teardown", ENV, &["--dry-run"])
        .assert_success()
        .assert_contains("[app_config_lookup] skip_on_delete is set")
        .assert_contains("dry run delete for [feature_flags]")
        .assert_contains("dry run delete for [app_config]")
        .assert_contains("dry-run teardown complete");

    // 8. Teardown.
    stack
        .run("teardown", ENV, &[])
        .assert_success()
        .assert_contains(
            "[app_config_lookup] skip_on_delete is set, query not executed during teardown",
        )
        .assert_contains("successfully deleted feature_flags")
        .assert_contains(
            "RETURNING [RequestToken] for [app_config] captured as [this.RequestToken]",
        )
        .assert_contains("[app_config] delete callback completed successfully")
        .assert_contains("successfully deleted app_config")
        .assert_not_contains("could not be confirmed")
        .assert_contains("teardown complete");
    guard.disarm();

    // 9. Teardown again: nothing left, still succeeds.
    stack
        .run("teardown", ENV, &[])
        .assert_success()
        .assert_contains("resource [feature_flags] does not exist, skipping delete")
        .assert_contains("resource [app_config] does not exist, skipping delete");
}

/// Two deletes that can never succeed. S3 DeleteBucket on a bucket that
/// never existed is rejected synchronously (the exact failure from issue
/// #56's teardown log): the default aborts the run, `--on-failure ignore`
/// reports and continues. A Cloud Control DeleteResource on a parameter that
/// never existed fails asynchronously: the statement succeeds, the
/// post-delete check fails, and the `troubleshoot:delete` anchor surfaces the
/// ProgressEvent error through the RequestToken captured by
/// `return_vals.delete`.
#[test]
#[ignore = "live AWS test; run with --ignored and AWS credentials in the environment"]
fn live_teardown_on_failure_modes() {
    require_env("AWS_ACCESS_KEY_ID", "the AWS on-failure live test");
    require_env("AWS_SECRET_ACCESS_KEY", "the AWS on-failure live test");
    let region = env_or("AWS_REGION", "us-east-1");
    let run = run_id();

    let stack = LiveStack::new("aws_ssm_onfailure", 5452);
    stack.input("AWS_REGION", &region).input("RUN_ID", &run);

    // ghost_parameter is processed first (reverse manifest order), so its
    // troubleshoot diagnostics appear before the S3 failure aborts the run.
    stack
        .run("teardown", ENV, &[])
        .assert_failure()
        .assert_contains(
            "RETURNING [RequestToken] for [ghost_parameter] captured as [this.RequestToken]",
        )
        .assert_contains("[ghost_parameter] troubleshoot diagnostics (delete)")
        .assert_contains("\"ErrorCode\": \"NotFound\"")
        .assert_contains("[ghost_parameter] delete could not be confirmed")
        .assert_contains("deleting [ghost]...")
        .assert_contains("Exception during stackql command execution")
        .assert_contains("stackql-deploy operation failed");

    stack
        .run("teardown", ENV, &["--on-failure", "ignore"])
        .assert_success()
        .assert_contains("on-failure=ignore")
        .assert_contains("[ghost_parameter] troubleshoot diagnostics (delete)")
        .assert_contains("Command failed (ignored)")
        .assert_contains("[ghost] delete could not be confirmed")
        .assert_contains("whose delete could not be confirmed: [ghost_parameter], [ghost]")
        .assert_contains("teardown complete");
}

/// Plain REST provider (no RETURNING, 404-as-not-found exists check) on
/// GitHub repository labels: create, idempotent re-build, update, test,
/// teardown, idempotent re-teardown.
#[test]
#[ignore = "live GitHub test; run with --ignored and a GitHub token with issues:write in the environment"]
fn live_github_labels_lifecycle() {
    require_env("STACKQL_GITHUB_USERNAME", "the GitHub labels live test");
    require_env("STACKQL_GITHUB_PASSWORD", "the GitHub labels live test");
    let (default_owner, default_repo) = match std::env::var("GITHUB_REPOSITORY") {
        Ok(full) if full.contains('/') => {
            let (o, r) = full.split_once('/').unwrap();
            (o.to_string(), r.to_string())
        }
        _ => ("stackql".to_string(), "stackql-deploy-rs".to_string()),
    };
    let owner = env_or("GITHUB_OWNER", &default_owner);
    let repo = env_or("GITHUB_REPO", &default_repo);
    let run = run_id();
    let label_name = format!("stackql-deploy-live-{}", run);

    let stack = LiveStack::new("github_labels", 5453);
    stack
        .input("GITHUB_OWNER", &owner)
        .input("GITHUB_REPO", &repo)
        .input("RUN_ID", &run)
        .input("LABEL_COLOR", "0e8a16");
    let mut guard = TeardownGuard::new(&stack, ENV);

    // Build from nothing: 404 on the exists check means "create".
    let exports_json = stack.scratch("exports.json");
    stack
        .run(
            "build",
            ENV,
            &["--output-file", exports_json.to_str().unwrap()],
        )
        .assert_success()
        .assert_contains("[run_label] does not exist")
        .assert_contains("creating [run_label]...")
        .assert_contains("[run_label] is in the desired state")
        .assert_contains("successfully deployed run_label");
    let exports = read_exports_json(&exports_json);
    assert!(
        export_str(&exports, "run_label_id").parse::<u64>().is_ok(),
        "run_label_id is numeric"
    );
    assert!(export_str(&exports, "run_label_url").ends_with(&format!("/labels/{}", label_name)));
    assert!(
        export_str(&exports, "label_count")
            .parse::<u64>()
            .unwrap_or(0)
            >= 1
    );

    // Idempotent re-build.
    stack
        .run("build", ENV, &[])
        .assert_success()
        .assert_not_contains("creating [run_label]...")
        .assert_not_contains("updating [run_label]...");

    // Change the colour: update path.
    stack.input("LABEL_COLOR", "d73a4a");
    stack
        .run("build", ENV, &[])
        .assert_success()
        .assert_contains("[run_label] is not in the desired state")
        .assert_contains("updating [run_label]...")
        .assert_contains("successfully deployed run_label");

    stack
        .run("test", ENV, &[])
        .assert_success()
        .assert_contains("test passed for run_label");

    stack
        .run("teardown", ENV, &[])
        .assert_success()
        .assert_contains("successfully deleted run_label")
        .assert_contains("teardown complete");
    guard.disarm();

    stack
        .run("teardown", ENV, &[])
        .assert_success()
        .assert_contains("resource [run_label] does not exist, skipping delete");
}
