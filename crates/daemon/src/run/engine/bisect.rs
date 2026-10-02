//! Milestone 9.1 decisions 35–38 (task M9.1.15): a red tier 3 bisected over the
//! stage's merges with only the failing tests, and the merge that turned it red given
//! an engine-made fix task one rung up. A pure state machine over `OpKind::TestAt`
//! probes: first the last green commit `G`, then the red head `H`, then a binary
//! search over the merges between them (`StageRecord.merges`), with `G` green and `H`
//! red as its ends. A probe only moves the search: it never marks a commit green or
//! red, and nothing is cached from it (ruling C-7). An executor failure of a probe is
//! never red (ruling C-18): it is issued again after a backoff, and the bisect ends
//! without a culprit after the third. Pure (design decision 2).

use super::history::BisectResult;
use super::requests::log;
use super::{Effect, OpId, OpKind, OpResult, deciders, emit_op, fixes, full, next_op, wake};
use crate::decider::{CheckSummaryInput, DeciderRequest};
use crate::run::contract::{
    BisectFix, bisect_fix_acceptance, bisect_fix_brief, bisect_fix_title, sha7,
};
use crate::run::env::profile_env;
use crate::run::model::{BisectRecord, FixOf, Probe, Run, StageMerge, StageRecord};
use crate::run::proof::proof_command;
use crate::run::roster::escalate;
use crate::run::tiers::{RETRY_NAMES_MAX, TestAtSpec, TierOutcome};
use proto::{Route, RouteSpec, TaskOrigin, TestMode};

/// Decision 37's reason for a bisect fix task's `check` test mode.
pub(crate) const FIX_TEST_MODE_REASON: &str = "the failing tests already exist; they must pass";
/// Ruling C-18 for probes: the waits after the executor's first and second failures of
/// one probe; the third ends the bisect without a culprit.
const PROBE_BACKOFF: [u64; 2] = [30, 120];
const PROBE_FAILURES_MAX: u8 = 3;
/// Decision 38's wake notes name at most this many tests.
const WAKE_TESTS: usize = 3;

#[path = "bisect_end.rs"]
mod ended;
#[cfg(test)]
pub(crate) use ended::REBASELINED;
pub(super) use ended::rebaselined;
use ended::{end, end_with, record};

fn commit_of(m: &StageMerge) -> &str {
    match m {
        StageMerge::Task { commit, .. } | StageMerge::Propagate { commit, .. } => commit,
    }
}

fn first(tests: &[String], k: usize) -> String {
    let shown: Vec<&str> = tests.iter().take(k).map(String::as_str).collect();
    shown.join(", ")
}

/// Decision 36's range: `G`, the stage's last green tier 3 when it is on the stage's
/// line (else its floor, ruling C-28 (1), else its creation point), and the merges
/// after it up to `head`, in order. `None` when `head` is not one of those merges.
fn range(s: &StageRecord, head: &str) -> Option<(String, Vec<StageMerge>)> {
    let green = s.full.green_at.as_deref().and_then(|g| {
        s.merges
            .iter()
            .position(|m| commit_of(m) == g)
            .map(|k| (g, k + 1))
    });
    let floor = s.floor.as_deref().unwrap_or(s.created_from.as_str());
    let (base, from) = green.unwrap_or((floor, 0));
    let to = s.merges.iter().rposition(|m| commit_of(m) == head)?;
    (to >= from).then(|| (base.to_string(), s.merges[from..=to].to_vec()))
}

/// Ruling C-19: stage `n` has a bisect fix task that has not finished (not merged,
/// cancelled or reported); its stage then starts no idle tier 3 and no bisect.
pub(super) fn fix_open(run: &Run, n: u16) -> Option<&str> {
    run.tasks
        .iter()
        .find(|t| {
            matches!(&t.fixes, Some(FixOf::Bisect { stage, .. }) if *stage == n)
                && !t.state.is_finished()
        })
        .map(|t| t.id())
}

/// Whether a stage of the run is being bisected.
pub(super) fn bisecting(run: &Run) -> bool {
    run.stages.iter().any(|s| s.bisect.is_some())
}

/// Decision 35: sets up stage `stage`'s bisect of its red commit `head` on `tests`
/// and issues its first probe (`G`). The caller made decision 35's checks (the cap,
/// failing names, `single_test`), so 9.2 can bisect a CI failure through here too.
/// `Ok(<merges in the range>)` once started; `Err(<the no-culprit reason>)` when it
/// cannot start, with nothing changed.
pub(super) fn start(
    run: &mut Run,
    stage: u16,
    head: &str,
    tests: Vec<String>,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<usize, String> {
    if run.full_op.is_some() || bisecting(run) {
        return Err("a tier-3 job or a bisect is in flight".to_string());
    }
    if run.profile.single_test.is_none() {
        return Err("the profile has no single_test to bisect with".to_string());
    }
    if tests.is_empty() {
        return Err("no failing test names to bisect with".to_string());
    }
    let Some(s) = run.stage(stage) else {
        return Err(format!("stage {stage} is not created"));
    };
    let Some((base, candidates)) = range(s, head) else {
        // Ruling C-28 (1): red at the floor itself, with no merge after it; the line
        // below the floor is not the engine's.
        if s.floor.as_deref() == Some(head) {
            return Err(format!(
                "red before the rebaselined head {}; not bisected",
                sha7(head)
            ));
        }
        return Err(format!(
            "{} is not a merge recorded on the stage's line",
            sha7(head)
        ));
    };
    let m = candidates.len();
    let record = BisectRecord {
        head: head.to_string(),
        tests,
        base,
        candidates,
        lo: 0,
        hi: m,
        probe: None,
        probes: 0,
        summary: None,
        summary_decider: None,
        show: None,
        infra: 0,
        retry_at: 0,
        ci: None,
    };
    if let Some(s) = stage_mut(run, stage) {
        s.bisect = Some(record);
    }
    issue(run, stage, now, fx);
    Ok(m)
}

/// Decision 37: the ≤ 40-line summary of the tier-3 job's red step for the fix task's
/// brief. Its fallback (M8a's last 40 lines) is kept at once; M8b's check-summary
/// decider, queued as for a red check, replaces it when it answers first.
pub(super) fn summarise(run: &mut Run, stage: u16, outcome: &TierOutcome, now: u64) -> Vec<Effect> {
    let step = outcome.steps.iter().find(|s| !s.ok);
    let label = format!("stage-{stage}");
    let input = CheckSummaryInput {
        task_id: label.clone(),
        command: step.map(|s| s.command.clone()).unwrap_or_default(),
        code: step.and_then(|s| s.code),
        timed_out: step.is_some_and(|s| s.timed_out),
        tail: outcome.tail.clone(),
    };
    let fallback = crate::run::exec::summary(&input.tail);
    let id = deciders::queue(run, vec![label], DeciderRequest::CheckSummary(input), now);
    if let Some(b) = stage_mut(run, stage).and_then(|s| s.bisect.as_mut()) {
        b.summary = Some(fallback);
        b.summary_decider = Some(id);
    }
    deciders::dispatch(run, now)
}

/// A check-summary decider's `lines` for a bisect's summary: `true` when decider
/// `decider_id` was a bisect's (and the summary is now its answer).
pub(super) fn summarised(run: &mut Run, decider_id: u64, lines: &[String]) -> bool {
    let bisect = run
        .stages
        .iter_mut()
        .filter_map(|s| s.bisect.as_mut())
        .find(|b| b.summary_decider == Some(decider_id));
    let Some(b) = bisect else {
        return false;
    };
    b.summary = Some(lines.join("\n"));
    b.summary_decider = None;
    true
}

fn stage_mut(run: &mut Run, n: u16) -> Option<&mut StageRecord> {
    run.stages.iter_mut().find(|s| s.n == n)
}

/// The probe due next: `G`, then `H`, then the middle of the search.
fn due(b: &BisectRecord) -> Probe {
    match b.probes {
        0 => Probe::Base,
        1 => Probe::Head,
        _ => Probe::Mid((b.lo + b.hi) / 2),
    }
}

fn probe_commit(b: &BisectRecord, probe: Probe) -> String {
    match probe {
        Probe::Base => b.base.clone(),
        Probe::Head => b.head.clone(),
        Probe::Mid(p) => b
            .candidates
            .get(p.wrapping_sub(1))
            .map(commit_of)
            .unwrap_or_default()
            .to_string(),
    }
}

/// Decision 36: the due probe of stage `n` as `TestAt` in `.full`, at `FullStage` on
/// half the slots (the executor's), running `single_test` once per failing name (the
/// first ten). It holds `Run.full_op`. Only while the run runs.
fn issue(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    // Review of c21b745: a paused or halted run keeps its result; the next running
    // pass (`pass`) issues the probe.
    if run.state != proto::RunState::Running {
        return;
    }
    let Some(b) = run.stage(n).and_then(|s| s.bisect.as_ref()) else {
        return;
    };
    let Some(single) = run.profile.single_test.clone() else {
        return;
    };
    let probe = due(b);
    let commit = probe_commit(b, probe);
    let commands = b
        .tests
        .iter()
        .take(RETRY_NAMES_MAX)
        .map(|t| proof_command(&single, t))
        .collect();
    let dir = run.full_path();
    let spec = TestAtSpec {
        root: run.root.clone(),
        env: profile_env(&run.profile, &dir),
        dir,
        commit: commit.clone(),
        setup: run.profile.setup.clone(),
        commands,
        timeout_secs: run.profile.check_timeout_secs,
    };
    let op = next_op(run);
    run.full_op = Some(op);
    if let Some(b) = stage_mut(run, n).and_then(|s| s.bisect.as_mut()) {
        b.probe = Some((op, probe));
    }
    emit_op(run, op, None, OpKind::TestAt(Box::new(spec)), fx);
    log(
        run,
        now,
        format!("stage {n}: bisect probe at {}", sha7(&commit)),
    );
}

/// Every scheduler pass of a running run: a bisect whose probe was lost in a restart
/// (decision 29) or waits out an executor failure's backoff is issued again; a run
/// ending anyway (the `finish` edit, `run cancel`) ends its bisects.
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let idle: Vec<(u16, u64)> = run
        .stages
        .iter()
        .filter_map(|s| s.bisect.as_ref().map(|b| (s.n, b)))
        .filter(|(_, b)| b.probe.is_none())
        .map(|(n, b)| (n, b.retry_at))
        .collect();
    for (n, retry_at) in idle {
        if full::ending(run) {
            end(
                run,
                n,
                "the run ended before the bisect did".to_string(),
                now,
                fx,
            );
        } else if now >= retry_at {
            issue(run, n, now, fx);
        }
    }
}

/// Decision 29: a probe lost in a restart. `true` when `op` was one; the next running
/// pass issues it again.
pub(super) fn lost(run: &mut Run, op: OpId) -> bool {
    let bisect = run
        .stages
        .iter_mut()
        .filter_map(|s| s.bisect.as_mut())
        .find(|b| b.probe.is_some_and(|(p, _)| p == op));
    let Some(b) = bisect else {
        return false;
    };
    b.probe = None;
    if run.full_op == Some(op) {
        run.full_op = None;
    }
    true
}

/// A probe's result (decision 36).
pub(super) fn probe_done(
    run: &mut Run,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let found = run.stages.iter().find_map(|s| {
        let (p, probe) = s.bisect.as_ref()?.probe?;
        (p == op).then_some((s.n, probe))
    });
    let Some((n, probe)) = found else {
        return;
    };
    if run.full_op == Some(op) {
        run.full_op = None;
    }
    let Some(b) = stage_mut(run, n).and_then(|s| s.bisect.as_mut()) else {
        return;
    };
    b.probe = None;
    let at = probe_commit(b, probe);
    if full::ending(run) {
        return end(
            run,
            n,
            "the run ended before the bisect did".to_string(),
            now,
            fx,
        );
    }
    match result {
        OpResult::TestAt { red, show, .. } => {
            let Some(b) = stage_mut(run, n).and_then(|s| s.bisect.as_mut()) else {
                return;
            };
            b.probes += 1;
            b.infra = 0;
            let verdict = if red { "red" } else { "green" };
            log(
                run,
                now,
                format!("stage {n}: probe at {} {verdict}", sha7(&at)),
            );
            probed(run, n, probe, red, show, now, fx);
        }
        OpResult::SetupFailed { output } => {
            let line = full::first_line(&output);
            end(
                run,
                n,
                format!("could not probe {}: setup failed: {line}", sha7(&at)),
                now,
                fx,
            );
        }
        OpResult::Failed { message } => {
            log(
                run,
                now,
                format!("stage {n}: could not probe {}: {message}", sha7(&at)),
            );
            let Some(b) = stage_mut(run, n).and_then(|s| s.bisect.as_mut()) else {
                return;
            };
            b.infra = b.infra.saturating_add(1);
            if b.infra >= PROBE_FAILURES_MAX {
                let line = full::first_line(&message);
                return end(
                    run,
                    n,
                    format!("could not probe {}: {line}", sha7(&at)),
                    now,
                    fx,
                );
            }
            let wait = PROBE_BACKOFF[usize::from(b.infra - 1).min(PROBE_BACKOFF.len() - 1)];
            b.retry_at = now.saturating_add(wait);
        }
        _ => {}
    }
}

/// Decision 36's order: `G` red or `H` green ends without a culprit; otherwise the
/// search narrows until one merge is left.
fn probed(
    run: &mut Run,
    n: u16,
    probe: Probe,
    red: bool,
    show: Option<String>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let floor = run.stage(n).and_then(|s| s.floor.clone());
    let Some(b) = stage_mut(run, n).and_then(|s| s.bisect.as_mut()) else {
        return;
    };
    match (probe, red) {
        (Probe::Base, true) => {
            let floored = floor.as_ref() == Some(&b.base);
            let reason = if floored {
                // Ruling C-28 (1): the line below the floor is not the engine's.
                format!(
                    "red before the rebaselined head {}; not bisected",
                    sha7(&b.base)
                )
            } else {
                format!("the failing tests already fail at {}", sha7(&b.base))
            };
            return end(run, n, reason, now, fx);
        }
        (Probe::Head, false) => {
            let reason = format!(
                "the failing tests pass alone at {}; they fail only with the whole suite",
                sha7(&b.head)
            );
            return end(run, n, reason, now, fx);
        }
        (Probe::Base, false) => {}
        (Probe::Head, true) => b.show = show,
        (Probe::Mid(p), true) => {
            b.hi = p;
            b.show = show;
        }
        (Probe::Mid(p), false) => b.lo = p,
    }
    if b.probes >= 2 && b.hi.saturating_sub(b.lo) <= 1 {
        return culprit(run, n, now, fx);
    }
    issue(run, n, now, fx);
}

/// Decision 36's culprit, the first red merge: a task's gets a fix task (decision 37);
/// a propagate's is no single culprit.
fn culprit(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    let Some(b) = run.stage(n).and_then(|s| s.bisect.clone()) else {
        return;
    };
    let entry = b.hi.checked_sub(1).and_then(|k| b.candidates.get(k));
    let id = match entry {
        Some(StageMerge::Task { id, .. }) => id.clone(),
        // Milestone 9.2 decision 33: `from: 0` is a base sync, the base branch's merge.
        Some(StageMerge::Propagate { from, .. }) => {
            let what = match from {
                0 => "the merge of the base branch".to_string(),
                k => format!("the propagate of stage {k}"),
            };
            return end(run, n, format!("the first red merge is {what}"), now, fx);
        }
        None => return end(run, n, "no merge is left to blame".to_string(), now, fx),
    };
    // Milestone 9.2 decision 27: a CI red's culprit gets a `ci` fix (`ci_fix_max`, not ours).
    if b.ci.is_some() {
        let handled = super::delivery::ci_culprit(run, n, &b, &id, now, fx);
        if let Some(s) = stage_mut(run, n) {
            s.bisect = None;
        }
        return record(run, n, &b, handled.result(&id), now, fx);
    }
    let added = add_fix(run, n, &b, &id, now, fx);
    if let Some(s) = stage_mut(run, n) {
        s.bisect = None;
    }
    let fix = match added {
        Ok(fix) => fix,
        Err(message) => {
            let reason = format!("fix task refused: {message}");
            let refused = BisectResult::Refused {
                task: &id,
                reason: &reason,
            };
            record(run, n, &b, refused, now, fx);
            return end_with(run, n, &b, reason, now, fx);
        }
    };
    let found = BisectResult::Culprit {
        task: &id,
        fix: &fix,
    };
    record(run, n, &b, found, now, fx);
    if let Some(s) = stage_mut(run, n) {
        s.full.bisect_fixes = s.full.bisect_fixes.saturating_add(1);
    }
    log(
        run,
        now,
        format!(
            "stage {n}: bisected to {id} in {} probes; added fix task {fix}",
            b.probes
        ),
    );
    wake::note(
        run,
        format!(
            "stage {n} tier 3 red ({}): bisected to {id}; added fix task {fix}",
            first(&b.tests, WAKE_TESTS)
        ),
    );
}

/// Decision 37: the culprit's fix task on its route one rung up, else on its own route.
fn add_fix(
    run: &mut Run,
    n: u16,
    b: &BisectRecord,
    culprit: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let Some(task) = run.task(culprit).cloned() else {
        return Err(format!("{culprit} is not a task"));
    };
    let id = fixes::next_fix_id(run);
    let brief = bisect_fix_brief(&BisectFix {
        id: &id,
        stage: n,
        culprit,
        culprit_title: &task.spec.title,
        culprit_brief: &task.spec.brief,
        tests: &b.tests,
        summary: b.summary.as_deref().unwrap_or_default(),
        show: b.show.as_deref().unwrap_or("(not available)"),
    });
    let spec = |route: &Route| fixes::FixSpec {
        origin: TaskOrigin::Bisect,
        fixes: FixOf::Bisect {
            culprit: culprit.to_string(),
            stage: n,
            tests: b.tests.clone(),
        },
        stage: n,
        title: bisect_fix_title(&b.tests, culprit),
        brief: brief.clone(),
        acceptance: bisect_fix_acceptance(&b.tests),
        owns: task.spec.owns.clone(),
        size: task.size,
        epic: task.spec.epic.clone(),
        route: RouteSpec {
            runtime: Some(route.runtime),
            model: Some(route.model.clone()),
            strength: Some(route.strength),
            effort: Some(route.effort),
        },
        test_mode: TestMode::Check,
        test_mode_reason: Some(FIX_TEST_MODE_REASON.to_string()),
        sync: None,
    };
    let up = escalate(&run.roster, &task.route);
    match fixes::add_fix(run, spec(&up), now, fx) {
        Ok(id) => Ok(id),
        Err(_) if up != task.route => fixes::add_fix(run, spec(&task.route), now, fx),
        Err(message) => Err(message),
    }
}
