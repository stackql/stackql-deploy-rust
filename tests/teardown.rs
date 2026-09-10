//! Integration tests for the `teardown` flow.
//!
//! Each test drives `run_teardown` against a fixture stack and an in-process
//! mock stackql server, then asserts on the exact statements that were sent
//! and on the exports left in the runner's global context.
//!
//! Background (issue #56): tearing down a partially deleted stack used to
//! interpolate the `<unknown>` export placeholder into downstream queries,
//! e.g. `https://<unknown>.cloud.databricks.com/...`, which failed with a
//! fatal `dial tcp` error and aborted the whole teardown.

mod common;

use common::{
    assert_no_unknown_placeholder_sent, export_value, FakeCloud, MockResponse, MockServer,
    TestStack, UNKNOWN,
};
use stackql_deploy::commands::common_args::FailureAction;
use stackql_deploy::commands::teardown::run_teardown;

const PROVIDERS: &[&str] = &[
    "databricks_account::v26.07.00430",
    "databricks_workspace::v26.07.00430",
];
const WORKSPACES: &str = "databricks_account.provisioning.workspaces";
const CURRENT_USER: &str = "databricks_workspace.iam.current_user";
const STORAGE_CREDENTIALS: &str = "databricks_workspace.catalog.storage_credentials";

/// A fully deployed estate: every table present with realistic values.
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
fn teardown_skips_queries_that_depend_on_a_deleted_upstream_resource() {
    // The workspace was deleted by an earlier, partially successful teardown.
    // Everything that needs its deployment_name must be skipped, not run.
    let cloud = FakeCloud::new(PROVIDERS)
        .table_absent(WORKSPACES, &[])
        .table_present(CURRENT_USER, &[("userName", "cicd-sp@example.com")])
        .table_present(STORAGE_CREDENTIALS, &[("id", "07abf775-f0de-4749")]);
    let server = MockServer::start(cloud.into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Error);

    assert_no_unknown_placeholder_sent(&server);

    // The run did real work: it checked whether the workspace still exists.
    assert!(server.received(WORKSPACES));

    // Nothing that interpolates deployment_name was sent to the provider.
    assert!(
        !server.received(CURRENT_USER),
        "workspace_ready query must not run when deployment_name is unknown"
    );
    assert!(
        !server.received(STORAGE_CREDENTIALS),
        "storage_credential exists/delete must not run when deployment_name is unknown"
    );

    // Exports that could not be collected are marked unknown, not missing.
    assert_eq!(
        export_value(&runner, "deployment_name").as_deref(),
        Some(UNKNOWN)
    );
    assert_eq!(
        export_value(&runner, "workspace_id").as_deref(),
        Some(UNKNOWN)
    );
    assert_eq!(
        export_value(&runner, "workspace_principal").as_deref(),
        Some(UNKNOWN)
    );
    assert_eq!(
        export_value(&runner, "storage_credential_id").as_deref(),
        Some(UNKNOWN)
    );
    assert_eq!(
        export_value(&runner, "workspace_status").as_deref(),
        Some(UNKNOWN)
    );
}

#[test]
fn teardown_deletes_everything_when_the_estate_is_healthy() {
    // Control case for the test above: with all exports collectable, every
    // dependent query runs with real values and every resource is deleted.
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Error);

    assert_no_unknown_placeholder_sent(&server);

    assert_eq!(
        export_value(&runner, "deployment_name").as_deref(),
        Some("dbc-d586e37e-0269")
    );
    assert_eq!(
        export_value(&runner, "workspace_principal").as_deref(),
        Some("cicd-sp@example.com")
    );
    assert_eq!(
        export_value(&runner, "workspace_status").as_deref(),
        Some("RUNNING")
    );

    let deletes = server.queries_containing("DELETE FROM");
    assert_eq!(deletes.len(), 2, "expected two deletes, got: {:?}", deletes);
    // Reverse manifest order: the workspace-level resource goes first.
    assert!(deletes[0].contains(STORAGE_CREDENTIALS));
    assert!(deletes[0].contains("deployment_name = 'dbc-d586e37e-0269'"));
    assert!(deletes[1].contains(WORKSPACES));
    assert!(deletes[1].contains("workspace_id = '7474649669073866'"));
}

#[test]
fn skip_on_delete_query_is_not_executed_during_teardown() {
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::with_manifest("databricks_workspace", "manifest_skip_query.yml");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Error);

    assert_no_unknown_placeholder_sent(&server);

    // The flagged query never reaches the provider, even though it would
    // have succeeded, and its export is reported as unknown.
    assert!(
        !server.received(CURRENT_USER),
        "skip_on_delete query must not be executed during teardown"
    );
    assert_eq!(
        export_value(&runner, "workspace_principal").as_deref(),
        Some(UNKNOWN)
    );

    // Everything else still tears down normally with real values.
    let deletes = server.queries_containing("DELETE FROM");
    assert_eq!(deletes.len(), 2, "expected two deletes, got: {:?}", deletes);
    assert!(deletes[0].contains(STORAGE_CREDENTIALS));
    assert!(deletes[1].contains(WORKSPACES));
    assert_eq!(
        export_value(&runner, "workspace_status").as_deref(),
        Some("RUNNING")
    );
}

#[test]
fn skip_on_delete_resource_is_retained_but_its_exports_still_flow_downstream() {
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::with_manifest("databricks_workspace", "manifest_skip_resource.yml");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Error);

    assert_no_unknown_placeholder_sent(&server);

    // The retained resource is never deleted...
    let deletes = server.queries_containing("DELETE FROM");
    assert!(
        deletes.iter().all(|q| !q.contains(WORKSPACES)),
        "workspace must not be deleted, got: {:?}",
        deletes
    );
    // ...but its exports were collected and used by dependants.
    assert_eq!(
        export_value(&runner, "deployment_name").as_deref(),
        Some("dbc-d586e37e-0269")
    );
    assert!(server.received(CURRENT_USER));
    assert_eq!(deletes.len(), 1, "expected one delete, got: {:?}", deletes);
    assert!(deletes[0].contains(STORAGE_CREDENTIALS));
    assert!(deletes[0].contains("deployment_name = 'dbc-d586e37e-0269'"));
}

#[test]
fn teardown_tolerates_a_failing_exports_query() {
    // A non-fatal provider error on an exports query (e.g. the workspace API
    // rejecting the call while the workspace is being deleted) must not abort
    // the teardown; the export is marked unknown and dependants are skipped.
    let cloud = healthy_cloud().error_on(
        CURRENT_USER,
        "Query execution failed: query returns error: http response status code: 400, \
         response body: {\"error_code\":\"BAD_REQUEST\"}",
    );
    let server = MockServer::start(cloud.into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Error);

    assert_no_unknown_placeholder_sent(&server);
    assert!(server.received(CURRENT_USER));
    assert_eq!(
        export_value(&runner, "workspace_principal").as_deref(),
        Some(UNKNOWN)
    );

    let deletes = server.queries_containing("DELETE FROM");
    assert_eq!(deletes.len(), 2, "expected two deletes, got: {:?}", deletes);
}

#[test]
fn on_failure_ignore_continues_past_a_failed_delete() {
    // The provider rejects the storage credential delete (a non-fatal 409).
    // With --on-failure ignore the run logs it, reports the resource as not
    // confirmed deleted, and still tears down the workspace. With the
    // default (error) the same failure aborts the process, which is why only
    // the ignore path is covered in-process; the live suite covers the exit
    // code of the error path.
    let cloud = healthy_cloud().error_on(
        "DELETE FROM databricks_workspace.catalog.storage_credentials",
        "Query execution failed: query returns error: http response status code: 409,          response body: {\"error_code\":\"RESOURCE_CONFLICT\"}",
    );
    let server = MockServer::start(cloud.into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Ignore);

    assert_no_unknown_placeholder_sent(&server);
    let deletes = server.queries_containing("DELETE FROM");
    assert!(
        deletes.iter().any(|q| q.contains(STORAGE_CREDENTIALS)),
        "the failing delete must have been attempted"
    );
    assert!(
        deletes.iter().any(|q| q.contains(WORKSPACES)),
        "teardown must continue to the workspace after the ignored failure, got: {:?}",
        deletes
    );
}

#[test]
fn teardown_does_not_abort_on_script_resources() {
    // Scripts have no .iql file; collecting their exports used to try to
    // load one and exit. They are simply marked unknown.
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::with_manifest("databricks_workspace", "manifest_with_script.yml");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Error);

    assert_no_unknown_placeholder_sent(&server);
    assert_eq!(
        export_value(&runner, "hook_result").as_deref(),
        Some(UNKNOWN)
    );
    let deletes = server.queries_containing("DELETE FROM");
    assert_eq!(deletes.len(), 2, "expected two deletes, got: {:?}", deletes);
}

#[test]
fn delete_callback_is_skipped_when_no_returning_row_was_captured() {
    // No RETURNING row exists to poll, either because it is a dry run or
    // because the provider answered the DELETE with a bare command tag. Both
    // must skip the callback rather than fail to render `callback.*`.
    let stack =
        TestStack::with_manifest("databricks_workspace", "manifest_with_delete_callback.yml");

    let server = MockServer::start(healthy_cloud().into_handler());
    let mut runner = stack.runner(&server, "dev");
    run_teardown(&mut runner, true, false, FailureAction::Error);
    assert!(!server.received("workspace_status = 'DELETED'"));

    let cloud = healthy_cloud().answer(
        "DELETE FROM databricks_account.provisioning.workspaces",
        MockResponse::Command("DELETE 1".to_string()),
    );
    let server = MockServer::start(cloud.into_handler());
    let mut runner = stack.runner(&server, "dev");
    run_teardown(&mut runner, false, false, FailureAction::Error);
    assert!(server.received("DELETE FROM databricks_account.provisioning.workspaces"));
    assert!(!server.received("workspace_status = 'DELETED'"));
}

#[test]
fn delete_callback_runs_before_the_post_delete_check_and_postdelete_polling_is_honoured() {
    // The workspace delete returns a RETURNING row, and the delete only
    // takes effect after one further read (an asynchronous provider). The
    // exists anchor allows postdelete_retries=1, so the second check must
    // confirm the delete; and the callback must be polled before that check.
    let cloud = healthy_cloud().async_delete(WORKSPACES, 1).answer(
        "workspace_status = 'DELETED'",
        MockResponse::single_row(&[("success", "1")]),
    );
    let server = MockServer::start(cloud.into_handler());
    let stack =
        TestStack::with_manifest("databricks_workspace", "manifest_with_delete_callback.yml");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Error);

    let queries = server.queries();
    let delete_at = queries
        .iter()
        .position(|q| q.starts_with("DELETE FROM databricks_account.provisioning.workspaces"))
        .expect("workspace delete sent");
    let callback_at = queries
        .iter()
        .position(|q| q.contains("workspace_status = 'DELETED'"))
        .expect("delete callback polled");
    let checks_after_delete: Vec<usize> = queries
        .iter()
        .enumerate()
        .filter(|(i, q)| {
            *i > delete_at && q.contains("SELECT COUNT(*) AS count") && q.contains(WORKSPACES)
        })
        .map(|(i, _)| i)
        .collect();
    assert!(
        callback_at > delete_at && callback_at < checks_after_delete[0],
        "callback must run between the delete and the first post-delete check: {:?}",
        queries
    );
    assert_eq!(
        checks_after_delete.len(),
        2,
        "one immediate check plus one postdelete retry, got: {:?}",
        queries
    );
    assert_eq!(
        queries
            .iter()
            .filter(|q| q.starts_with("DELETE FROM databricks_account.provisioning.workspaces"))
            .count(),
        1,
        "only one delete was needed"
    );
}

#[test]
fn unconfirmed_delete_does_not_stop_the_run() {
    // The storage credential delete never takes effect within the
    // postdelete budget, so it ends the run unconfirmed after one delete
    // (the delete anchor's default retries=1); the workspace is still deleted.
    let cloud = healthy_cloud().async_delete(STORAGE_CREDENTIALS, 5);
    let server = MockServer::start(cloud.into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, false, false, FailureAction::Error);

    let queries = server.queries();
    assert_eq!(
        queries
            .iter()
            .filter(
                |q| q.starts_with("DELETE FROM databricks_workspace.catalog.storage_credentials")
            )
            .count(),
        1
    );
    assert!(server.received("DELETE FROM databricks_account.provisioning.workspaces"));
}

#[test]
fn dry_run_teardown_sends_no_mutations() {
    let server = MockServer::start(healthy_cloud().into_handler());
    let stack = TestStack::new("databricks_workspace");
    let mut runner = stack.runner(&server, "dev");

    run_teardown(&mut runner, true, false, FailureAction::Error);

    let mutations: Vec<String> = server
        .queries()
        .into_iter()
        .filter(|q| {
            let u = q.trim_start().to_ascii_uppercase();
            u.starts_with("DELETE") || u.starts_with("INSERT") || u.starts_with("UPDATE")
        })
        .collect();
    assert!(
        mutations.is_empty(),
        "dry run sent mutations: {:?}",
        mutations
    );
}
