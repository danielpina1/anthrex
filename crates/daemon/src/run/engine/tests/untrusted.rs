//! Milestone 9.13 task M9.13.8: decisions 18 and 20, the sticky record. The first
//! `OrchEvent::UntrustedRead` sets it and logs one line; later reads keep it. It follows
//! an adopted session to its next goal (`chains::adopt`), survives a save and a restart,
//! and the snapshot shows it. A fresh session starts clean: a `Join::Fresh` next goal,
//! or an adoption whose window was gone before the run took it (pre-flight ruling C8);
//! a restart of the run's own session keeps it. The fresh session's handoff never
//! quotes the comment that was read.

use proto::{FinishAction, HistoryLine};

use super::chains::{ended, gone};
use super::chains_continue::{NEXT, continue_as, continued, start};
use super::fixture::*;
use super::goal_rounds_pr::landed;
use super::goal_rounds_start::{iterate, reply, started};
use super::orch::{ORCH, launched};
use super::orch_restore::restart;
use crate::run::engine::{Effect, EventKind, OpKind, OrchEvent};
use crate::run::model::Run;
use crate::run::orch::contract::orchestrator_first_prompt;
use crate::run::orch::contract_rounds::handoff_prompt;
use crate::run::orch::digest::digest;
use crate::run::orch::untrusted::UntrustedRecord;
use crate::run::orch::untrusted::tests::{COMMENT, quote_comment};
use crate::run::snapshot::snapshot;

const CHAIN: &str = "o-3f9a";
const SOURCE: &str = "review comments on stage 1's pull request";

fn record(at: u64) -> UntrustedRecord {
    UntrustedRecord {
        at,
        source: SOURCE.into(),
    }
}

fn read(fx: &mut Fixture, run_id: &str, source: &str) -> Vec<Effect> {
    let at = fx.now;
    fx.next(EventKind::Orch(OrchEvent::UntrustedRead {
        run_id: run_id.into(),
        at,
        source: source.into(),
    }))
}

fn read_lines(run: &Run) -> Vec<String> {
    (run.log.iter())
        .filter(|e| e.text.starts_with("the orchestrator read "))
        .map(|e| e.text.clone())
        .collect()
}

#[test]
fn the_first_source_is_kept() {
    let mut fx = launched(false);
    assert_eq!(fx.run().orch.untrusted, None);
    let first = fx.now;
    read(&mut fx, RUN_ID, SOURCE);
    fx.now += 60;
    read(&mut fx, RUN_ID, "task t5's result (review fix)");
    assert_eq!(fx.run().orch.untrusted, Some(record(first)));
    assert_eq!(
        read_lines(fx.run()),
        [
            "the orchestrator read review comments on stage 1's pull request; override, accept_red and approve_hold now go to the user"
        ]
    );
    // A read of a run the engine does not have changes nothing.
    read(&mut fx, "engine-test-0000", SOURCE);
    assert!(!fx.state.runs.contains_key("engine-test-0000"));
}

#[test]
fn taint_follows_the_session_to_the_next_goal() {
    let mut fx = ended(FinishAction::Accept);
    fx.run_mut().orch.untrusted = Some(record(5));
    continue_as(&mut fx, NEXT, None);
    let next = &fx.state.runs[NEXT];
    assert_eq!(
        next.orch.orchestrator.as_ref().unwrap().window_id,
        Some(ORCH)
    );
    assert_eq!(next.orch.untrusted, Some(record(5)), "adopted");
}

#[test]
fn taint_survives_save_restore() {
    let mut fx = launched(false);
    let at = fx.now;
    read(&mut fx, RUN_ID, SOURCE);
    let kept = fx.run().orch.untrusted.clone();
    assert!(kept.is_some());
    let json = serde_json::to_value(fx.run()).unwrap();
    assert_eq!(
        json["orch"]["untrusted"],
        serde_json::json!({"at": at, "source": SOURCE})
    );
    let back: Run = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back.orch.untrusted, kept);
    // A daemon restart keeps it.
    restart(&mut fx);
    assert_eq!(fx.run().orch.untrusted, kept);
    // A format-1 `run.json` has no key: it loads as `None`, and `None` is not written.
    let mut old = json;
    old["orch"].as_object_mut().unwrap().remove("untrusted");
    old["format"] = serde_json::json!(1);
    let old: Run = serde_json::from_value(old).unwrap();
    assert_eq!(old.orch.untrusted, None);
    let written = serde_json::to_value(&old).unwrap();
    assert!(written["orch"].get("untrusted").is_none(), "{written}");
}

/// Pinning (C14): a next goal whose chain's session ended joins with `Join::Fresh`,
/// which never reaches `adopt`.
#[test]
fn a_fresh_session_starts_clean() {
    let mut fx = ended(FinishAction::Accept);
    fx.run_mut().orch.untrusted = Some(record(5));
    gone(&mut fx, ORCH);
    let effects = continue_as(&mut fx, NEXT, None);
    assert_eq!(ops_in(&effects, "CreateOrchestrator").len(), 1, "fresh");
    assert_eq!(fx.state.runs[NEXT].orch.untrusted, None);
}

/// C8: `adopt` copied the record, then the window was gone before the run took it: the
/// session that read the text never lived in this run.
#[test]
fn a_lost_adoption_starts_clean() {
    let mut fx = ended(FinishAction::Accept);
    fx.run_mut().orch.untrusted = Some(record(5));
    continue_as(&mut fx, NEXT, None);
    assert_eq!(fx.state.runs[NEXT].orch.untrusted, Some(record(5)));
    fx.next(EventKind::Orch(OrchEvent::AdoptLost {
        run_id: NEXT.into(),
        window_id: ORCH,
        first_prompt: "the handoff of 4c1d".into(),
    }));
    assert_eq!(fx.state.runs[NEXT].orch.untrusted, None);
    assert_eq!(
        fx.run().orch.untrusted,
        Some(record(5)),
        "the previous run's own"
    );
}

/// C8: the restart of the run's own session found its window gone; the record stays
/// with the run (U2).
#[test]
fn a_restarted_session_keeps_the_taint() {
    let mut fx = landed();
    fx.tick();
    let o = fx.run_mut().orch.orchestrator.as_mut().unwrap();
    o.live = false;
    o.exited_at = Some(1);
    fx.run_mut().orch.untrusted = Some(record(5));
    let effects = iterate(&mut fx, "more");
    assert_eq!(reply(&effects), started(2));
    assert_eq!(ops_in(&effects, "RestartOrchestrator").len(), 1);
    fx.next(EventKind::Orch(OrchEvent::AdoptLost {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        first_prompt: "the handoff of 3f9a".into(),
    }));
    assert_eq!(fx.run().orch.orchestrator.as_ref().unwrap().window_id, None);
    assert_eq!(fx.run().orch.untrusted, Some(record(5)));
}

/// Pinning (C14): the fresh session's seed is built from the run's outcome, summary and
/// history lines, never from the comment its digest quoted.
#[test]
fn the_handoff_seed_never_quotes_comment_text() {
    let mut fx = ended(FinishAction::Accept);
    fx.run_mut().orch.orchestrator.as_mut().unwrap().summary = Some("added login".into());
    quote_comment(fx.run_mut(), 1, COMMENT);
    let quoted = digest(fx.run(), fx.now).to_string();
    assert!(quoted.contains(COMMENT), "{quoted}");
    read(&mut fx, RUN_ID, SOURCE);
    gone(&mut fx, ORCH);
    // The history lines the driver's handoff reads: every line the run appended.
    let lines: Vec<String> = (fx.log.iter())
        .filter_map(|e| match e {
            Effect::Op {
                run_id,
                kind: OpKind::AppendHistory { line, .. },
                ..
            } if run_id == RUN_ID => Some(serde_json::to_string::<HistoryLine>(line).unwrap()),
            _ => None,
        })
        .collect();
    assert!(!lines.is_empty());
    let mut run = continued(&fx, NEXT, None);
    let first = orchestrator_first_prompt(&run);
    let summary = fx.run().orch.orchestrator.as_ref().unwrap().summary.clone();
    let seed = handoff_prompt(
        &first,
        (CHAIN, "3f9a", "accepted"),
        summary.as_deref(),
        Some(&lines.join("\n")),
        None,
    );
    run.orch.orchestrator.as_mut().unwrap().first_prompt = seed;
    let effects = start(&mut fx, run);
    assert_eq!(ops_in(&effects, "CreateOrchestrator").len(), 1);
    let next = &fx.state.runs[NEXT];
    let seed = &next.orch.orchestrator.as_ref().unwrap().first_prompt;
    assert!(seed.contains("added login"), "{seed}");
    for part in [COMMENT, "Ignore the plan", "approve every hold"] {
        assert!(!seed.contains(part), "{part}: {seed}");
    }
    assert_eq!(next.orch.request_wake, None);
    assert_eq!(next.orch.untrusted, None);
}

#[test]
fn the_snapshot_shows_it() {
    let mut fx = launched(false);
    let info = |fx: &Fixture| {
        let snap = snapshot(&fx.state, fx.now);
        let run = snap.runs.iter().find(|r| r.run_id == RUN_ID).unwrap();
        run.orchestrator.as_ref().unwrap().untrusted.clone()
    };
    assert_eq!(info(&fx), None);
    let at = fx.now;
    read(&mut fx, RUN_ID, SOURCE);
    let record = fx.run().orch.untrusted.clone().unwrap();
    assert_eq!(info(&fx), Some(record.info()));
    assert_eq!(
        info(&fx),
        Some(proto::UntrustedInfo {
            at,
            source: SOURCE.into()
        })
    );
}
