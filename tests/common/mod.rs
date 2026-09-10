//! Shared integration-test harness.
//!
//! - [`mock_server`]: an in-process PostgreSQL-wire mock of the stackql server
//!   that records every statement it receives.
//! - [`fake_cloud`]: a stateful stand-in for provider tables.
//! - [`TestStack`]: copies a fixture stack from `tests/fixtures/<name>` into a
//!   temporary directory, optionally swapping in one of the fixture's
//!   alternative manifests, and builds a `CommandRunner` connected to the
//!   mock server.
//! - [`cli`]: runs the real binary against the live stacks under
//!   `tests/live_stacks/` (used only by the `#[ignore]`d tests in
//!   `tests/live.rs`).
//!
//! The mock layer needs no stackql binary, provider registry, or cloud
//! credentials.

// Each test binary compiles this module separately and uses a different
// subset of it.
#![allow(dead_code, unused_imports)]

pub mod cli;
pub mod fake_cloud;
pub mod mock_server;

use std::fs;
use std::path::{Path, PathBuf};

use stackql_deploy::commands::base::CommandRunner;
use stackql_deploy::core::utils::UNKNOWN_EXPORT_PLACEHOLDER;
use stackql_deploy::utils::pgwire::PgwireLite;

pub use fake_cloud::FakeCloud;
pub use mock_server::MockServer;

/// Route stackql-deploy's `log` output through the test harness so a failing
/// test shows the same trail an operator would see. Honour `RUST_LOG` when
/// set; default to `info`.
pub fn init_test_logging() {
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default());
    if std::env::var_os("RUST_LOG").is_none() {
        builder.filter_level(log::LevelFilter::Info);
    }
    let _ = builder.is_test(true).try_init();
}

/// Placeholder written into exports that teardown could not collect.
pub const UNKNOWN: &str = UNKNOWN_EXPORT_PLACEHOLDER;

/// Absolute path of `tests/fixtures/<name>`.
pub fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// A fixture stack copied into a temporary directory.
pub struct TestStack {
    /// Kept alive so the directory is removed when the test ends.
    _tempdir: tempfile::TempDir,
    pub dir: PathBuf,
}

impl TestStack {
    /// Copy `tests/fixtures/<fixture>` into a temp dir using its default
    /// `stackql_manifest.yml`.
    pub fn new(fixture: &str) -> Self {
        Self::with_manifest(fixture, "stackql_manifest.yml")
    }

    /// Copy `tests/fixtures/<fixture>` into a temp dir and install
    /// `<fixture>/<manifest>` as the stack's `stackql_manifest.yml`. Lets one
    /// set of `resources/*.iql` files be shared by several manifest variants.
    pub fn with_manifest(fixture: &str, manifest: &str) -> Self {
        init_test_logging();
        let source = fixture_dir(fixture);
        assert!(
            source.is_dir(),
            "fixture directory not found: {}",
            source.display()
        );
        let tempdir = tempfile::tempdir().expect("create temp dir");
        let dir = tempdir.path().join(fixture);
        copy_dir(&source, &dir);

        let chosen = source.join(manifest);
        assert!(
            chosen.is_file(),
            "manifest variant not found: {}",
            chosen.display()
        );
        fs::copy(&chosen, dir.join("stackql_manifest.yml")).expect("install manifest");

        TestStack {
            _tempdir: tempdir,
            dir,
        }
    }

    /// Build a `CommandRunner` for this stack connected to `server`.
    pub fn runner(&self, server: &MockServer, stack_env: &str) -> CommandRunner {
        // `catch_error_and_exit` tries to stop a *local* stackql server on
        // the globally configured host/port before exiting. Point the
        // globals at a non-local host so that path can never touch a real
        // server on this machine. `init_globals` is first-write-wins, which
        // is fine: every test in a binary uses the same inert host.
        stackql_deploy::globals::init_globals("mock-stackql.invalid".to_string(), server.port());

        let client = PgwireLite::new("127.0.0.1", server.port(), false, "default")
            .expect("connect to mock server");
        let env_file = self.dir.join(".env");
        CommandRunner::new(
            client,
            self.dir.to_str().expect("utf-8 stack dir"),
            stack_env,
            env_file.to_str().expect("utf-8 env file"),
            &[],
        )
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read fixture dir") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}

/// Value of an exported variable in the runner's global context.
pub fn export_value(runner: &CommandRunner, name: &str) -> Option<String> {
    runner.global_context.get(name).cloned()
}

/// Assert that no statement sent to the mock server interpolated the
/// `<unknown>` placeholder. This is the core regression check for issue #56:
/// such a statement turns a missing export into a bogus hostname or
/// identifier and a fatal provider error.
pub fn assert_no_unknown_placeholder_sent(server: &MockServer) {
    let offenders = server.queries_containing(UNKNOWN);
    assert!(
        offenders.is_empty(),
        "statements containing {} were sent to the server:\n{}",
        UNKNOWN,
        offenders.join("\n---\n")
    );
}
