//! Decision 27 step 4 (task M9.2.9): a CI red reproduced at the PR head with 9.1's
//! probe, holding `Run.full_op` (one tier-3 job or bisect per run at a time):
//! `single_test` once per failing name, never the whole suite; `build_check` (else
//! `check`) for a build or lint red; else the stage's tier-2 steps. Red with names on a
//! tiered profile goes to 9.1's bisect; any other red, and a green, is a stage fix.
//! Pure (design decision 2).

use proto::CiCategory;

use super::super::requests::log;
use super::super::{Effect, OpId, OpKind, OpResult, ScratchAt, bisect, tiers};
use super::ci::{drop_record, gone, record, record_mut};
use super::fix::{self, Repro};
use crate::run::contract::sha7;
use crate::run::delivery::snapshot::stage_count;
use crate::run::delivery::{CiPhase, CiRecord};
use crate::run::env::profile_env;
use crate::run::model::Run;
use crate::run::proof::proof_command;
use crate::run::slots::Priority;
use crate::run::tiers::{RETRY_NAMES_MAX, TestAtSpec};

/// A local reproduction the executor could not run is tried this many times, after
/// these waits (9.1's ruling C-18 for probes).
const PROBE_TRIES: u8 = 3;
const PROBE_BACKOFF: [u64; 2] = [30, 120];

/// How record `rec` is reproduced (step 4): `single_test` per failing name; else, for a
/// build or lint red, `build_check` (else `check`); else the stage's tier-2 steps.
enum Plan {
    Tests(Vec<String>),
    One(String),
    Tier2,
    None(&'static str),
}

fn plan(run: &Run, rec: &CiRecord) -> Plan {
    let p = &run.profile;
    if let (false, Some(single)) = (rec.failing_tests.is_empty(), &p.single_test) {
        let names = rec.failing_tests.iter().take(RETRY_NAMES_MAX);
        return Plan::Tests(names.map(|t| proof_command(single, t)).collect());
    }
    if matches!(rec.category, Some(CiCategory::Build | CiCategory::Lint)) {
        return match p.tiers.build_check.clone().or_else(|| p.check.clone()) {
            Some(c) => Plan::One(c),
            None => Plan::None("the profile has no build_check or check"),
        };
    }
    if tiers::has_check_gate(run) {
        Plan::Tier2
    } else {
        Plan::None("the profile has no command to run it")
    }
}

/// What stage `n`'s changes are measured from for a tier-2 reproduction: the stage
/// below's head, else where the stage was created.
fn stage_base(run: &Run, n: u16) -> String {
    let below = n
        .checked_sub(1)
        .and_then(|m| run.stage(m))
        .map(|s| s.head.clone());
    let created = run.stage(n).map(|s| s.created_from.clone());
    below.or(created).unwrap_or_default()
}

/// Step 4: the reproduction at the PR head, holding `full_op` (one tier-3 job or bisect
/// per run at a time); it waits while one is in flight.
pub(super) fn reproduce(run: &mut Run, n: u16, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let Some(rec) = record(run, n, i).cloned() else {
        return;
    };
    if run.full_op.is_some() || bisect::bisecting(run) || rec.probe.is_some() {
        return;
    }
    let dir = run.full_path();
    let head = rec.head.clone();
    let scratch = ScratchAt {
        root: run.root.clone(),
        commit: head.clone(),
        setup: run.profile.setup.clone(),
    };
    let (kind, shown) = match plan(run, &rec) {
        Plan::None(why) => {
            return fix::add_stage(run, n, i, Repro::Unavailable(why.into()), now, fx);
        }
        Plan::Tests(commands) => {
            let shown = commands.join("; ");
            (test_at(run, &head, commands), shown)
        }
        Plan::One(command) => (test_at(run, &head, vec![command.clone()]), command),
        Plan::Tier2 => {
            let mut job = tiers::spec(run, 2, n, dir);
            job.scratch = Some(scratch);
            job.diff_base = stage_base(run, n);
            job.head = head.clone();
            job.priority = Priority::FullStage;
            (
                OpKind::Tier(Box::new(job)),
                "the stage's tier-2 steps".to_string(),
            )
        }
    };
    let op = super::super::next_op(run);
    run.full_op = Some(op);
    if let Some(r) = record_mut(run, n, i) {
        r.probe = Some(op);
    }
    super::super::emit_op(run, op, None, kind, fx);
    let text = format!(
        "stage {n}: reproducing CI red at {} with {shown}",
        sha7(&head)
    );
    log(run, now, text);
}

fn test_at(run: &Run, head: &str, commands: Vec<String>) -> OpKind {
    let dir = run.full_path();
    OpKind::TestAt(Box::new(TestAtSpec {
        root: run.root.clone(),
        env: profile_env(&run.profile, &dir),
        dir,
        commit: head.to_string(),
        setup: run.profile.setup.clone(),
        commands,
        timeout_secs: run.profile.check_timeout_secs,
    }))
}

/// Whether `op` is a CI reproduction (`results.rs` routes its answer here).
pub(crate) fn reproducing(run: &Run, op: OpId) -> bool {
    run.delivery
        .stages
        .iter()
        .any(|s| s.ci.iter().any(|r| r.probe == Some(op)))
}

/// A reproduction's answer (step 4): red with names on a tiered profile is bisected;
/// any other red is a stage fix; green is the environment fix.
pub(crate) fn reproduced(
    run: &mut Run,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let found = (1..=stage_count(run)).find_map(|n| {
        let s = run.delivery.stage(n)?;
        Some((n, s.ci.iter().position(|r| r.probe == Some(op))?))
    });
    let Some((n, i)) = found else {
        return;
    };
    if run.full_op == Some(op) {
        run.full_op = None;
    }
    let Some(r) = record_mut(run, n, i) else {
        return;
    };
    r.probe = None;
    let rec = r.clone();
    // Fix round 1: a red that stopped mattering, or a run ending, adds nothing.
    if let Some(why) = gone(run, n, &rec) {
        return drop_record(run, n, i, &why, now);
    }
    let (red, command) = match result {
        OpResult::TestAt { red, failing, .. } => (red, failing.join("; ")),
        OpResult::Tier(outcome) => {
            let step = outcome.steps.iter().find(|s| !s.ok);
            (
                !outcome.ok,
                step.map(|s| s.command.clone()).unwrap_or_default(),
            )
        }
        OpResult::SetupFailed { output } => {
            return probe_failed(
                run,
                n,
                i,
                &format!("setup failed: {}", first_line(&output)),
                now,
                fx,
            );
        }
        OpResult::Failed { message } => {
            return probe_failed(run, n, i, &first_line(&message), now, fx);
        }
        _ => return,
    };
    if !red {
        log(
            run,
            now,
            format!(
                "stage {n}: CI red at {} does not reproduce locally",
                sha7(&rec.head)
            ),
        );
        return fix::add_stage(run, n, i, Repro::NotReproduced, now, fx);
    }
    if let Some(r) = record_mut(run, n, i) {
        r.command = Some(command.clone());
    }
    let names = matches!(plan(run, &rec), Plan::Tests(_));
    if !(names && tiers::tiered(run)) {
        return fix::add_stage(run, n, i, Repro::Reproduced(command), now, fx);
    }
    let tests: Vec<String> = rec
        .failing_tests
        .iter()
        .take(RETRY_NAMES_MAX)
        .cloned()
        .collect();
    match bisect::start(run, n, &rec.head, tests, now, fx) {
        Ok(k) => {
            if let Some(b) = run
                .stages
                .iter_mut()
                .find(|s| s.n == n)
                .and_then(|s| s.bisect.as_mut())
            {
                b.ci = Some((n, rec.key.clone()));
            }
            if let Some(r) = record_mut(run, n, i) {
                r.phase = CiPhase::Bisecting;
            }
            let text = format!(
                "stage {n}: CI red at {} reproduces; bisecting {k} merges",
                sha7(&rec.head)
            );
            log(run, now, text);
        }
        Err(why) => {
            let text = format!(
                "stage {n}: CI red at {} reproduces; not bisected: {why}",
                sha7(&rec.head)
            );
            log(run, now, text);
            fix::add_stage(run, n, i, Repro::Reproduced(command), now, fx);
        }
    }
}

fn first_line(text: &str) -> String {
    let line = proto::safe_text::one_line(text.lines().next().unwrap_or_default());
    line.chars().take(200).collect()
}

/// The executor could not run the reproduction: tried again after a wait, and after
/// [`PROBE_TRIES`] the red is a stage fix that says it was not reproduced.
fn probe_failed(run: &mut Run, n: u16, i: usize, why: &str, now: u64, fx: &mut Vec<Effect>) {
    log(
        run,
        now,
        format!("stage {n}: could not reproduce CI red: {why}"),
    );
    let Some(r) = record_mut(run, n, i) else {
        return;
    };
    r.probe_failures = r.probe_failures.saturating_add(1);
    if r.probe_failures >= PROBE_TRIES {
        let reason = format!("the local run failed: {why}");
        return fix::add_stage(run, n, i, Repro::Unavailable(reason), now, fx);
    }
    let k = usize::from(r.probe_failures - 1).min(PROBE_BACKOFF.len() - 1);
    r.retry_at = now.saturating_add(PROBE_BACKOFF[k]);
}
