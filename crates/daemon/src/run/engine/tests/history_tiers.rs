//! Milestone 9.1 task M9.1.18: decision 57's `tier`, `flaky` and `bisect` history
//! lines, each appended once through M8b's journaled `AppendHistory` with its stable
//! `record_id`.

use std::path::PathBuf;

use proto::{BisectLine, FlakyRecord, HISTORY_VERSION, HistoryLine, TierRunRecord};

use super::bisect::{TEST, merged, red_full, with_orchestrator};
use super::fixture::*;
use super::gates::only_op;
use super::merge::commit;
use super::tiers::{outcome, proved, working};
use crate::run::contract::sha7;
use crate::run::engine::{Effect, OpKind, OpResult};

/// Every history line the effects append, with its `record_id`.
pub(super) fn lines(effects: &[Effect]) -> Vec<(String, HistoryLine)> {
    ops_in(effects, "AppendHistory")
        .into_iter()
        .map(|(_, kind)| match kind {
            OpKind::AppendHistory {
                record_id, line, ..
            } => (record_id, *line),
            _ => unreachable!(),
        })
        .collect()
}

pub(super) fn tier_lines(all: &[(String, HistoryLine)]) -> Vec<TierRunRecord> {
    all.iter()
        .filter_map(|(_, l)| match l {
            HistoryLine::Tier(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn flaky_lines(all: &[(String, HistoryLine)]) -> Vec<FlakyRecord> {
    all.iter()
        .filter_map(|(_, l)| match l {
            HistoryLine::Flaky(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn bisect_lines(all: &[(String, HistoryLine)]) -> Vec<BisectLine> {
    all.iter()
        .filter_map(|(_, l)| match l {
            HistoryLine::Bisect(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

/// A few idle passes: nothing more may be appended for a job already recorded.
fn ticks(fx: &mut Fixture, all: &mut Vec<(String, HistoryLine)>) {
    for _ in 0..3 {
        all.extend(lines(&fx.tick()));
    }
}

#[test]
fn tier_flaky_and_bisect_lines_are_written_once_each() {
    // A green tier 1, then a green tier 2 with one flaky test.
    let (mut fx, window) = working();
    let run_id = fx.run().id.clone();
    let effects = proved(&mut fx, window);
    let (tier1, _) = only_op(&effects, "Tier");
    let mut all = lines(&fx.done(tier1, OpResult::Tier(Box::new(outcome(1, true, false)))));
    let (candidate, _) = pending_one_candidate(&fx);
    let mut flaky = outcome(2, true, false);
    flaky.steps[1].retried = true;
    flaky.steps[1].flaky = vec!["a::flaky".into()];
    all.extend(lines(&fx.done(
        candidate,
        OpResult::Merged {
            commit: "1".repeat(40),
            tier: Some(Box::new(flaky)),
        },
    )));
    ticks(&mut fx, &mut all);

    let tiers = tier_lines(&all);
    assert_eq!(tiers.len(), 2, "one line per tier job: {tiers:#?}");
    let t2 = &tiers[1];
    let expected = TierRunRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run_id}/tier/{candidate}"),
        at: t2.at,
        run_id: run_id.clone(),
        task_id: Some("t1".into()),
        stage: 1,
        tier: 2,
        secs: 10,
        affected: 1,
        full_reason: None,
        cache_hit: false,
        cached_steps: 0,
        steps: 2,
        ok: true,
        flaky: vec!["a::flaky".into()],
    };
    assert_eq!(t2, &expected);
    assert_eq!(tiers[0].record_id, format!("{run_id}/tier/{tier1}"));
    assert_eq!((tiers[0].tier, tiers[0].ok), (1, true));
    assert!(tiers[0].flaky.is_empty());
    let flakes = flaky_lines(&all);
    assert_eq!(
        flakes,
        [FlakyRecord {
            v: HISTORY_VERSION,
            record_id: format!("{run_id}/flaky/{candidate}/a::flaky"),
            at: t2.at,
            run_id: run_id.clone(),
            task_id: Some("t1".into()),
            tier: 2,
            test: "a::flaky".into(),
        }]
    );
    assert!(bisect_lines(&all).is_empty());
    // Each line carries its own record id as the op's.
    for (id, line) in &all {
        assert_eq!(id, &crate::run::history_io::record_id(line).to_string());
    }

    // A red tier 3 bisected to `t4`: one `tier` line for the red job, one `bisect`.
    let mut fx = merged(&["t1", "t2", "t3", "t4", "t5", "t6"], "");
    with_orchestrator(&mut fx);
    fx.run_mut().repo_dir = PathBuf::from("/tmp/data/repos/r-1");
    let run_id = fx.run().id.clone();
    let mut all = lines(&red_full(&mut fx));
    let tiers = tier_lines(&all);
    assert_eq!(tiers.len(), 1, "{tiers:#?}");
    let t3 = &tiers[0];
    assert_eq!(
        (t3.task_id.clone(), t3.stage, t3.tier, t3.ok, t3.affected),
        (None, 1, 3, false, 0)
    );
    assert_eq!(t3.full_reason.as_deref(), Some("full suite"));
    assert!(t3.record_id.starts_with(&format!("{run_id}/tier/")));
    // The probes add nothing until the bisect ends; then exactly one line.
    answer_collecting(&mut fx, 4, &mut all);
    ticks(&mut fx, &mut all);
    let bisects = bisect_lines(&all);
    assert_eq!(
        bisects,
        [BisectLine {
            v: HISTORY_VERSION,
            record_id: format!("{run_id}/bisect/1/1"),
            at: bisects[0].at,
            run_id: run_id.clone(),
            stage: 1,
            head: commit(6),
            tests: vec![TEST.into()],
            range: 6,
            probes: 4,
            culprit: Some("t4".into()),
            reason: None,
            fix_task: Some("fix1".into()),
        }]
    );
    assert_eq!(tier_lines(&all).len(), 1, "no tier line for a probe");
}

#[test]
fn a_bisect_without_a_culprit_records_its_reason() {
    let mut fx = merged(&["t1", "t2"], "");
    with_orchestrator(&mut fx);
    fx.run_mut().repo_dir = PathBuf::from("/tmp/data/repos/r-1");
    let run_id = fx.run().id.clone();
    let mut all = lines(&red_full(&mut fx));
    // Red everywhere: `G` is red, so no merge is to blame.
    answer_collecting(&mut fx, 0, &mut all);
    ticks(&mut fx, &mut all);
    let bisects = bisect_lines(&all);
    assert_eq!(bisects.len(), 1, "{bisects:#?}");
    let b = &bisects[0];
    assert_eq!(b.record_id, format!("{run_id}/bisect/1/1"));
    assert_eq!((b.range, b.probes), (2, 1));
    assert_eq!((b.culprit.clone(), b.fix_task.clone()), (None, None));
    assert_eq!(
        b.reason,
        Some(format!("the failing tests already fail at {}", sha7(BASE)))
    );
}

#[test]
fn a_run_without_history_writes_no_tier_line() {
    let mut fx = merged(&["t1"], "");
    assert!(fx.run().repo_dir.as_os_str().is_empty());
    let all = lines(&red_full(&mut fx));
    assert!(tier_lines(&all).is_empty(), "{all:#?}");
}

/// The one pending `MergeCandidate`.
fn pending_one_candidate(fx: &Fixture) -> (crate::run::model::OpId, OpKind) {
    super::merge::pending_one(fx, "MergeCandidate", Some("t1"))
}

/// [`answer`], keeping every line the probes' results append.
pub(super) fn answer_collecting(
    fx: &mut Fixture,
    red_from: u32,
    all: &mut Vec<(String, HistoryLine)>,
) {
    let mut n = 0;
    while !super::merge::pending(fx, "TestAt", None).is_empty() {
        let (op, spec) = super::bisect::probe(fx);
        let red = index(&spec.commit) >= red_from;
        all.extend(lines(
            &fx.done(op, super::bisect::probe_result(red, &spec.commit)),
        ));
        n += 1;
        assert!(n <= 12, "the bisect does not end");
    }
}

fn index(commit: &str) -> u32 {
    if commit == BASE {
        return 0;
    }
    let digits: String = commit.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect(commit)
}
