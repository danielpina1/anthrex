//! Milestone 9.5 decision 40 (task M9.5.5b, review ruling I11): the footer, pure, and
//! its scan on every manager tick against a real manager and real PTY windows whose
//! `codex` is a stand-in shell script (never an agent) drawing what a control file
//! holds. Each test kills only the windows it made.

use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{HookSource, Runtime, Status};
use serde_json::json;

use super::rig::{SLACK, draw, fixture_screen, role, screen, shown, spec, stand_in};
use super::*;
use crate::launch::LaunchGate;
use crate::manager::{ManagerConfig, WindowManager};
use crate::status::{CodexTitle, StatusContext, StatusEvent, next};

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
    // The count (review m6): the highest match; none counted is no footer.
    assert_eq!(question_count(&["? 2 questions", "? 3 questions"]), Some(3));
    assert_eq!(question_count(&["? 1 question"]), Some(1));
    assert_eq!(question_count(&["? 0 questions"]), None);
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

/// Ruling T5b-1: while the footer shows (`StatusContext::codex_question`), a Codex
/// window's `Attention` holds against every event but client input and its exit; with
/// the footer gone, the same events move it as before.
#[test]
fn a_codex_question_holds_attention_against_all_but_input_and_exit() {
    use StatusEvent as E;
    let events = [
        E::Output,
        E::Quiet,
        E::Bell,
        E::Focused,
        E::SessionStart,
        E::UserPromptSubmit,
        E::PreToolUse,
        E::PostToolUse,
        E::PermissionRequest,
        E::PermissionPrompt,
        E::IdlePrompt,
        E::Stop,
        E::CodexNotify,
        E::CodexQuestion,
        E::Title(CodexTitle::Starting),
        E::Title(CodexTitle::Working),
        E::Title(CodexTitle::Thinking),
        E::Title(CodexTitle::Waiting),
        E::Title(CodexTitle::Ready),
    ];
    for hooks_seen in [false, true] {
        for focused in [false, true] {
            let held = StatusContext {
                focused,
                signals_seen: true,
                hooks_seen,
                codex_question: true,
            };
            for event in events {
                assert_eq!(
                    next(Status::Attention, event, Runtime::Codex, held),
                    Status::Attention,
                    "{event:?} hooks {hooks_seen} focused {focused}"
                );
            }
            let input = next(Status::Attention, E::InputSent, Runtime::Codex, held);
            assert_eq!(input, Status::Working);
            let exit = next(Status::Attention, E::Exited, Runtime::Codex, held);
            assert_eq!(exit, Status::Exited);
            // Only `Attention` is held: a title still moves an `Idle` window.
            let idle = next(Status::Idle, E::Stop, Runtime::Codex, held);
            assert_eq!(idle, if focused { Status::Idle } else { Status::Done });
        }
    }
    // The footer gone: released, with nothing forced.
    let free = StatusContext {
        signals_seen: true,
        ..StatusContext::default()
    };
    let title = E::Title(CodexTitle::Working);
    assert_eq!(
        next(Status::Attention, title, Runtime::Codex, free),
        Status::Working
    );
    assert_eq!(
        next(Status::Attention, E::Stop, Runtime::Codex, free),
        Status::Done
    );
    // A Claude window is never held.
    let held = StatusContext {
        codex_question: true,
        ..free
    };
    assert_eq!(
        next(Status::Attention, E::Stop, Runtime::Claude, held),
        Status::Done
    );
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

impl Rig {
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

    /// Draws [`screen`]`(questions)`, waits until it shows, then ticks the manager once.
    async fn tick_with(&self, window: u32, ctl: &str, questions: Option<u32>) {
        draw(self.dir.path(), ctl, &screen(questions));
        shown(&self.manager, window, questions).await;
        self.manager.tick();
    }

    /// A Codex orchestrator window for run `r1` drawing `orch.ctl`, signalled and
    /// `Idle` with no footer.
    async fn orchestrator(&self) -> u32 {
        draw(self.dir.path(), "orch.ctl", &screen(None));
        let window = self
            .manager
            .create_run_window(
                spec(self.dir.path(), "r1/orchestrator"),
                self.dir.path().to_path_buf(),
                role("r1", "orch.ctl"),
            )
            .await
            .expect("the stand-in's window")
            .id;
        self.signalled(window).await;
        // Its title ends the start's output as `Idle`, or as `Done` when the output came
        // first; a look and its end make it `Idle` either way.
        self.manager.focus(window);
        self.manager.unfocus(window);
        shown(&self.manager, window, None).await;
        self.manager.tick();
        assert_eq!(self.status(window).0, Status::Idle);
        window
    }
}

/// Review ruling I11: on every tick, a Codex orchestrator's question footer makes its
/// window `Attention` whatever its status; input clears it, the same footer staying on
/// screen does not raise it again, and the footer gone for one tick and back does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_codex_question_footer_is_attention() {
    let rig = rig();
    let window = rig.orchestrator().await;
    // From `Idle`.
    rig.tick_with(window, "orch.ctl", Some(1)).await;
    assert_eq!(rig.status(window).0, Status::Attention);
    // Input clears it; the same footer, still there, raises nothing.
    rig.manager.write_client_input(window, b"x").unwrap();
    assert_eq!(rig.status(window).0, Status::Working);
    rig.manager.tick();
    rig.manager.tick();
    assert_eq!(rig.status(window).0, Status::Working);
    // From `Working`: gone for one tick, then back.
    rig.tick_with(window, "orch.ctl", None).await;
    assert_eq!(rig.status(window).0, Status::Working);
    rig.tick_with(window, "orch.ctl", Some(1)).await;
    assert_eq!(rig.status(window).0, Status::Attention);
    // From `Done`.
    rig.manager.write_client_input(window, b"x").unwrap();
    let stop = json!({"hook_event_name": "Stop", "session_id": "s-codex"});
    rig.manager
        .handle_hook(window, HookSource::CodexHook, &stop)
        .unwrap();
    rig.tick_with(window, "orch.ctl", None).await;
    assert_eq!(rig.status(window).0, Status::Done);
    rig.tick_with(window, "orch.ctl", Some(1)).await;
    assert_eq!(rig.status(window).0, Status::Attention);
    let _ = rig.manager.kill(window);
}

/// A Codex window with no role (a plain window) showing the footer stays as it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_codex_window_with_the_footer_is_not_attention() {
    let rig = rig();
    draw(rig.dir.path(), "plain.ctl", &screen(None));
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
    rig.tick_with(plain, "plain.ctl", Some(1)).await;
    rig.manager.tick();
    assert_eq!(rig.status(plain).0, before);
    let _ = rig.manager.kill(plain);
}

/// Review m6: after input, the same count on screen raises nothing, one answered (a
/// lower count) raises nothing, and a further question (a higher count) raises it
/// again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_further_question_raises_it_again() {
    let rig = rig();
    let window = rig.orchestrator().await;
    rig.tick_with(window, "orch.ctl", Some(2)).await;
    assert_eq!(rig.status(window).0, Status::Attention);
    rig.manager.write_client_input(window, b"x").unwrap();
    assert_eq!(rig.status(window).0, Status::Working);
    rig.tick_with(window, "orch.ctl", Some(1)).await;
    assert_eq!(rig.status(window).0, Status::Working);
    rig.tick_with(window, "orch.ctl", Some(3)).await;
    assert_eq!(rig.status(window).0, Status::Attention);
    let _ = rig.manager.kill(window);
}
