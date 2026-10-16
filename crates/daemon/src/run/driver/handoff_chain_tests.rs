//! Milestone 9.16 decisions 22–28 (U13, rulings C14 and C15): a continued goal measures
//! the chain's session before adopting it. On `chain_goal_tests.rs`' rig (a real socket,
//! a temporary checkout with a stored profile, a real PTY window whose `claude` or
//! `codex` is a stand-in that sleeps), with the session's transcript or rollout real
//! files in the rig's temporary directory, named by a real `SessionStart` hook. No agent
//! runs; no test reads `~/.claude` or `~/.codex` (the Codex sessions folder is the rig's
//! own). Each test kills the windows its rig made.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{Effort, HookSource, Route, Runtime};
use serde_json::json;

use crate::live_config::LiveSettings;
use crate::run::driver::RunContext;
use crate::run::driver::orch::chain_goal::tests::{ANSWER, ChainRig, PREV, started};
use crate::run::engine::HISTORY_FILE;
use crate::run::model::{HandoffRecord, Run};
use crate::run::orch::contract_rounds::FILL_UNKNOWN_TEXT;

/// A history line of [`PREV`]'s, which a fresh session's prompt carries.
const MARK: &str = "the-history-line-of-3f9a";

/// A Claude transcript at `<tmp>/transcript.jsonl` whose last main-chain assistant line
/// sums to `tokens` (a sidechain line after it is skipped).
fn transcript(rig: &ChainRig, tokens: u64) -> PathBuf {
    let path = rig.checkout.tmp.path().join("transcript.jsonl");
    let line = |sidechain: bool, input: u64| {
        json!({
            "type": "assistant",
            "isSidechain": sidechain,
            "message": {"model": "claude-opus-5-5", "usage": {
                "input_tokens": input, "cache_read_input_tokens": 0,
                "cache_creation_input_tokens": 0, "output_tokens": 0}},
        })
        .to_string()
    };
    let text = format!("{}\n{}\n", line(false, tokens), line(true, 1));
    std::fs::write(&path, text).unwrap();
    path
}

/// The chain's window's `SessionStart`, naming `transcript` (Claude) or the thread
/// `session` (Codex).
fn session_start(rig: &ChainRig, runtime: Runtime, transcript: Option<&Path>, session: &str) {
    let (source, mut hook) = match runtime {
        Runtime::Codex => (HookSource::CodexHook, json!({})),
        _ => (HookSource::Claude, json!({})),
    };
    hook["hook_event_name"] = json!("SessionStart");
    hook["session_id"] = json!(session);
    hook["source"] = json!("startup");
    if let Some(path) = transcript {
        hook["transcript_path"] = json!(path);
    }
    rig.manager.handle_hook(rig.window, source, &hook).unwrap();
}

/// [`PREV`]'s line in the project's history file.
fn history(rig: &ChainRig) {
    let path = crate::lock(&rig.s.state).runs[PREV]
        .repo_dir
        .join(HISTORY_FILE);
    let line = json!({"type": "run", "run_id": PREV, "note": MARK});
    std::fs::write(path, format!("{line}\n")).unwrap();
}

/// The goal continued from [`PREV`], as the engine started it.
async fn continued(rig: &ChainRig) -> Run {
    let run_id = started(&rig.continued(PREV, &rig.checkout.work.clone()).await);
    crate::lock(&rig.s.state).runs[&run_id].clone()
}

/// Whether `run` adopted the chain's window.
fn adopted(rig: &ChainRig, run: &Run) -> bool {
    run.handoff.is_none()
        && run.orch.orchestrator.as_ref().and_then(|o| o.window_id) == Some(rig.window)
}

/// Waits until the chain's old window is named `name`.
async fn named(rig: &ChainRig, name: &str) {
    let deadline = Instant::now() + ANSWER;
    loop {
        let list = rig.manager.list();
        if list.iter().any(|w| w.id == rig.window && w.name == name) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the window was never renamed {name}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Stops the rig and kills every window its manager made (the fresh orchestrator's too).
async fn finish(rig: ChainRig) {
    let manager = rig.manager.clone();
    rig.stop().await;
    for window in manager.list() {
        let _ = manager.kill(window.id);
    }
}

/// `adjust`'s change of the rig's `[orchestrator]` handoff keys.
fn keys(ctx: &mut RunContext, percent: u8, cap: u64) {
    let mut orchestrator = ctx.settings.current().orchestrator.clone();
    orchestrator.handoff_at_percent = percent;
    orchestrator.handoff_at_tokens = cap;
    ctx.settings = LiveSettings::defaults_of(orchestrator);
}

/// The hand-off the tests expect of [`PREV`]'s window.
fn record(
    rig: &ChainRig,
    run: &Run,
    fill_tokens: u64,
    window_tokens: Option<u64>,
) -> HandoffRecord {
    HandoffRecord {
        from_run: "3f9a".into(),
        from_window: rig.window,
        fill_tokens,
        window_tokens,
        at: run.handoff.as_ref().map_or(0, |h| h.at),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_orchestrator_at_80_percent_hands_off() {
    let rig = ChainRig::new(|_| {}).await;
    let path = transcript(&rig, 170_000);
    session_start(&rig, Runtime::Claude, Some(&path), "sess-a");
    history(&rig);
    let run = continued(&rig).await;
    assert_eq!(
        run.handoff,
        Some(record(&rig, &run, 170_000, Some(200_000)))
    );
    let o = run.orch.orchestrator.as_ref().unwrap();
    assert_ne!(o.window_id, Some(rig.window), "the old window was adopted");
    assert!(
        o.first_prompt
            .ends_with("The previous session was 85% full (170000/200000); this one starts fresh."),
        "{}",
        o.first_prompt
    );
    assert!(o.first_prompt.contains(MARK), "{}", o.first_prompt);
    let texts: Vec<&str> = run.log.iter().map(|l| l.text.as_str()).collect();
    assert!(
        texts.contains(&"handed off from 3f9a: context 85% full"),
        "{texts:?}"
    );
    named(&rig, "3f9a/orchestrator (handed off)").await;
    finish(rig).await;
}

/// U13: whichever comes first; on a 1M window 310,000 tokens is 31 %, past the cap.
#[tokio::test(flavor = "multi_thread")]
async fn cap_fires_first_on_a_1m_window() {
    let rig = ChainRig::new(|prev| {
        let o = prev.orch.orchestrator.as_mut().unwrap();
        o.route.model = "claude-opus-5-5[1m]".into();
    })
    .await;
    let path = transcript(&rig, 310_000);
    session_start(&rig, Runtime::Claude, Some(&path), "sess-a");
    let run = continued(&rig).await;
    assert_eq!(
        run.handoff,
        Some(record(&rig, &run, 310_000, Some(1_000_000)))
    );
    finish(rig).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn zero_disables_each_criterion() {
    for (percent, cap, tokens) in [(0, 300_000, 190_000), (0, 0, 900_000)] {
        let rig = ChainRig::with_context(|_| {}, |ctx| keys(ctx, percent, cap)).await;
        let path = transcript(&rig, tokens);
        session_start(&rig, Runtime::Claude, Some(&path), "sess-a");
        let run = continued(&rig).await;
        assert!(adopted(&rig, &run), "{percent} {cap}: {:?}", run.handoff);
        let texts: Vec<&str> = run.log.iter().map(|l| l.text.as_str()).collect();
        assert!(!texts.contains(&FILL_UNKNOWN_TEXT), "{texts:?}");
        finish(rig).await;
    }
}

/// U13: a Codex orchestrator's fill is its rollout's, found by the thread id its
/// `SessionStart` named, under the rig's own sessions folder.
#[tokio::test(flavor = "multi_thread")]
async fn codex_orchestrator_auto_hands_off_from_its_rollout() {
    let sessions = tempfile::Builder::new()
        .prefix("anthrex-handoff-codex-")
        .tempdir_in("/tmp")
        .unwrap();
    let dir = sessions.path().to_path_buf();
    let rig = ChainRig::with_context(
        |prev| {
            let o = prev.orch.orchestrator.as_mut().unwrap();
            o.route = Route {
                runtime: Runtime::Codex,
                model: "gpt-6".into(),
                effort: Effort::HIGH,
            };
        },
        |ctx| ctx.codex_sessions = Some(dir.clone()),
    )
    .await;
    let thread = format!("0199{}-handoff", std::process::id());
    let date = common::text::format_utc(super::super::unix_now());
    let day = date[..10].split('-').fold(dir.clone(), |d, p| d.join(p));
    std::fs::create_dir_all(&day).unwrap();
    let usage = json!({"type": "event_msg", "payload": {"type": "token_count",
        "info": {"last_token_usage": {"input_tokens": 219_000, "cached_input_tokens": 0,
            "output_tokens": 1_000, "total_tokens": 220_000}, "model_context_window": 258_400},
        "rate_limits": null}});
    let name = format!("rollout-2026-10-16T10-00-00-{thread}.jsonl");
    std::fs::write(day.join(name), format!("{usage}\n")).unwrap();
    session_start(&rig, Runtime::Codex, None, &thread);
    let run = continued(&rig).await;
    assert_eq!(
        run.handoff,
        Some(record(&rig, &run, 220_000, Some(258_400)))
    );
    named(&rig, "3f9a/orchestrator (handed off)").await;
    finish(rig).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unmeasured_fill_adopts_and_logs_once() {
    let rig = ChainRig::new(|_| {}).await;
    let run = continued(&rig).await;
    assert!(adopted(&rig, &run), "{:?}", run.handoff);
    let unknown = run
        .log
        .iter()
        .filter(|l| l.text == FILL_UNKNOWN_TEXT)
        .count();
    assert_eq!(unknown, 1, "{:?}", run.log);
    finish(rig).await;
}

/// Ruling C26: a FIFO named as the transcript is never opened (`read_tail` reads a
/// regular file only), and a read that hangs past the injected 200 ms wait is given up
/// on; both adopt with the unknown line. The hung read is released at the end.
#[tokio::test(flavor = "multi_thread")]
async fn transcript_read_times_out_and_adopts() {
    let wait = Duration::from_millis(200);
    let rig = ChainRig::with_context(|_| {}, |ctx| ctx.fill_read_cap = Some(wait)).await;
    let fifo = rig.checkout.tmp.path().join("transcript.jsonl");
    let c_path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: a plain mkfifo on a path this test owns.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    session_start(&rig, Runtime::Claude, Some(&fifo), "sess-a");
    let run = continued(&rig).await;
    assert!(adopted(&rig, &run), "{:?}", run.handoff);
    assert!(
        run.log.iter().any(|l| l.text == FILL_UNKNOWN_TEXT),
        "{:?}",
        run.log
    );
    finish(rig).await;

    // A read that hangs (a stalled filesystem), the rig's reader replaced.
    let rig = ChainRig::with_context(|_| {}, |ctx| ctx.fill_read_cap = Some(wait)).await;
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = std::sync::Mutex::new(released);
    rig.s.handoff.set_reader(Arc::new(move |_, _| {
        let _ = crate::lock(&released).recv_timeout(wait * 100);
        None
    }));
    let path = transcript(&rig, 190_000);
    session_start(&rig, Runtime::Claude, Some(&path), "sess-a");
    let begun = Instant::now();
    let run = continued(&rig).await;
    let _ = release.send(());
    assert!(adopted(&rig, &run), "{:?}", run.handoff);
    assert!(
        run.log.iter().any(|l| l.text == FILL_UNKNOWN_TEXT),
        "{:?}",
        run.log
    );
    assert!(
        begun.elapsed() < wait * 100,
        "the start waited for the hung read"
    );
    finish(rig).await;
}

/// Ruling C15: the orchestrator's own `start_goal` is a goal boundary too.
#[tokio::test(flavor = "multi_thread")]
async fn the_start_goal_tool_hands_off_too() {
    let rig = ChainRig::new(|_| {}).await;
    let path = transcript(&rig, 170_000);
    session_start(&rig, Runtime::Claude, Some(&path), "sess-a");
    let answer = tokio::time::timeout(ANSWER, rig.s.start_goal_tool(PREV, "Add logout".into()))
        .await
        .expect("answered")
        .expect("started");
    let run_id = answer
        .strip_prefix("run ")
        .unwrap()
        .strip_suffix(" started")
        .unwrap();
    let run = crate::lock(&rig.s.state).runs[run_id].clone();
    assert_eq!(
        run.handoff,
        Some(record(&rig, &run, 170_000, Some(200_000)))
    );
    finish(rig).await;
}

/// Decision 29: `on_tick`'s check measures a running run's live orchestrator off every
/// lock and reports it; the engine shows it as an attention line.
#[tokio::test(flavor = "multi_thread")]
async fn the_live_check_reports_a_running_orchestrators_fill() {
    let rig = ChainRig::new(|_| {}).await;
    let path = transcript(&rig, 184_000);
    session_start(&rig, Runtime::Claude, Some(&path), "sess-a");
    {
        let mut state = crate::lock(&rig.s.state);
        let prev = state.runs.get_mut(PREV).unwrap();
        prev.state = proto::RunState::Running;
        prev.orch.orchestrator.as_mut().unwrap().live = true;
    }
    let deadline = Instant::now() + ANSWER;
    let fill = loop {
        // `on_tick` may have checked before the run was running.
        rig.s.handoff.rearm();
        rig.s.check_live();
        let fill = crate::lock(&rig.s.state).runs[PREV].orch.fill.clone();
        if let Some(fill) = fill {
            break fill;
        }
        assert!(Instant::now() < deadline, "no fill was reported");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!((fill.tokens, fill.window), (184_000, Some(200_000)));
    let run = crate::lock(&rig.s.state).runs[PREV].clone();
    let lines = crate::run::snapshot::attention(&run, super::super::unix_now());
    assert!(
        lines.contains(&"orchestrator context 92% full".to_string()),
        "{lines:?}"
    );
    finish(rig).await;
}
