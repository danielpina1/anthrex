//! M8b.7's decider-call fixtures, shared by `decider_call.rs` and `decider_call_edges.rs`.
//! Each test's program is a wrapper script that points `fake-agent` at the test's own
//! decider directory and `exec`s it through a symlink under the test's temporary
//! directory, so no test changes its own environment and the process can be found by a
//! marker that is in no test's command line.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use daemon::decider::{BlockedReasonInput, DeciderContext, DeciderRequest, TriageInput};
use daemon::manager::ManagerConfig;
use proto::{DeciderMode, DeciderSource};
use serde_json::Value;
use serde_json::json;

use super::{fake_agent_bin, tempdir};

/// `deciders.timeout_secs` in every test.
pub const TIMEOUT_SECS: u64 = 5;
/// `HeadlessHandle::kill`'s grace in the call.
pub const KILL_GRACE: Duration = Duration::from_secs(2);
/// A spawn and a few thread starts, generously (docs/timing-budgets.md, M8b.7).
pub const SPAWN_SLACK: Duration = Duration::from_secs(5);
/// How long a killed decider may take to disappear from the process table.
pub const GONE_WITHIN: Duration = Duration::from_secs(5);
/// Set on the child process of `the_decider_sees_no_api_credentials` only.
pub const CHILD_DIR: &str = "ANTHREX_DECIDER_CALL_CHILD_DIR";

pub struct Fixture {
    pub dir: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let fx = Fixture { dir: tempdir() };
        std::fs::create_dir_all(fx.deciders()).unwrap();
        let bin = fx.dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(fake_agent_bin(), bin.join("fake-agent")).unwrap();
        let script = format!(
            "#!/bin/sh\nexport FAKE_AGENT_DECIDER_DIR='{}'\nexport FAKE_AGENT_ENV_FILE='{}'\nexec '{}' \"$@\"\n",
            fx.deciders().display(),
            fx.env_file().display(),
            bin.join("fake-agent").display()
        );
        write_executable(&fx.program(), &script);
        fx
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn program(&self) -> PathBuf {
        self.root().join("decider.sh")
    }

    pub fn deciders(&self) -> PathBuf {
        self.root().join("deciders")
    }

    pub fn env_file(&self) -> PathBuf {
        self.root().join("env-keys.txt")
    }

    /// A string found in the decider's command line (the symlink's path) and in no test's.
    pub fn marker(&self) -> String {
        self.root()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    pub fn script(&self, kind: &str, n: u32, value: Value) {
        let path = self.deciders().join(format!("{kind}-{n}.json"));
        std::fs::write(path, value.to_string()).unwrap();
    }

    pub fn calls(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.deciders().join("calls.jsonl")).unwrap_or_default();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    pub fn context(&self, mode: DeciderMode) -> DeciderContext {
        let program = self.program().to_string_lossy().into_owned();
        context_with(self.root(), mode, move |key| {
            (key == "ANTHREX_DECIDER_BIN").then(|| program.clone())
        })
    }
}

/// Where every test context's `claude` and `codex` commands point: a path that does not
/// exist, so no test, and no mutation of program selection, can reach a real agent
/// binary on `PATH`.
pub const NO_CLAUDE_BIN: &str = "/nonexistent/anthrex-test/claude";
pub const NO_CODEX_BIN: &str = "/nonexistent/anthrex-test/codex";

/// A context from `var` (for `ANTHREX_DECIDER_BIN`), whatever it says about the runtime
/// commands: `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` are always [`NO_CLAUDE_BIN`]
/// and [`NO_CODEX_BIN`].
pub fn context_with(
    root: &Path,
    mode: DeciderMode,
    var: impl Fn(&str) -> Option<String>,
) -> DeciderContext {
    let var = move |key: &str| match key {
        "ANTHREX_CLAUDE_BIN" => Some(NO_CLAUDE_BIN.to_string()),
        "ANTHREX_CODEX_BIN" => Some(NO_CODEX_BIN.to_string()),
        _ => var(key),
    };
    let mut cfg = config::Orchestrator::default();
    cfg.deciders.mode = mode;
    cfg.deciders.timeout_secs = TIMEOUT_SECS;
    let manager = ManagerConfig::from_vars(
        root.join("unused.sock"),
        "/bin/sh".into(),
        root.join("unused-anthrex"),
        var,
        &config::Config::default().runtimes,
    );
    DeciderContext::new(&cfg, &manager, &root.join("data"))
}

pub fn write_executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn blocked(reason: &str) -> DeciderRequest {
    DeciderRequest::BlockedReason(BlockedReasonInput {
        task_id: "t1".into(),
        title: "Add a retry to the fetch helper".into(),
        reason: reason.into(),
    })
}

pub fn triage() -> DeciderRequest {
    DeciderRequest::Triage(TriageInput {
        goal: "fix the typo in the README".into(),
        profile_summary: String::new(),
        report_summary: None,
        report_files: vec![],
        files: vec!["README.md".into()],
        files_total: 1,
        planner_task_cap: 12,
    })
}

pub fn answer() -> Value {
    json!({"kind": "environment", "reason": "the linker is missing"})
}

pub fn assert_fallback(decision: &daemon::decider::Decision, reason: &str) {
    assert_eq!(decision.source, DeciderSource::Fallback, "{decision:?}");
    assert_eq!(decision.fallback_reason.as_deref(), Some(reason));
    assert_eq!(
        decision.answer,
        daemon::decider::fallback::fallback(&blocked("x")),
        "{decision:?}"
    );
}

/// The pids `pgrep -f` finds for `marker`, which is in no test's command line (and pgrep
/// never lists itself). Looks only; signals nothing.
pub fn pgrep(marker: &str) -> String {
    let found = Command::new("pgrep").args(["-f", marker]).output().unwrap();
    String::from_utf8_lossy(&found.stdout).trim().to_string()
}

/// How many processes `pgrep -f marker` lists.
pub fn running(marker: &str) -> usize {
    pgrep(marker).lines().count()
}

/// Waits (a deadline loop, looking only) until `pgrep -f marker` lists at least `n`
/// processes; `false` when `within` passes first.
pub fn saw_running(marker: &str, n: usize, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if running(marker) >= n {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Fails unless `pgrep -f marker` lists nothing within [`GONE_WITHIN`]. Looks only; the
/// test signals nothing.
pub fn assert_gone(marker: &str) {
    let deadline = Instant::now() + GONE_WITHIN;
    loop {
        let pids = pgrep(marker);
        if pids.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "decider processes left after {GONE_WITHIN:?}: {}",
            pids.replace('\n', " ")
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
