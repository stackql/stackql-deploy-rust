//! Integration tests for the `build` flow.
//!
//! These drive `run_build` against the same fixture stack and mock server as
//! the teardown tests. They double as a smoke test of the harness (a full
//! exists -> create -> post-create exists -> statecheck -> exports cycle) and
//! pin down that `skip_on_delete` is a teardown-only setting.

mod common;

use common::{export_value, FakeCloud, MockServer, TestStack};
use stackql_deploy::commands::build::run_build;

const PROVIDERS: &[&str] = &[
    "databricks_account::v26.07.00430",
    "databricks_workspace::v26.07.00430",
];
const WORKSPACES: &str = "databricks_account.provisioning.workspaces";
const CURRENT_USER: &str = "databricks_workspace.iam.current_user";
const STORAGE_CREDENTIALS: &str = "databricks_workspace.catalog.storage_credentials";

/// Nothing deployed yet; each table becomes present with these values once
/// its `INSERT` arrives.
fn empty_cloud() -> FakeCloud {
    FakeCloud::new(PROVIDERS)
        .table_absent(
            WORKSPACES,
            &[
                ("workspace_id", "7474649669073866"),
                ("deployment_name", "dbc-d586e37e-0269"),
                ("workspace_status", "RUNNING"),
            ],
        )
        .table_present(CURRENT_USER, &[("userName", "cicd-sp@example.com")])
        .table_absent(STORAGE_CREDENTIALS, &[("id", "07abf775-f0de-4749")])
}

fn mutations(server: &MockServer) -> Vec<String> {
    server
        .queries()
        .into_iter()
        .filter(|q| {
            let u = q.trim_start().to_ascii_uppercase();
            u.starts_with("INSERT") || u.starts_with("UPDATE") || u.starts_with("DELETE")
        })
        .collect()
}

#[test]
fn build_creates_missing_resources_and_chains_exports() {
    let server = MockServer::start(empty_cloud().into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_build(&mut runner, false, false, "Error", None);

    let inserts = mutations(&server);
    assert_eq!(inserts.len(), 2, "expected two creates, got: {:?}", inserts);
    assert!(inserts[0].contains(WORKSPACES));
    // The second create was rendered with the export of the first.
    assert!(inserts[1].contains(STORAGE_CREDENTIALS));
    assert!(inserts[1].contains("'dbc-d586e37e-0269'"));

    assert_eq!(
        export_value(&runner, "deployment_name").as_deref(),
        Some("dbc-d586e37e-0269")
    );
    assert_eq!(
        export_value(&runner, "workspace_principal").as_deref(),
        Some("cicd-sp@example.com")
    );
    assert_eq!(
        export_value(&runner, "storage_credential_id").as_deref(),
        Some("07abf775-f0de-4749")
    );
    assert_eq!(
        export_value(&runner, "workspace_status").as_deref(),
        Some("RUNNING")
    );
}

#[test]
fn build_still_executes_queries_flagged_skip_on_delete() {
    // skip_on_delete only affects teardown: on build the readiness query
    // runs and its export is populated as usual.
    let server = MockServer::start(empty_cloud().into_handler());
    let stack = TestStack::with_manifest("databricks_workspace", "manifest_skip_query.yml");
    let mut runner = stack.runner(&server, "dev");

    run_build(&mut runner, false, false, "Error", None);

    assert!(
        server.received(CURRENT_USER),
        "skip_on_delete must not suppress the query during build"
    );
    assert_eq!(
        export_value(&runner, "workspace_principal").as_deref(),
        Some("cicd-sp@example.com")
    );
}

#[test]
fn build_still_creates_resources_flagged_skip_on_delete() {
    let server = MockServer::start(empty_cloud().into_handler());
    let stack = TestStack::with_manifest("databricks_workspace", "manifest_skip_resource.yml");
    let mut runner = stack.runner(&server, "dev");

    run_build(&mut runner, false, false, "Error", None);

    let inserts = mutations(&server);
    assert!(
        inserts.iter().any(|q| q.contains(WORKSPACES)),
        "skip_on_delete must not suppress create during build, got: {:?}",
        inserts
    );
}

#[test]
fn dry_run_build_sends_no_mutations() {
    let server = MockServer::start(empty_cloud().into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_build(&mut runner, true, false, "Error", None);

    let sent = mutations(&server);
    assert!(sent.is_empty(), "dry run sent mutations: {:?}", sent);
}
