//! Milestone 9.2 task M9.2.11, decisions 33 and 34: the base branch moves while stage
//! PRs are open. A conflict on the lowest open PR (`on_conflict`), or any view of it
//! (`always`), fetches the base into anthrex's own ref; a base that moved is merged
//! into that stage only, as 9.1's `Propagate` with `from: 0` (two parents, never a
//! rebase), and the stages above get it by propagate. A conflict is 9.1's `sync` fix
//! task with `FixOf::Base`, its worktree holding the conflicted merge before its first
//! session; a base sync lost in a restart is a base sync again (ruling R-7).

use proto::{AgentRole, HoldKind, RouteSpec, Size, TaskOrigin, TestMode};

use super::bisect::with_orchestrator;
use super::control_restore::restart;
use super::delivery_open::{answer, host_ops, host_ops_in};
use super::delivery_watch::{PR, poll_with, view, watched};
use super::delivery_watch_adopt::{poll_stage, two_stages, view_of};
use super::fixture::*;
use super::merge::{commit, pending_one, window_of};
use super::propagate::{TREE, merged_at, propagates};
use super::wake_notes::notes;
use crate::host::{FetchOutcome, Mergeable, PrView};
use crate::run::delivery::SyncPolicy;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::{FixOf, OpId, PropagateSpec, StageMerge, SyncState};

/// `refs/anthrex/<run>/remote/base`.
pub(super) fn base_ref() -> String {
    format!("refs/anthrex/{RUN_ID}/remote/base")
}

/// The base fetch decision 33 makes due, asking for `parents_of`'s parents.
pub(super) fn base_fetch_op(parents_of: Option<String>) -> HostOp {
    HostOp::Fetch {
        stage: None,
        branch: "main".into(),
        into: base_ref(),
        adopt: None,
        parents_of,
    }
}

/// The pending base fetch.
pub(super) fn base_fetch(fx: &Fixture) -> (OpId, HostOp) {
    let ops: Vec<_> = (host_ops(fx).into_iter())
        .filter(|(_, op)| matches!(op, HostOp::Fetch { stage: None, .. }))
        .collect();
    assert_eq!(ops.len(), 1, "one base fetch: {:#?}", host_ops(fx));
    ops[0].clone()
}

pub(super) fn fetching(fx: &Fixture) -> bool {
    (host_ops(fx).iter()).any(|(_, op)| matches!(op, HostOp::Fetch { stage: None, .. }))
}

/// Answers the pending base fetch: the remote base is at `sha`.
pub(super) fn fetched(fx: &mut Fixture, sha: &str, parents: Option<u32>) -> Vec<Effect> {
    let (op, _) = base_fetch(fx);
    let outcome = FetchOutcome::Fetched {
        sha: sha.into(),
        parents,
    };
    answer(fx, op, HostResult::Fetched(outcome))
}

/// The pending base syncs (`Propagate` with `from: 0`).
pub(super) fn base_syncs(fx: &Fixture) -> Vec<(OpId, PropagateSpec)> {
    propagates(fx)
        .into_iter()
        .filter(|(_, s)| s.from == 0)
        .collect()
}

pub(super) fn base_sync(fx: &Fixture) -> (OpId, PropagateSpec) {
    let all = base_syncs(fx);
    assert_eq!(all.len(), 1, "one base sync: {all:#?}");
    all[0].clone()
}

/// A view of stage PR `number` at `head` that GitHub finds conflicting.
pub(super) fn conflicting(number: u64, head: &str) -> PrView {
    PrView {
        mergeable: Mergeable::Conflicting,
        ..view_of(number, head)
    }
}

fn head(fx: &Fixture, n: u16) -> String {
    fx.run().stage_head(n).unwrap().to_string()
}

fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

#[test]
fn on_conflict_syncs_the_lowest_open_stage() {
    let (mut fx, _) = two_stages(true);
    assert_eq!(fx.run().delivery.limits.sync, SyncPolicy::OnConflict);
    let h1 = head(&fx, 1);
    // A mergeable view fetches nothing under `on_conflict`.
    let effects = poll_stage(&mut fx, 1, view_of(11, &h1));
    assert!(!fetching(&fx), "{:#?}", host_ops_in(&effects));
    // A conflicting view of the lowest open PR fetches the base into anthrex's own ref.
    let effects = poll_stage(&mut fx, 1, conflicting(11, &h1));
    assert!(host_ops_in(&effects).contains(&base_fetch_op(None)));
    // The base moved: it is merged into stage 1, in the same step.
    let effects = fetched(&mut fx, &commit(50), None);
    let (_, spec) = base_sync(&fx);
    assert_eq!((spec.from, spec.to), (0, 1));
    assert_eq!(spec.from_head, commit(50));
    assert_eq!(ops_in(&effects, "Propagate").len(), 1, "{effects:#?}");
    assert!(logged(
        &fx,
        "stage 1: main moved to 50eeeee; merging it into stage 1"
    ));
    assert!(fx.run().delivery.base_sync_due.is_empty(), "taken");
    // A base the stages already hold syncs nothing.
    let (op, _) = base_sync(&fx);
    fx.done(op, OpResult::AlreadyHeld);
    assert_eq!(
        fx.run().delivery.base_synced.as_deref(),
        Some(commit(50).as_str())
    );
    poll_stage(&mut fx, 1, conflicting(11, &h1));
    fetched(&mut fx, &commit(50), None);
    assert!(base_syncs(&fx).is_empty());
    assert!(fx.run().delivery.base_sync_due.is_empty());
}

#[test]
fn always_syncs_when_the_base_moves() {
    let (mut fx, _) = two_stages(true);
    fx.run_mut().delivery.limits.sync = SyncPolicy::Always;
    let h1 = head(&fx, 1);
    // Every view of the lowest open PR fetches the base, conflicting or not.
    poll_stage(&mut fx, 1, view_of(11, &h1));
    assert_eq!(base_fetch(&fx).1, base_fetch_op(None));
    // Unmoved: nothing to merge.
    fetched(&mut fx, BASE, None);
    assert!(base_syncs(&fx).is_empty());
    assert!(fx.run().delivery.base_sync_due.is_empty());
    // Moved: merged into stage 1.
    poll_stage(&mut fx, 1, view_of(11, &h1));
    fetched(&mut fx, &commit(50), None);
    let (_, spec) = base_sync(&fx);
    assert_eq!((spec.to, spec.from_head.as_str()), (1, commit(50).as_str()));
}

#[test]
fn never_syncs_a_stage_above_the_lowest_open_one_directly() {
    let (mut fx, _) = two_stages(true);
    fx.run_mut().delivery.limits.sync = SyncPolicy::Always;
    let (h1, h2) = (head(&fx, 1), head(&fx, 2));
    // Stage 2's PR is not the lowest open one: its views fetch nothing, even conflicting.
    poll_stage(&mut fx, 2, conflicting(12, &h2));
    assert!(!fetching(&fx));
    poll_stage(&mut fx, 1, view_of(11, &h1));
    fetched(&mut fx, &commit(50), None);
    let (op, spec) = base_sync(&fx);
    assert_eq!(spec.to, 1, "the lowest open stage only");
    assert!(!fx.run().delivery.base_sync_due.contains_key(&2));
    // Merged into stage 1, it arrives in stage 2 by 9.1's propagate.
    fx.done(op, merged_at(&commit(51)));
    assert_eq!(head(&fx, 1), commit(51));
    let stage = fx.run().stage(1).unwrap();
    assert_eq!(
        stage.merges.last(),
        Some(&StageMerge::Propagate {
            from: 0,
            commit: commit(51)
        })
    );
    let ups: Vec<_> = propagates(&fx).into_iter().map(|(_, s)| s).collect();
    assert_eq!(ups.len(), 1, "{ups:#?}");
    assert_eq!((ups[0].from, ups[0].to), (1, 2));
    assert_eq!(ups[0].from_head, commit(51));
    assert!(base_syncs(&fx).is_empty());
}

#[test]
fn base_sync_is_a_two_parent_merge_never_a_rebase() {
    // A `Single` run takes its own emit path (ruling R-7): 9.1's propagate never runs
    // for it.
    let mut fx = watched();
    let at = head(&fx, 1);
    poll_with(&mut fx, conflicting(PR, &at));
    fetched(&mut fx, &commit(50), None);
    let (op, spec) = base_sync(&fx);
    let run = fx.run();
    assert_eq!(
        spec,
        PropagateSpec {
            root: run.root.clone(),
            integration: run.integration_path(),
            from: 0,
            to: 1,
            from_head: commit(50),
            to_branch: run.run_branch(),
            expected_to_head: at.clone(),
            also_integration: false,
            base_branch: "main".into(),
            expected_base: BASE.into(),
            guarded: crate::run::engine::stages::guard_list(run),
            message: "anthrex: merge main@50eeeee into stage-1".into(),
            tier: None,
            check: Some("cargo test".into()),
            timeout_secs: run.profile.check_timeout_secs,
            env: spec.env.clone(),
            tasks: Default::default(),
        }
    );
    // `merge-tree` + `commit-tree` with parents `[stage head, base]` (9.1's executor,
    // tested in `driver/stage_ops_propagate_tests.rs`): the stage head moves forward,
    // and the PR is pushed fast-forward.
    fx.done(op, merged_at(&commit(51)));
    assert_eq!(fx.run().run_head, commit(51));
    assert_eq!(
        fx.run().delivery.base_synced.as_deref(),
        Some(commit(50).as_str())
    );
    assert!(logged(&fx, "stage 1: merged main@50eeeee (51eeeee)"));
    // The view that went out meanwhile, then the push.
    poll_with(&mut fx, view(&at));
    let push = HostOp::Push {
        stage: 1,
        sha: commit(51),
    };
    assert!(
        host_ops(&fx).iter().any(|(_, op)| *op == push),
        "{:#?}",
        host_ops(&fx)
    );
}

/// A `Single` run (with an orchestrator) whose base sync of `commit(50)` conflicted on
/// `files`.
fn base_conflicted(files: &[&str]) -> Fixture {
    let mut fx = watched();
    with_orchestrator(&mut fx);
    let at = head(&fx, 1);
    poll_with(&mut fx, conflicting(PR, &at));
    fetched(&mut fx, &commit(50), None);
    let (op, _) = base_sync(&fx);
    let files = files.iter().map(|f| f.to_string()).collect();
    fx.done(
        op,
        OpResult::Conflict {
            files,
            tree: Some(TREE.into()),
        },
    );
    fx
}

#[test]
fn base_conflict_adds_a_sync_task_through_the_hand_back() {
    let mut fx = base_conflicted(&["docs/t1/a.md"]);
    let at = commit(1);
    let fix = fx.task("fix1").clone();
    assert_eq!(fix.origin, TaskOrigin::Sync);
    assert_eq!(
        fix.fixes,
        Some(FixOf::Base {
            stage: 1,
            base_sha: commit(50)
        })
    );
    assert_eq!(
        fix.spec.owns,
        ["docs/t1/a.md"],
        "the conflicted files, exactly"
    );
    assert_eq!((fix.stage(), fix.size), (1, Size::M));
    assert_eq!(fix.test_mode, TestMode::Check);
    assert_eq!(fix.spec.route, RouteSpec::default());
    assert_eq!(fix.spec.priority, 100);
    assert!(fix.spec.deps.is_empty());
    assert_eq!(fix.spec.title, "Sync stage 1 with main");
    assert_eq!(
        fix.spec.acceptance,
        [
            "The merge is committed with both parents, every conflict is resolved, and the stage builds and passes tier 1."
        ]
    );
    assert_eq!(
        fix.spec.brief,
        "Merging main@50eeeee into stage 1's branch conflicted in: docs/t1/a.md.\n\
         The merge is in progress in your worktree, with the conflict markers in those files. Resolve them, commit the merge (keep both parents: do not rebase, do not reset), and call task_done.\n\
         Stage 1's pull request: https://github.com/fake/app/pull/7.\n\
         Your commits reach the pull request after tier 1, tier 2 and the merge queue; do not push, open or merge anything yourself."
    );
    assert_eq!(
        fix.sync,
        Some(SyncState {
            onto: commit(50),
            base_tree: TREE.into(),
            tasks: Default::default(),
            handed_back: false,
            to_head: at.clone(),
            handed: Vec::new(),
        })
    );
    assert_eq!(fix.orch.gate_hold, None, "inside the stage: no approval");
    assert_eq!(
        notes(&fx),
        ["stage 1 conflicts with main@50eeeee: sync task fix1 added"]
    );
    assert_eq!(fx.run().run_head, at, "nothing landed");
    // Its worktree starts at the stage head the conflict was found on, and before its
    // first session M8a's hand-back merges the base commit into it (never `git merge`
    // in the worktree, never a rebase).
    fx.tick();
    let (op, kind) = pending_one(&fx, "PrepareWorktree", Some("fix1"));
    let OpKind::PrepareWorktree { from, .. } = kind else {
        unreachable!()
    };
    assert_eq!(from, at);
    let effects = fx.done(op, OpResult::Worktree { head: at.clone() });
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    let (op, kind) = pending_one(&fx, "HandBack", Some("fix1"));
    assert_eq!(
        kind,
        OpKind::HandBack {
            worktree: super::dispatch::task_path("fix1"),
            run_head: commit(50),
            task_head: Some(at.clone()),
            list_merged: false,
        }
    );
    let effects = fx.done(op, handed(&at));
    assert_eq!(ops_in(&effects, "CreateWindow").len(), 1, "{effects:#?}");
    // Its display text (decision 41).
    let fixes = fx.task("fix1").fixes.clone().unwrap();
    let text = super::super::fixes::fix_text(fx.run(), &fixes);
    assert_eq!(text, "sync with main@50eeeee");
}

fn handed(at: &str) -> OpResult {
    OpResult::HandedBack {
        files: vec!["docs/t1/a.md".into()],
        head: Some(at.into()),
        onto: Some(at.into()),
        merged: Vec::new(),
        merged_total: 0,
    }
}

/// Pinning (9.1's `spill_base`, for a base sync): the sync task's claim is checked
/// against the conflicted tree, so the base's own changes are never its work; its
/// claim must keep the base commit; its reviewer judges only the resolution; once it
/// merges, the stages hold that base.
#[test]
fn sync_task_spill_is_measured_from_the_conflicted_tree() {
    let mut fx = base_conflicted(&["docs/t1/a.md"]);
    let at = commit(1);
    fx.tick();
    let (op, _) = pending_one(&fx, "PrepareWorktree", Some("fix1"));
    fx.done(op, OpResult::Worktree { head: at.clone() });
    let (op, _) = pending_one(&fx, "HandBack", Some("fix1"));
    fx.done(op, handed(&at));
    let windows = fx.launch_all();
    let window = window_of(&windows, "fix1");
    let args = serde_json::json!({"summary": "resolved"});
    let effects = fx.tool_as(AgentRole::Worker, window, "fix1", "task_done", args);
    let (_, kind) = ops_in(&effects, "VerifyDone")[0].clone();
    let OpKind::VerifyDone {
        spill_base, sync, ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(spill_base.as_deref(), Some(TREE));
    assert_eq!(sync.map(|s| s.onto), Some(commit(50)));
    // The ruling's `FixOf::Base` arms: the lost-merge refusal and the reviewer line
    // name the base commit, not a "stage 0".
    let task = fx.task("fix1").clone();
    assert_eq!(
        super::super::propagate::lost_merge(fx.run(), &task),
        "task_done rejected: sync task must keep the merge of main@50eeeee: its head does not contain 50eeeee"
    );
    let prompt = crate::run::contract::reviewer_prompt(fx.run(), &task, 1, BASE, HEAD, "", "");
    assert!(
        prompt.contains("This is a sync task: judge only how the conflicts were resolved; the changes main@50eeeee brought in are not this task's."),
        "{prompt}"
    );
    assert!(!prompt.contains("stage 0"), "{prompt}");
    // Merged: the stages hold commit(50); the next view's base is not merged again.
    super::kinds_integration::merge_real(&mut fx, "fix1", &commit(52));
    assert_eq!(
        fx.run().delivery.base_synced.as_deref(),
        Some(commit(50).as_str())
    );
}

/// Decision 26's approval rule, for a base sync: a conflicted file no task of the stage
/// owns waits in a fix hold.
#[test]
fn a_base_conflict_outside_the_stage_is_held() {
    let fx = base_conflicted(&["Cargo.toml"]);
    let fix = fx.task("fix1");
    assert_eq!(fix.orch.gate_hold.as_deref(), Some("hold-fix1"));
    let hold = (fx.run().orch.gate_holds.iter())
        .find(|h| h.id == "hold-fix1")
        .unwrap();
    assert_eq!(
        hold.kind,
        HoldKind::Fix {
            stage: 1,
            paths: vec!["Cargo.toml".into()]
        }
    );
}

/// Ruling R-7: a base sync that a restart lost is a base sync again (back into
/// `base_sync_due`), never a stage propagate; a red one is not tried again on the same
/// base and head.
#[test]
fn a_base_sync_lost_in_a_restart_is_a_base_sync_again() {
    let (mut fx, _) = two_stages(true);
    let h1 = head(&fx, 1);
    poll_stage(&mut fx, 1, conflicting(11, &h1));
    fetched(&mut fx, &commit(50), None);
    base_sync(&fx);
    restart(&mut fx, Vec::new());
    assert_eq!(
        fx.run().delivery.base_sync_due.get(&1),
        Some(&commit(50)),
        "back into base_sync_due"
    );
    assert!(fx.run().propagate_due.is_empty(), "not a stage propagate");
    super::control::resume(&mut fx);
    let (op, spec) = base_sync(&fx);
    assert_eq!((spec.from, spec.to), (0, 1));
    // Red: an attention line, and the same base on the same head is not merged again.
    let effects = fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(1),
            timed_out: false,
            tail: String::new(),
            secs: 1,
            tier: None,
        },
    );
    let line = "the merge of main@50eeeee into stage 1 is red: the check failed";
    assert!(fx.run().delivery.alerts.values().any(|l| l == line));
    assert!(ops_in(&effects, "Propagate").is_empty());
    poll_stage(&mut fx, 1, conflicting(11, &h1));
    fetched(&mut fx, &commit(50), None);
    assert!(base_syncs(&fx).is_empty());
    // A newer base clears it and is merged.
    poll_stage(&mut fx, 1, conflicting(11, &h1));
    fetched(&mut fx, &commit(53), None);
    assert_eq!(base_sync(&fx).1.from_head, commit(53));
    assert!(!fx.run().delivery.alerts.values().any(|l| l == line));
}

/// A base sync's merge is no single culprit for 9.1's bisect (decision 33), and the
/// end names it as the base branch's merge, never a "stage 0".
#[test]
fn a_bisect_names_a_base_sync_merge_as_the_base() {
    use super::bisect::{answer as probe, merged, red_full};
    use super::full::attention;
    let mut fx = merged(&["t1", "t2"], "");
    with_orchestrator(&mut fx);
    let stage = &mut fx.run_mut().stages[0];
    stage.merges.push(StageMerge::Propagate {
        from: 0,
        commit: commit(3),
    });
    crate::run::engine::stages::set_stage_head(fx.run_mut(), 1, &commit(3));
    fx.run_mut().last_green_candidate = Some(commit(3));
    red_full(&mut fx);
    probe(&mut fx, 3);
    let line = "tier 3 red, no single culprit: a::works (stage 1: the first red merge is the merge of the base branch)";
    assert!(
        attention(&fx).contains(&line.to_string()),
        "{:?}",
        attention(&fx)
    );
    assert!(fx.run().task("fix1").is_none());
}
