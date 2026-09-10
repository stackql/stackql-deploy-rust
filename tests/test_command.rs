//! Integration tests for the `test` command against the mock server.

mod common;

use common::{export_value, FakeCloud, MockServer, TestStack};
use stackql_deploy::commands::build::run_build;
use stackql_deploy::commands::test::run_test;

const PROVIDERS: &[&str] = &[
    "databricks_account::v26.07.00430",
    "databricks_workspace::v26.07.00430",
];
const WORKSPACES: &str = "databricks_account.provisioning.workspaces";
const CURRENT_USER: &str = "databricks_workspace.iam.current_user";
const STORAGE_CREDENTIALS: &str = "databricks_workspace.catalog.storage_credentials";

fn healthy_cloud() -> FakeCloud {
    FakeCloud::new(PROVIDERS)
        .table_present(
            WORKSPACES,
            &[
                ("workspace_id", "7474649669073866"),
                ("deployment_name", "dbc-d586e37e-0269"),
                ("workspace_status", "RUNNING"),
            ],
        )
        .table_present(CURRENT_USER, &[("userName", "cicd-sp@example.com")])
        .table_present(STORAGE_CREDENTIALS, &[("id", "07abf775-f0de-4749")])
}

#[test]
fn test_command_validates_a_healthy_estate_without_mutations() {
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_test(&mut runner, false, false, "Error", None);

    let mutations: Vec<String> = server
        .queries()
        .into_iter()
        .filter(|q| {
            let u = q.trim_start().to_ascii_uppercase();
            u.starts_with("INSERT") || u.starts_with("UPDATE") || u.starts_with("DELETE")
        })
        .collect();
    assert!(mutations.is_empty(), "test sent mutations: {:?}", mutations);
    assert_eq!(
        export_value(&runner, "workspace_principal").as_deref(),
        Some("cicd-sp@example.com")
    );
    assert_eq!(
        export_value(&runner, "workspace_status").as_deref(),
        Some("RUNNING")
    );
}

#[test]
fn test_command_honours_if_conditions() {
    // estate_summary is gated on stack_env == prd; in dev it must not run.
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::with_manifest("databricks_workspace", "manifest_with_condition.yml");
    let mut runner = stack.runner(&server, "dev");

    run_test(&mut runner, false, false, "Error", None);

    assert!(
        !server.received("SELECT workspace_status"),
        "conditional query must not run when its condition is false"
    );
    assert!(export_value(&runner, "workspace_status").is_none());
}

#[test]
fn build_honours_if_conditions() {
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::with_manifest("databricks_workspace", "manifest_with_condition.yml");
    let mut runner = stack.runner(&server, "dev");

    run_build(&mut runner, false, false, "Error", None);

    assert!(!server.received("SELECT workspace_status"));
    assert!(export_value(&runner, "workspace_status").is_none());
}

/// Scripts run through `sh -c`, which is not on PATH in a default Windows
/// shell; the live suite covers scripts on Linux CI.
#[cfg(unix)]
#[test]
fn test_command_runs_script_resources_and_exports_their_output() {
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::with_manifest("databricks_workspace", "manifest_with_script.yml");
    let mut runner = stack.runner(&server, "dev");

    run_test(&mut runner, false, false, "Error", None);

    assert_eq!(export_value(&runner, "hook_result").as_deref(), Some("ok"));
    assert_ne!(
        export_value(&runner, "hook_result").as_deref(),
        Some(UNKNOWN)
    );
}
