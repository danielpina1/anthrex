//! Milestone 9.16 decisions 25, 27, 29 and 30 (rulings C17, C30): a continued run that
//! arrives with a handoff record starts a fresh session though its chain's window is
//! there; the old window is renamed and left open; a live orchestrator's fill is an
//! attention line only; the record is kept in `run.json` and the history.

use proto::{FinishAction, RunState};

use super::chains::ended;
use super::chains_continue::{NEXT, adopted, continued, start};
use super::dispatch::replies;
use super::fixture::*;
use super::orch::ORCH;
use crate::run::engine::{Effect, EventKind};
use crate::run::model::{HandoffRecord, Run};
use crate::transcript::fill::Fill;

const CHAIN: &str = "o-3f9a";

fn record() -> HandoffRecord {
    HandoffRecord {
        from_run: "3f9a".into(),
        from_window: ORCH,
        fill_tokens: 166_000,
        window_tokens: Some(200_000),
        at: 1_790_000_000,
    }
}

fn renames(effects: &[Effect]) -> Vec<(u32, String)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::RenameWindow { window_id, name } => Some((*window_id, name.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn fill_at_threshold_starts_fresh_session() {
    let mut fx = ended(FinishAction::Accept);
    let tainted = crate::run::orch::untrusted::UntrustedRecord {
        at: 1,
        source: "a web page".into(),
    };
    fx.run_mut().orch.untrusted = Some(tainted);
    assert_eq!(fx.state.chains[CHAIN].window_id, ORCH);
    let mut run = continued(&fx, NEXT, Some("the driver's handoff prompt"));
    run.handoff = Some(record());
    let effects = start(&mut fx, run);
    assert_eq!(replies(&effects), vec![Ok(NEXT.to_string())]);
    assert!(adopted(&effects).is_empty(), "{effects:#?}");
    assert_eq!(
        renames(&effects),
        vec![(ORCH, "3f9a/orchestrator (handed off)".to_string())]
    );
    assert_eq!(
        ops_in(&effects, "CreateOrchestrator").len(),
        1,
        "{effects:#?}"
    );
    let chain = &fx.state.chains[CHAIN];
    assert_eq!(chain.window_id, 0);
    assert_eq!(chain.runs, [RUN_ID, NEXT]);
    let prev = fx.state.runs[RUN_ID].orch.orchestrator.as_ref().unwrap();
    assert!(!prev.live);
    let next = &fx.state.runs[NEXT];
    assert!(
        next.log
            .iter()
            .any(|e| e.text == "handed off from 3f9a: context 83% full (166k / 200k)"),
        "{:#?}",
        next.log
    );
    let o = next.orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.first_prompt, "the driver's handoff prompt");
    assert_ne!(o.window_id, Some(ORCH));
    // Ruling C30: a fresh session starts clean.
    assert_eq!(next.orch.untrusted, None);
    assert_eq!(next.handoff, Some(record()));
    let snap = crate::run::snapshot::snapshot(&fx.state, fx.now);
    let info = snap.runs.iter().find(|r| r.run_id == NEXT).unwrap();
    assert_eq!(info.handoff, Some(record().info()));
}

#[test]
fn fill_below_threshold_adopts() {
    let mut fx = ended(FinishAction::Accept);
    let run = continued(&fx, NEXT, None);
    let effects = start(&mut fx, run);
    assert_eq!(
        adopted(&effects),
        vec![(NEXT.to_string(), ORCH, "4c1d/orchestrator".to_string())]
    );
    assert!(renames(&effects).is_empty());
    assert_eq!(fx.state.chains[CHAIN].window_id, ORCH);
    assert_eq!(fx.state.runs[NEXT].handoff, None);
}

fn fill(tokens: u64, window: Option<u64>) -> Option<Fill> {
    Some(Fill {
        tokens,
        window,
        model: Some("claude-opus-5-5".into()),
    })
}

fn attention(fx: &Fixture) -> Vec<String> {
    crate::run::snapshot::attention(fx.run(), fx.now)
}

#[test]
fn mid_run_fill_adds_an_attention_line_only() {
    let mut fx = ended(FinishAction::Accept);
    fx.run_mut().state = RunState::Running;
    let window = fx.run().orch.orchestrator.as_ref().unwrap().window_id;
    let line = "orchestrator context 92% full (184k / 200k)".to_string();
    let report = |fx: &mut Fixture, f| {
        fx.next(EventKind::OrchestratorFill {
            run_id: RUN_ID.into(),
            fill: f,
        })
    };
    let effects = report(&mut fx, fill(184_000, Some(200_000)));
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    // No alert, no handoff, the window untouched.
    assert!(renames(&effects).is_empty() && adopted(&effects).is_empty());
    assert!(ops_in(&effects, "CreateOrchestrator").is_empty());
    let o = fx.run().orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.window_id, window);
    assert_eq!(fx.run().handoff, None);
    // Ruling C17: the same fill again moves nothing.
    let revision = fx.state.revision;
    report(&mut fx, fill(184_100, Some(200_000)));
    assert_eq!(fx.state.revision, revision);
    // 85 %: below the line.
    report(&mut fx, fill(170_000, Some(200_000)));
    assert!(
        !attention(&fx)
            .iter()
            .any(|l| l.starts_with("orchestrator context")),
        "{:?}",
        attention(&fx)
    );
    // An unknown window shows nothing however many tokens.
    report(&mut fx, fill(990_000, None));
    assert!(
        !attention(&fx)
            .iter()
            .any(|l| l.starts_with("orchestrator context"))
    );
}

/// Fix round 1 (review I2): ruling C17 keys on the attention's threshold percent and
/// whether the window is known, not on the line's text. A step that moves the shown
/// tokens but not the percent bumps no revision (and keeps the line), a new percent
/// does, and past the window (`over 100%`) more tokens bump nothing.
#[test]
fn a_fill_moves_the_run_only_when_its_percent_does() {
    let mut fx = ended(FinishAction::Accept);
    fx.run_mut().state = RunState::Running;
    let report = |fx: &mut Fixture, f| {
        fx.next(EventKind::OrchestratorFill {
            run_id: RUN_ID.into(),
            fill: f,
        })
    };
    report(&mut fx, fill(184_000, Some(200_000)));
    let revision = fx.state.revision;
    // 184k to 185k: both 92%.
    report(&mut fx, fill(185_500, Some(200_000)));
    assert_eq!(
        fx.state.revision, revision,
        "a sub-percent step moved the run"
    );
    let line = "orchestrator context 92% full (184k / 200k)".to_string();
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    // 93%: the line moves.
    report(&mut fx, fill(186_000, Some(200_000)));
    assert!(
        fx.state.revision > revision,
        "a new percent did not move the run"
    );
    let line = "orchestrator context 93% full (186k / 200k)".to_string();
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    // Past the window: over 100% is over 100%, however many more tokens.
    report(&mut fx, fill(900_000, Some(200_000)));
    let revision = fx.state.revision;
    report(&mut fx, fill(950_000, Some(200_000)));
    assert_eq!(
        fx.state.revision, revision,
        "more tokens past the window moved the run"
    );
    let line = "orchestrator context over 100% (900k / 200k window)".to_string();
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
}

const M93_RUN: &str = include_str!("../../../../tests/fixtures/run/m93-run.json");
const M95_RUN: &str = include_str!("../../../../tests/fixtures/run/m95-run.json");

/// Pinning: older runs load with no handoff, and a saved one round-trips it.
#[test]
fn old_run_json_without_handoff_loads() {
    for (name, text) in [("m93", M93_RUN), ("m95", M95_RUN)] {
        let run: Run = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(run.handoff, None, "{name}");
        let saved = serde_json::to_value(&run).unwrap();
        assert!(saved.get("handoff").is_none(), "{name}: written while None");
    }
    let mut run: Run = serde_json::from_str(M95_RUN).unwrap();
    run.handoff = Some(record());
    let text = serde_json::to_string(&run).unwrap();
    let back: Run = serde_json::from_str(&text).unwrap();
    assert_eq!(back.handoff, Some(record()));
}

#[test]
fn history_run_record_carries_handoff() {
    let mut fx = ended(FinishAction::Accept);
    fx.run_mut().handoff = Some(record());
    let line = crate::run::history::run_record(fx.run(), "accepted", fx.now);
    assert_eq!(line.handoff, Some(record().info()));
    let value = serde_json::to_value(&line).unwrap();
    assert_eq!(value["handoff"]["fill_tokens"], 166_000);
}
