//! Milestone 9.5 decision 40 (task M9.5.5b, review ruling I11): the footer, pure, and
//! its scan on every manager tick against a real manager and real PTY windows whose
//! `codex` is a stand-in shell script (never an agent) drawing what a control file
//! holds. Each test kills only the windows it made.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{AgentRole, Effort, HookSource, RunRef, Runtime, Status, WindowSpec};
use serde_json::json;

use super::*;
use crate::headless::McpTarget;
use crate::launch::LaunchGate;
use crate::launch::role::RoleLaunch;
use crate::manager::{ManagerConfig, WindowManager};

const FIXTURE: &str = include_str!("../tests/fixtures/screens/codex-question-footer.txt");

/// The fixture's screen: every line but its `# ` notes.
fn fixture_screen() -> String {
    FIXTURE
        .lines()
        .filter(|l| !l.starts_with("# "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_fixtures_footer_matches() {
    let screen = fixture_screen();
    let rows = footer_rows(&screen);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(codex_question_footer(&rows));
    assert!(screen_asks(&screen));
}

#[test]
fn the_footer_pattern() {
    assert!(codex_question_footer(&["? 2 questions"]));
    assert!(codex_question_footer(&[
        "  Queued · ?  12  question · shift+← to answer"
    ]));
    assert!(!codex_question_footer(&["questions are welcome"]));
    assert!(!codex_question_footer(&["? 1 questionnaire"]));
    assert!(!codex_question_footer(&["?1 question", "? question"]));
    assert!(!codex_question_footer(&[]));
}

/// Only the last three non-empty rows count: a question further up is the transcript,
/// not the footer.
#[test]
fn only_the_last_three_rows_are_the_footer() {
    let screen = "? 1 question\n\nwork\n\n  \nmore\nstill more\nthe prompt\n\n";
    assert_eq!(footer_rows(screen), ["more", "still more", "the prompt"]);
    assert!(!screen_asks(screen));
}

#[test]
fn a_codex_question_is_attention_from_any_status_but_exited() {
    use crate::status::{StatusContext, StatusEvent, next};
    for status in [
        Status::Starting,
        Status::Working,
        Status::Idle,
        Status::Done,
        Status::Attention,
    ] {
        let ctx = StatusContext::default();
        assert_eq!(
            next(status, StatusEvent::CodexQuestion, Runtime::Codex, ctx),
            Status::Attention
        );
    }
    let ctx = StatusContext::default();
    assert_eq!(
        next(
            Status::Exited,
            StatusEvent::CodexQuestion,
            Runtime::Codex,
            ctx
        ),
        Status::Exited
    );
}

/// Slack on top of each derived bound below (a process start, a few 20 ms polls).
const SLACK: Duration = Duration::from_secs(5);

/// `codex` as a stand-in: `--version` answered at once (`docs/timing-budgets.md`), no
/// echo, a `Ready` title (its signal, as a Codex session with no hooks gives), then the
/// screen redrawn each time the control file in its directory changes: the one its
/// environment's `ANTHREX_TEST_CTL` names (the role's `env`), else `plain.ctl`.
fn stand_in(dir: &Path) -> String {
    let codex = dir.join("codex");
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo codex-cli 0.0.0; exit 0; }}\nstty -echo 2>/dev/null\nprintf '\\033]0;Ready\\007'\nctl={dir}/\"${{ANTHREX_TEST_CTL:-plain.ctl}}\"\n\
         last=''\nwhile :; do\n  c=$(cat \"$ctl\" 2>/dev/null)\n  if [ \"$c\" != \"$last\" ]; then\n    \
         printf '\\033[2J\\033[H%s' \"$c\"\n    last=$c\n  fi\n  sleep 0.05\ndone\n",
        dir = dir.display()
    );
    std::fs::write(&codex, script).unwrap();
    std::fs::set_permissions(&codex, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    codex.to_str().unwrap().into()
}

struct Rig {
    dir: tempfile::TempDir,
    manager: Arc<WindowManager>,
}

fn rig() -> Rig {
    let dir = tempfile::Builder::new()
        .prefix("ax-codex-q-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = stand_in(dir.path());
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, mut pumped) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = pumped.recv().await {
            pump.handle_event(id, event);
        }
    });
    Rig { dir, manager }
}

/// Run `r1`'s orchestrator role, as the engine launches it; `ctl` names the control
/// file its stand-in draws.
fn role(ctl: &str) -> RoleLaunch {
    RoleLaunch {
        run_ref: RunRef {
            run_id: "r1".into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 1,
            lane: None,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: "r1".into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
        },
        instructions: "the orchestrator contract".into(),
        effort: Effort::High,
        claude_allowed_tools: Vec::new(),
        claude_disallowed_tools: Vec::new(),
        env: vec![("ANTHREX_TEST_CTL".into(), ctl.into())],
        remove_env: Vec::new(),
    }
}

fn spec(dir: &Path, name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.into()),
        runtime: Runtime::Codex,
        cwd: dir.to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

impl Rig {
    fn draw(&self, ctl: &str, screen: &str) {
        let path = self.dir.path().join(ctl);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, screen).unwrap();
        std::fs::rename(tmp, path).unwrap();
    }

    fn status(&self, window: u32) -> (Status, bool) {
        let w = self
            .manager
            .list()
            .into_iter()
            .find(|w| w.id == window)
            .unwrap();
        (w.status, w.signals_seen)
    }

    /// Waits until `window` has its `Ready` title and is quiet (`Idle` or `Done`);
    /// that status.
    async fn signalled(&self, window: u32) -> Status {
        let deadline = Instant::now() + SLACK;
        loop {
            if let (status @ (Status::Idle | Status::Done), true) = self.status(window) {
                return status;
            }
            assert!(Instant::now() < deadline, "{:?}", self.status(window));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits until `window`'s screen shows the footer (`asks`) or does not.
    async fn shown(&self, window: u32, asks: bool) {
        let deadline = Instant::now() + SLACK;
        loop {
            let (bytes, _, _) = self.manager.snapshot(window).unwrap();
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if text.contains("? 1 question") == asks && text.contains("the screen") {
                return;
            }
            assert!(Instant::now() < deadline, "screen never {asks}: {text:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Draws the fixture (`asks`) or a screen with no footer, waits until it shows, then
    /// ticks the manager once.
    async fn tick_with(&self, window: u32, ctl: &str, asks: bool) {
        let screen = match asks {
            true => format!("the screen\n{}", fixture_screen()),
            false => "the screen\n\n› Explain this codebase\n  gpt-6.1-sol high".to_string(),
        };
        self.draw(ctl, &screen);
        self.shown(window, asks).await;
        self.manager.tick();
    }
}

/// Review ruling I11: on every tick, a Codex orchestrator's question footer makes its
/// window `Attention` whatever its status; input clears it, the same footer staying on
/// screen does not raise it again, and the footer gone for one tick and back does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_codex_question_footer_is_attention() {
    let rig = rig();
    rig.draw("orch.ctl", "the screen\n");
    let window = rig
        .manager
        .create_run_window(
            spec(rig.dir.path(), "r1/orchestrator"),
            rig.dir.path().to_path_buf(),
            role("orch.ctl"),
        )
        .await
        .expect("the stand-in's window")
        .id;
    rig.signalled(window).await;
    // Its title ends the start's output as `Idle`, or as `Done` when the output came
    // first; a look and its end make it `Idle` either way.
    rig.manager.focus(window);
    rig.manager.unfocus(window);
    rig.shown(window, false).await;
    rig.manager.tick();
    assert_eq!(rig.status(window).0, Status::Idle);
    // From `Idle`.
    rig.tick_with(window, "orch.ctl", true).await;
    assert_eq!(rig.status(window).0, Status::Attention);
    // Input clears it; the same footer, still there, raises nothing.
    rig.manager.write_client_input(window, b"x").unwrap();
    assert_eq!(rig.status(window).0, Status::Working);
    rig.manager.tick();
    rig.manager.tick();
    assert_eq!(rig.status(window).0, Status::Working);
    // From `Working`: gone for one tick, then back.
    rig.tick_with(window, "orch.ctl", false).await;
    assert_eq!(rig.status(window).0, Status::Working);
    rig.tick_with(window, "orch.ctl", true).await;
    assert_eq!(rig.status(window).0, Status::Attention);
    // From `Done`.
    rig.manager.write_client_input(window, b"x").unwrap();
    let stop = json!({"hook_event_name": "Stop", "session_id": "s-codex"});
    rig.manager
        .handle_hook(window, HookSource::CodexHook, &stop)
        .unwrap();
    rig.tick_with(window, "orch.ctl", false).await;
    assert_eq!(rig.status(window).0, Status::Done);
    rig.tick_with(window, "orch.ctl", true).await;
    assert_eq!(rig.status(window).0, Status::Attention);
    let _ = rig.manager.kill(window);
}

/// A Codex window with no role (a plain window) showing the footer stays as it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_codex_window_with_the_footer_is_not_attention() {
    let rig = rig();
    rig.draw("plain.ctl", "the screen\n");
    let plain = rig
        .manager
        .create(
            spec(rig.dir.path(), "plain"),
            rig.dir.path().to_path_buf(),
            None,
            80,
            24,
        )
        .await
        .expect("a plain stand-in window")
        .id;
    let before = rig.signalled(plain).await;
    rig.tick_with(plain, "plain.ctl", true).await;
    rig.manager.tick();
    assert_eq!(rig.status(plain).0, before);
    let _ = rig.manager.kill(plain);
}
