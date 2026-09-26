//! M8b.7: the decider call (decision 16) against real `fake-agent` processes. Each
//! test's program is a wrapper script that points `fake-agent` at the test's own
//! decider directory and `exec`s it through a symlink under the test's temporary
//! directory, so no test changes its own environment and the process can be found by a
//! marker that is in no test's command line.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use daemon::decider::call::{ANSWER_MAX_BYTES, decide};
use daemon::decider::{
    BlockKind, BlockedReasonInput, DeciderAnswer, DeciderContext, DeciderKind, DeciderRequest,
    TriageInput,
};
use daemon::manager::ManagerConfig;
use proto::{DeciderMode, DeciderSource, TokenUsage};
use serde_json::{Value, json};
use support::{fake_agent_bin, runtime, tempdir};

/// `deciders.timeout_secs` in every test.
const TIMEOUT_SECS: u64 = 5;
/// `HeadlessHandle::kill`'s grace in the call.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// A spawn and a few thread starts, generously (docs/timing-budgets.md, M8b.7).
const SPAWN_SLACK: Duration = Duration::from_secs(5);
/// How long a killed decider may take to disappear from the process table.
const GONE_WITHIN: Duration = Duration::from_secs(5);
/// Set on the child process of `the_decider_sees_no_api_credentials` only.
const CHILD_DIR: &str = "ANTHREX_DECIDER_CALL_CHILD_DIR";

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
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

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn program(&self) -> PathBuf {
        self.root().join("decider.sh")
    }

    fn deciders(&self) -> PathBuf {
        self.root().join("deciders")
    }

    fn env_file(&self) -> PathBuf {
        self.root().join("env-keys.txt")
    }

    /// A string found in the decider's command line (the symlink's path) and in no test's.
    fn marker(&self) -> String {
        self.root()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn script(&self, kind: &str, n: u32, value: Value) {
        let path = self.deciders().join(format!("{kind}-{n}.json"));
        std::fs::write(path, value.to_string()).unwrap();
    }

    fn calls(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.deciders().join("calls.jsonl")).unwrap_or_default();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn context(&self, mode: DeciderMode) -> DeciderContext {
        let program = self.program().to_string_lossy().into_owned();
        context_with(self.root(), mode, move |key| {
            (key == "ANTHREX_DECIDER_BIN").then(|| program.clone())
        })
    }
}

fn context_with(
    root: &Path,
    mode: DeciderMode,
    var: impl Fn(&str) -> Option<String>,
) -> DeciderContext {
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

fn write_executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn blocked(reason: &str) -> DeciderRequest {
    DeciderRequest::BlockedReason(BlockedReasonInput {
        task_id: "t1".into(),
        title: "Add a retry to the fetch helper".into(),
        reason: reason.into(),
    })
}

fn triage() -> DeciderRequest {
    DeciderRequest::Triage(TriageInput {
        goal: "fix the typo in the README".into(),
        profile_summary: String::new(),
        report_summary: None,
        report_files: vec![],
        files: vec!["README.md".into()],
        files_total: 1,
    })
}

fn answer() -> Value {
    json!({"kind": "environment", "reason": "the linker is missing"})
}

fn assert_fallback(decision: &daemon::decider::Decision, reason: &str) {
    assert_eq!(decision.source, DeciderSource::Fallback, "{decision:?}");
    assert_eq!(decision.fallback_reason.as_deref(), Some(reason));
    assert_eq!(
        decision.answer,
        daemon::decider::fallback::fallback(&blocked("x")),
        "{decision:?}"
    );
}

#[test]
fn claude_mode_returns_the_scripted_answer_and_usage() {
    let fx = Fixture::new();
    let usage = json!({"input": 10, "output": 357, "cache_read": 0, "cache_write": 17688});
    fx.script(
        "blocked_reason",
        1,
        json!({"answer": answer(), "usage": usage}),
    );
    let decision = runtime().block_on(decide(&fx.context(DeciderMode::Claude), &blocked("ld")));
    assert_eq!(decision.kind, DeciderKind::BlockedReason);
    assert_eq!(decision.source, DeciderSource::Decider, "{decision:?}");
    assert_eq!(decision.fallback_reason, None);
    assert_eq!(
        decision.answer,
        DeciderAnswer::BlockedReason {
            kind: BlockKind::Environment,
            reason: "the linker is missing".into()
        }
    );
    assert_eq!(
        decision.usage,
        Some(TokenUsage {
            input: 10,
            output: 357,
            cache_read: 0,
            cache_write: 17688
        })
    );
    assert!(decision.secs < TIMEOUT_SECS, "{decision:?}");
}

#[test]
fn codex_mode_returns_the_scripted_answer() {
    let fx = Fixture::new();
    fx.script("blocked_reason", 1, json!({"answer": answer()}));
    let decision = runtime().block_on(decide(&fx.context(DeciderMode::Codex), &blocked("ld")));
    assert_eq!(decision.source, DeciderSource::Decider, "{decision:?}");
    assert_eq!(
        decision.answer,
        DeciderAnswer::BlockedReason {
            kind: BlockKind::Environment,
            reason: "the linker is missing".into()
        }
    );
}

#[test]
fn the_prompt_goes_on_stdin_for_claude_and_last_for_codex() {
    let fx = Fixture::new();
    fx.script("blocked_reason", 1, json!({"answer": answer()}));
    fx.script("blocked_reason", 2, json!({"answer": answer()}));
    let request = blocked("cargo test fails: `cc` is not installed");
    let prompt = daemon::decider::prompt::render(&request);
    let rt = runtime();
    let claude = rt.block_on(decide(&fx.context(DeciderMode::Claude), &request));
    let codex = rt.block_on(decide(&fx.context(DeciderMode::Codex), &request));
    assert_eq!(claude.source, DeciderSource::Decider, "{claude:?}");
    assert_eq!(codex.source, DeciderSource::Decider, "{codex:?}");

    let calls = fx.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    for call in &calls {
        assert_eq!(call["kind"], "blocked_reason");
        assert_eq!(call["prompt"], prompt.as_str());
    }
    // Claude: the prompt arrived on stdin, never in the argv.
    let claude_argv: Vec<&str> = calls[0]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(claude_argv[0], "-p");
    assert!(!claude_argv.contains(&prompt.as_str()), "{claude_argv:?}");
    assert!(claude_argv.contains(&"--json-schema"), "{claude_argv:?}");
    // Codex: `--` and then the prompt, last; the schema file it names exists and holds
    // the kind's schema.
    let codex_argv: Vec<&str> = calls[1]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(codex_argv[0], "exec");
    assert_eq!(codex_argv[codex_argv.len() - 2], "--");
    assert_eq!(codex_argv[codex_argv.len() - 1], prompt);
    let at = codex_argv
        .iter()
        .position(|a| *a == "--output-schema")
        .unwrap();
    let schema_file = Path::new(codex_argv[at + 1]);
    assert!(
        schema_file.starts_with(fx.root().join("data/deciders/schemas")),
        "{schema_file:?}"
    );
    let written: Value = serde_json::from_slice(&std::fs::read(schema_file).unwrap()).unwrap();
    assert_eq!(
        written,
        daemon::decider::schema::schema(DeciderKind::BlockedReason)
    );
}

#[test]
fn decider_bin_wins_over_the_runtime_command() {
    let fx = Fixture::new();
    let program = fx.program().to_string_lossy().into_owned();
    let both = |key: &str| match key {
        "ANTHREX_DECIDER_BIN" => Some(program.clone()),
        "ANTHREX_CLAUDE_BIN" => Some("/nonexistent/claude".to_string()),
        "ANTHREX_CODEX_BIN" => Some("/nonexistent/codex".to_string()),
        _ => None,
    };
    let manager = ManagerConfig::from_vars(
        "/tmp/unused.sock".into(),
        "/bin/sh".into(),
        "/tmp/unused".into(),
        both,
        &config::Config::default().runtimes,
    );
    assert_eq!(manager.decider_bin.as_deref(), Some(program.as_str()));
    let ctx = context_with(fx.root(), DeciderMode::Claude, both);
    assert_eq!(ctx.program, fx.program().into_os_string());
    // It is the program that runs: the runtime command does not exist.
    fx.script("blocked_reason", 1, json!({"answer": answer()}));
    let decision = runtime().block_on(decide(&ctx, &blocked("ld")));
    assert_eq!(decision.source, DeciderSource::Decider, "{decision:?}");

    // Without it (or with it empty) the mode's runtime command is the program.
    let without = |key: &str| match key {
        "ANTHREX_DECIDER_BIN" => Some(String::new()),
        "ANTHREX_CLAUDE_BIN" => Some("/nonexistent/claude".to_string()),
        "ANTHREX_CODEX_BIN" => Some("/nonexistent/codex".to_string()),
        _ => None,
    };
    let claude = context_with(fx.root(), DeciderMode::Claude, without);
    assert_eq!(claude.program, "/nonexistent/claude");
    let codex = context_with(fx.root(), DeciderMode::Codex, without);
    assert_eq!(codex.program, "/nonexistent/codex");
}

#[test]
fn mode_off_never_spawns() {
    let fx = Fixture::new();
    let marker = fx.root().join("spawned.txt");
    let program = fx.root().join("marker.sh");
    write_executable(
        &program,
        &format!("#!/bin/sh\necho spawned >> '{}'\n", marker.display()),
    );
    let program_text = program.to_string_lossy().into_owned();
    let var = |key: &str| (key == "ANTHREX_DECIDER_BIN").then(|| program_text.clone());
    let rt = runtime();
    let off = rt.block_on(decide(
        &context_with(fx.root(), DeciderMode::Off, var),
        &blocked("x"),
    ));
    assert_fallback(&off, "deciders are off");
    assert_eq!(off.usage, None);
    // The control: the same program with the mode on runs, exits 0 without answering,
    // and appends one line. Had the call above spawned it too, there would be two.
    let on = rt.block_on(decide(
        &context_with(fx.root(), DeciderMode::Claude, var),
        &blocked("x"),
    ));
    assert_fallback(&on, "the decider exited before answering (code 0)");
    let lines = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(lines, "spawned\n");
}

#[test]
fn a_hanging_decider_falls_back_at_its_timeout_and_leaves_no_process() {
    let fx = Fixture::new();
    fx.script("blocked_reason", 1, json!({"hang": true}));
    fx.script("blocked_reason", 2, json!({"hang": true}));
    let marker = fx.marker();
    let request = blocked(&format!("hang {marker}"));
    let (claude_ctx, codex_ctx) = (
        fx.context(DeciderMode::Claude),
        fx.context(DeciderMode::Codex),
    );
    let started = Instant::now();
    // The control: while they hang, the marker does find them.
    let look = marker.clone();
    let (claude, codex, seen) = runtime().block_on(async {
        let seen = tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + Duration::from_secs(TIMEOUT_SECS - 1);
            while Instant::now() < deadline {
                if !pgrep(&look).is_empty() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        });
        let (claude, codex, seen) = tokio::join!(
            decide(&claude_ctx, &request),
            decide(&codex_ctx, &request),
            seen
        );
        (claude, codex, seen.unwrap())
    });
    let elapsed = started.elapsed();
    assert!(seen, "the marker never matched a running decider");
    let timeout = Duration::from_secs(TIMEOUT_SECS);
    assert!(elapsed >= timeout, "{elapsed:?}");
    assert!(elapsed <= timeout + KILL_GRACE + SPAWN_SLACK, "{elapsed:?}");
    for decision in [&claude, &codex] {
        assert_fallback(decision, "the decider timed out after 5 s");
    }
    assert_eq!(fx.calls().len(), 2, "both deciders ran");

    // Only look: `pgrep -f` with a marker in the deciders' command line (the program's
    // path) and in no test's. The test signals nothing.
    let deadline = Instant::now() + GONE_WITHIN;
    loop {
        let pids = pgrep(&marker);
        if pids.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "decider processes left after {GONE_WITHIN:?}: {pids}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The pids `pgrep -f` finds for `marker`, which is in no test's command line (and pgrep
/// never lists itself). Looks only; signals nothing.
fn pgrep(marker: &str) -> String {
    let found = Command::new("pgrep").args(["-f", marker]).output().unwrap();
    String::from_utf8_lossy(&found.stdout).trim().to_string()
}

#[test]
fn garbage_empty_and_oversized_answers_fall_back() {
    let fx = Fixture::new();
    let ctx = fx.context(DeciderMode::Claude);
    let rt = runtime();

    fx.script("blocked_reason", 1, json!({"text": "not json"}));
    let decision = rt.block_on(decide(&ctx, &blocked("x")));
    let error = serde_json::from_str::<Value>("not json").unwrap_err();
    assert_fallback(
        &decision,
        &format!("the decider's answer is not JSON: {error}"),
    );

    fx.script("triage", 1, json!({"text": "{}"}));
    let decision = rt.block_on(decide(&ctx, &triage()));
    assert_eq!(decision.source, DeciderSource::Fallback, "{decision:?}");
    assert_eq!(
        decision.fallback_reason.as_deref(),
        Some("the decider's answer does not match the schema: kinds: missing")
    );
    assert_eq!(
        decision.answer,
        daemon::decider::fallback::fallback(&triage())
    );

    // A JSON string of 300 KiB is valid JSON (and would be a schema error); cut to
    // ANSWER_MAX_BYTES it ends inside the string, so the reason is the JSON error.
    let big = format!("\"{}\"", "a".repeat(300 * 1024));
    fx.script("blocked_reason", 2, json!({"text": big}));
    let decision = rt.block_on(decide(&ctx, &blocked("x")));
    let error = serde_json::from_str::<Value>(&big[..ANSWER_MAX_BYTES]).unwrap_err();
    assert!(error.is_eof(), "{error}");
    assert_fallback(
        &decision,
        &format!("the decider's answer is not JSON: {error}"),
    );
}

#[test]
fn a_failed_turn_and_an_early_exit_fall_back() {
    let fx = Fixture::new();
    let rt = runtime();
    let claude = fx.context(DeciderMode::Claude);
    let codex = fx.context(DeciderMode::Codex);

    fx.script("blocked_reason", 1, json!({"fail_turn": "overloaded"}));
    let decision = rt.block_on(decide(&claude, &blocked("x")));
    assert_fallback(
        &decision,
        "the decider's turn failed: API Error: overloaded",
    );

    fx.script("blocked_reason", 2, json!({"fail_turn": "overloaded"}));
    let decision = rt.block_on(decide(&codex, &blocked("x")));
    assert_fallback(&decision, "the decider's turn failed: overloaded");

    fx.script("blocked_reason", 3, json!({"exit": 3}));
    let decision = rt.block_on(decide(&claude, &blocked("x")));
    assert_fallback(&decision, "the decider exited before answering (code 3)");

    fx.script("blocked_reason", 4, json!({"exit": 3}));
    let decision = rt.block_on(decide(&codex, &blocked("x")));
    assert_fallback(&decision, "the decider exited before answering (code 3)");
}

#[test]
fn a_missing_program_falls_back_with_could_not_start() {
    let fx = Fixture::new();
    let missing = fx.root().join("no-such-decider");
    let program = missing.to_string_lossy().into_owned();
    let ctx = context_with(fx.root(), DeciderMode::Claude, |key| {
        (key == "ANTHREX_DECIDER_BIN").then(|| program.clone())
    });
    let decision = runtime().block_on(decide(&ctx, &blocked("x")));
    assert_fallback(
        &decision,
        &format!(
            "the decider could not start: could not start {}: No such file or directory (os error 2)",
            missing.display()
        ),
    );
}

#[test]
fn the_decider_sees_no_api_credentials() {
    let fx = Fixture::new();
    fx.script("blocked_reason", 1, json!({"answer": answer()}));
    // The call runs in a child copy of this test binary whose command, and only that
    // command, carries the credentials: this process's environment is never changed.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "decider_call_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_DIR, fx.root())
        .env("ANTHROPIC_API_KEY", "sk-ant-not-a-real-key")
        .env("OPENAI_API_KEY", "sk-not-a-real-key")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let recorded = std::fs::read_to_string(fx.env_file()).unwrap();
    let keys: Vec<&str> = recorded.lines().collect();
    // The recording works (the child's own marker variable was inherited) ...
    assert!(keys.contains(&CHILD_DIR), "{keys:?}");
    // ... and no API credential reached the decider.
    for key in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"] {
        assert!(!keys.contains(&key), "{key} reached the decider: {keys:?}");
    }
}

/// The child half of `the_decider_sees_no_api_credentials`; does nothing unless that test
/// started it.
#[test]
#[ignore = "run by the_decider_sees_no_api_credentials"]
fn decider_call_child() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else {
        return;
    };
    assert!(std::env::var_os("ANTHROPIC_API_KEY").is_some());
    let root = PathBuf::from(dir);
    let program = root.join("decider.sh").to_string_lossy().into_owned();
    let ctx = context_with(&root, DeciderMode::Claude, |key| {
        (key == "ANTHREX_DECIDER_BIN").then(|| program.clone())
    });
    let decision = runtime().block_on(decide(&ctx, &blocked("x")));
    assert_eq!(decision.source, DeciderSource::Decider, "{decision:?}");
}
