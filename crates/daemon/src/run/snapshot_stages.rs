//! Milestone 9.1 decision 55: each stage of a run as a client sees it (`StageInfo`),
//! read by the orchestrator's digest (decision 58) and by the snapshot. Pure: it reads
//! the model only.

use proto::{DeliveryMode, FullInfo, FullState, PrState, StageInfo, TaskOrigin, TaskState};

use super::engine::OpKind;
use super::model::{Run, StageLayout, StageRecord, task_branch};

/// A stage's failing and flaky tests shown, the first ones (ruling C-24 caps both).
const NAMES_SHOWN: usize = 20;

/// One entry per stage from 1 to the highest the plan names or the run created, in
/// order. A stage not created yet has no `head` and no tier 3, and shows the branch it
/// will be created on (ruling C-24).
pub(crate) fn stage_infos(run: &Run) -> Vec<StageInfo> {
    let planned = planned(run);
    (1..=stage_count(run))
        .map(|n| stage_info(run, n, planned))
        .collect()
}

/// The highest stage the plan names (1 for a plan without stages).
fn planned(run: &Run) -> u16 {
    run.tasks.iter().map(|t| t.stage()).max().unwrap_or(1)
}

/// The run's stages: the highest the plan names or the run created, at least 1. The
/// one count of stages: the snapshot's, the digest's and a PR title's `<N>`.
pub(crate) fn stage_count(run: &Run) -> u16 {
    let created = run.stages.iter().map(|s| s.n).max().unwrap_or(0);
    planned(run).max(created).max(1)
}

/// Stage `n`'s record, when the stage exists. Before approval a plan with more than
/// one stage has only the placeholder stage 1 on `integration` that every built run
/// gets (decision 47); its approval clears it and creates `stage-<n>` branches
/// (decision 46), so it is not the stage's.
fn created(run: &Run, n: u16, planned: u16) -> Option<&StageRecord> {
    let placeholder = run.stage_layout == StageLayout::Single && planned > 1;
    run.stage(n).filter(|_| !placeholder)
}

fn stage_info(run: &Run, n: u16, planned: u16) -> StageInfo {
    let of_stage = || run.tasks.iter().filter(move |t| t.stage() == n);
    let record = created(run, n, planned);
    let head = record.and_then(|_| run.stage_head(n)).map(str::to_string);
    let branch = match record {
        Some(_) => run.stage_branch(n),
        None if run.stage_layout == StageLayout::Multi || planned > 1 => {
            task_branch(&run.id, &format!("stage-{n}"))
        }
        None => run.run_branch(),
    };
    StageInfo {
        actions: Vec::new(),
        n,
        branch,
        tasks: count(of_stage().count()),
        merged: count(of_stage().filter(|t| t.state == TaskState::Merged).count()),
        full: record.map_or_else(FullInfo::default, |s| {
            full(run, s, head.as_deref().unwrap_or_default())
        }),
        fix_tasks: of_stage()
            .filter(|t| t.origin != TaskOrigin::Plan)
            .map(|t| t.id().to_string())
            .collect(),
        propagate_red: record.and_then(|s| s.propagate_red.clone()),
        head,
        pr: None,
        round: record.map_or_else(|| run.planned_stage_round(n), |s| s.round),
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Decisions 17–19 and 35–38 on stage `s` at `head`: bisecting, a tier-3 job running,
/// green or red on the head itself, an open PR's last verdict (milestone 9.7 decision
/// 12), or nothing yet for it. The last job's figures stay whatever commit they were
/// about (`commit` says which).
fn full(run: &Run, s: &StageRecord, head: &str) -> FullInfo {
    let running = run
        .pending_ops
        .values()
        .any(|p| matches!(&p.kind, OpKind::Tier(spec) if spec.tier == 3 && spec.stage == s.n));
    let on_head = |at: &Option<String>| at.as_deref() == Some(head);
    let state = if s.bisect.is_some() {
        FullState::Bisecting
    } else if running {
        FullState::Running
    } else if on_head(&s.full.red_at) {
        FullState::Red
    } else if on_head(&s.full.green_at) {
        FullState::Green
    } else {
        open_pr_verdict(run, s)
    };
    let last = s.full.last.as_ref();
    FullInfo {
        state,
        at: last.map(|l| l.at),
        secs: last.map(|l| l.secs),
        commit: last.map(|l| l.commit.clone()).filter(|c| !c.is_empty()),
        shards: last.map_or(0, |_| run.profile.tiers.full_shards.max(1)),
        flaky: last
            .map(|l| l.flaky.iter().take(NAMES_SHOWN).cloned().collect())
            .unwrap_or_default(),
        failing: last
            .map(|l| l.failing.iter().take(NAMES_SHOWN).cloned().collect())
            .unwrap_or_default(),
        bisect_fixes: s.full.bisect_fixes,
        note: s.full.note.clone(),
        // Milestone 9.5 decision 45: held after executor failures (ruling C-18).
        held: super::engine::infra_held(s),
    }
}

/// Milestone 9.7 decision 12 (DH §2.3, BR-13): a `pr`-mode stage whose PR is open shows
/// its last tier-3 job's verdict. That job ran on the head the PR opened with; CI
/// carries later heads. Local mode, and a stage before its PR opens, show nothing.
fn open_pr_verdict(run: &Run, s: &StageRecord) -> FullState {
    let open = run.delivery.mode == DeliveryMode::Pr
        && run
            .delivery
            .pr(s.n)
            .is_some_and(|p| p.state == PrState::Open);
    match s.full.last.as_ref() {
        Some(last) if open && last.ok => FullState::Green,
        Some(_) if open => FullState::Red,
        _ => FullState::None,
    }
}

#[cfg(test)]
#[path = "snapshot_stages_tests.rs"]
mod tests;
