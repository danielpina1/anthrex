//! Milestone 9.1 decision 55: each stage of a run as a client sees it (`StageInfo`),
//! read by the orchestrator's digest (decision 58) and by the snapshot. Pure: it reads
//! the model only.

use proto::{FullInfo, FullState, StageInfo, TaskOrigin, TaskState};

use super::engine::OpKind;
use super::model::{Run, StageRecord};

/// A stage's failing tests shown, the first ones.
const FAILING_SHOWN: usize = 20;

/// One entry per stage from 1 to the highest the plan names or the run created, in
/// order. A stage not created yet has no `head` and no tier 3.
pub(crate) fn stage_infos(run: &Run) -> Vec<StageInfo> {
    let planned = run.tasks.iter().map(|t| t.stage()).max().unwrap_or(1);
    let created = run.stages.iter().map(|s| s.n).max().unwrap_or(0);
    (1..=planned.max(created).max(1))
        .map(|n| stage_info(run, n))
        .collect()
}

fn stage_info(run: &Run, n: u16) -> StageInfo {
    let of_stage = || run.tasks.iter().filter(move |t| t.stage() == n);
    let record = run.stage(n);
    let head = run.stage_head(n).map(str::to_string);
    StageInfo {
        n,
        branch: run.stage_branch(n),
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
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Decisions 17–19 and 35–38 on stage `s` at `head`: bisecting, a tier-3 job running,
/// green or red on the head itself, or nothing yet for it. The last job's figures
/// stay whatever commit they were about (`commit` says which).
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
        FullState::None
    };
    let last = s.full.last.as_ref();
    FullInfo {
        state,
        at: last.map(|l| l.at),
        secs: last.map(|l| l.secs),
        commit: last.map(|l| l.commit.clone()).filter(|c| !c.is_empty()),
        shards: last.map_or(0, |_| run.profile.tiers.full_shards.max(1)),
        flaky: last.map(|l| l.flaky.clone()).unwrap_or_default(),
        failing: last
            .map(|l| l.failing.iter().take(FAILING_SHOWN).cloned().collect())
            .unwrap_or_default(),
        bisect_fixes: s.full.bisect_fixes,
        note: s.full.note.clone(),
    }
}
