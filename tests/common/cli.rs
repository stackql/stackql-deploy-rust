//! Drive the real `stackql-deploy` binary against a live stack.
//!
//! Used by the `#[ignore]`d live tests in `tests/live.rs`. The binary is
//! pointed at a stack directory under `tests/live_stacks/` (which it only
//! reads) and runs with `target/live/` as its working directory, so the
//! stackql server's provider cache (`.stackql/`), `.stackql-deploy-exports`
//! and `stackql.log` land in a gitignored place that persists between runs.
//! Sharing one working directory means the provider documents are pulled
//! once, not once per stack.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A live stack and the working directory the binary runs in.
pub struct LiveStack {
    /// The stack directory in the source tree (read only).
    pub dir: PathBuf,
    /// Working directory for the binary and the stackql server.
    pub work_dir: PathBuf,
    pub name: String,
    port: u16,
    /// `-e KEY=VALUE` inputs; interior mutability lets a test change an input
    /// between runs while a [`TeardownGuard`] holds a shared borrow.
    inputs: RefCell<Vec<(String, String)>>,
}

/// Result of one `stackql-deploy` invocation.
pub struct Outcome {
    pub phase: String,
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl LiveStack {
    /// Bind to `tests/live_stacks/<name>`. `port` is the stackql server port
    /// for this stack; give every stack its own so runs never collide with
    /// each other or with a developer's default server.
    pub fn new(name: &str, port: u16) -> Self {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = manifest_dir.join("tests").join("live_stacks").join(name);
        assert!(dir.is_dir(), "live stack not found: {}", dir.display());
        let work_dir = manifest_dir.join("target").join("live");
        fs::create_dir_all(&work_dir).expect("create target/live");
        // Leftovers from an earlier run must not satisfy this run's asserts.
        let _ = fs::remove_file(work_dir.join(".stackql-deploy-exports"));
        LiveStack {
            dir,
            work_dir,
            name: name.to_string(),
            port,
            inputs: RefCell::new(Vec::new()),
        }
    }

    /// Set (or replace) a manifest input passed as `-e KEY=VALUE`.
    pub fn input(&self, key: &str, value: &str) -> &Self {
        let mut inputs = self.inputs.borrow_mut();
        inputs.retain(|(k, _)| k != key);
        inputs.push((key.to_string(), value.to_string()));
        self
    }

    /// Run `stackql-deploy <command> <stack dir> <stack_env> [extra...]`.
    pub fn run(&self, command: &str, stack_env: &str, extra: &[&str]) -> Outcome {
        let phase = format!("{} {} {}", command, stack_env, extra.join(" "))
            .trim()
            .to_string();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_stackql-deploy"));
        cmd.current_dir(&self.work_dir)
            .arg("--port")
            .arg(self.port.to_string())
            .arg(command)
            .arg(&self.dir)
            .arg(stack_env);
        for (k, v) in self.inputs.borrow().iter() {
            cmd.arg("-e").arg(format!("{}={}", k, v));
        }
        cmd.args(extra);

        eprintln!(
            "
[{}] $ stackql-deploy {}",
            self.name, phase
        );
        let output = cmd.output().expect("spawn stackql-deploy");
        let outcome = Outcome {
            phase,
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        };
        outcome.print();
        outcome
    }

    /// Path of the sourceable exports file the binary writes in its cwd.
    pub fn exports_file(&self) -> PathBuf {
        self.work_dir.join(".stackql-deploy-exports")
    }

    /// A scratch path in the working directory (for `--output-file`), with
    /// any previous run's file removed.
    pub fn scratch(&self, file_name: &str) -> PathBuf {
        let path = self.work_dir.join(format!("{}-{}", self.name, file_name));
        let _ = fs::remove_file(&path);
        path
    }
}

impl Outcome {
    /// stdout followed by stderr (the logger writes to stderr).
    pub fn log(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.log().contains(needle)
    }

    pub fn count(&self, needle: &str) -> usize {
        self.log().matches(needle).count()
    }

    pub fn assert_success(&self) -> &Self {
        assert_eq!(
            self.status,
            Some(0),
            "expected `{}` to succeed, exit status {:?}",
            self.phase,
            self.status
        );
        self
    }

    pub fn assert_failure(&self) -> &Self {
        assert_ne!(
            self.status,
            Some(0),
            "expected `{}` to fail, but it exited 0",
            self.phase
        );
        self
    }

    pub fn assert_contains(&self, needle: &str) -> &Self {
        assert!(
            self.contains(needle),
            "expected `{}` output to contain {:?}",
            self.phase,
            needle
        );
        self
    }

    pub fn assert_not_contains(&self, needle: &str) -> &Self {
        assert!(
            !self.contains(needle),
            "expected `{}` output NOT to contain {:?}",
            self.phase,
            needle
        );
        self
    }

    /// Echo the run's output, folded into a group on GitHub Actions.
    fn print(&self) {
        let on_actions = std::env::var_os("GITHUB_ACTIONS").is_some();
        if on_actions {
            eprintln!("::group::{} (exit {:?})", self.phase, self.status);
        }
        eprintln!("{}", self.log());
        if on_actions {
            eprintln!("::endgroup::");
        }
        eprintln!("[{}] exit status: {:?}", self.phase, self.status);
    }
}

/// Runs `teardown --on-failure ignore` when dropped unless disarmed, so a
/// failing test still removes what it created.
pub struct TeardownGuard<'a> {
    stack: &'a LiveStack,
    stack_env: String,
    armed: bool,
}

impl<'a> TeardownGuard<'a> {
    pub fn new(stack: &'a LiveStack, stack_env: &str) -> Self {
        TeardownGuard {
            stack,
            stack_env: stack_env.to_string(),
            armed: true,
        }
    }

    /// Call once the test has torn the stack down itself.
    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TeardownGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            eprintln!(
                "[{}] test did not reach its own teardown, cleaning up",
                self.stack.name
            );
            let _ = self
                .stack
                .run("teardown", &self.stack_env, &["--on-failure", "ignore"]);
        }
    }
}

/// Read a `--output-file` JSON document into a map of string values.
pub fn read_exports_json(path: &Path) -> serde_json::Map<String, serde_json::Value> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read exports file {}: {}", path.display(), e));
    serde_json::from_str::<serde_json::Value>(&text)
        .expect("exports file is valid JSON")
        .as_object()
        .expect("exports file is a JSON object")
        .clone()
}

/// Value of `key` in an exports JSON map as a string.
pub fn export_str(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> String {
    match map.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => panic!(
            "export {:?} missing from {:?}",
            key,
            map.keys().collect::<Vec<_>>()
        ),
    }
}

/// Environment variable or a default.
pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Fail fast with a clear message when a credential is missing.
pub fn require_env(key: &str, purpose: &str) {
    if std::env::var_os(key).is_none() {
        panic!(
            "environment variable {} is required for {}; see tests/README.md",
            key, purpose
        );
    }
}

/// Unique suffix for resource names: the CI run id, or a local timestamp.
pub fn run_id() -> String {
    std::env::var("STACKQL_DEPLOY_LIVE_RUN_ID").unwrap_or_else(|_| {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("local-{}", secs)
    })
}
