//! Decisions 33 and 34 (task M9.2.11): the base branch moves while stage PRs are open.
//! A fetch of the remote base is due when a stage PR merges (always), when a view of
//! the lowest open PR reports a conflict (`sync = "on_conflict"`), or with every view
//! of it (`sync = "always"`); it lands in `refs/anthrex/<run>/remote/base`, never in a
//! branch. A base that moved past what the stages absorbed is then merged into the
//! lowest stage that is still delivering, and only that one: the stages above get it
//! by 9.1's propagate. The merge is 9.1's `Propagate` with `from: 0` and the base
//! commit in place of a lower stage's head (`merge-tree` + `commit-tree` with two
//! parents: never `git merge`, never a rebase), taken by 9.1's merge queue before any
//! propagate into the same stage (ruling R-7: its own emit path, which a `Single` run
//! uses too). A conflict becomes a `sync` fix task (9.1's, reused with `FixOf::Base`)
//! whose worktree holds the conflicted merge before its first session. Nothing here
//! ever moves the user's base branch or pushes to the remote one. Pure.

use proto::{RouteSpec, RunState, Size, TaskOrigin, TestMode};

use super::super::fixes::{FixSpec, add_fix};
use super::super::merge::halt;
use super::super::requests::log;
use super::super::{Effect, OpId, OpKind, OpResult, emit_op, next_op, stages, wake};
use super::watch::{stage_busy, stage_paused};
use super::{emit, pr, stage_mut};
use crate::host::allow::is_object_id;
use crate::host::{Contains, FetchOutcome, Mergeable};
use crate::run::contract::sha7;
use crate::run::delivery::ops::HostOp;
use crate::run::delivery::snapshot::stage_count;
use crate::run::delivery::{SyncPolicy, templates};
use crate::run::env::profile_env;
use crate::run::model::{FixOf, PropagateSpec, Run, StageLayout, StageMerge, SyncState};

/// What a red base sync names when its job gave no test names (as 9.1's propagate).
const NO_NAMES: &str = "the check failed";
/// A red base sync's attention line names at most this many tests.
const LINE_TESTS: usize = 5;

/// `refs/anthrex/<run>/remote/base`: where the base is fetched (decision 33).
pub(super) fn base_ref(run: &Run) -> String {
    format!("refs/anthrex/{}/remote/base", run.id)
}

/// The base commit the stages last absorbed: `RunDelivery.base_synced`, else the base
/// the run started from.
fn synced(run: &Run) -> &str {
    run.delivery.base_synced.as_deref().unwrap_or(&run.base_sha)
}

/// Stage `n` is still delivering: not skipped, and its PR (if any) neither merged nor
/// closed.
pub(super) fn live(run: &Run, n: u16) -> bool {
    let landed = run
        .delivery
        .pr(n)
        .is_some_and(|p| p.state != proto::PrState::Open);
    !landed && !run.delivery.stage(n).is_some_and(|s| s.skipped)
}

/// The one stage a base sync goes into: the lowest one still delivering that exists
/// ([`exists`]), unless a closed PR below it pauses it (decision 37).
pub(super) fn target(run: &Run) -> Option<u16> {
    let n = (1..=stage_count(run)).find(|&n| live(run, n) && exists(run, n))?;
    (!stage_paused(run, n)).then_some(n)
}

/// Stage `n` exists or is still to be created: it has a head or a PR, or one of its
/// tasks is unfinished. A cancelled round's stage that was never created has none of
/// these, and a base sync is never due into it (the final fix wave, review A C1).
pub(in crate::run::engine) fn exists(run: &Run, n: u16) -> bool {
    run.stage_head(n).is_some()
        || run.delivery.pr(n).is_some()
        || (run.tasks.iter()).any(|t| t.stage() == n && !t.state.is_finished())
}

/// An unfinished sync task of stage `n` (9.1's or a base sync's): the stage's next base
/// sync waits for it (decision 34).
fn sync_open(run: &Run, n: u16) -> bool {
    (run.tasks.iter()).any(|t| t.sync.is_some() && t.stage() == n && !t.state.is_finished())
}

/// An unfinished sync task of stage `n` is resolving the merge of base `sha` (fix
/// round 1, m1): its merge brings `sha` in, so it is not queued again.
fn resolving(run: &Run, n: u16, sha: &str) -> bool {
    (run.tasks.iter()).any(|t| {
        !t.state.is_finished()
            && matches!(&t.fixes, Some(FixOf::Base { stage, base_sha }) if *stage == n && base_sha == sha)
    })
}

/// A base sync of `sha` into stage `n` is in flight.
fn in_flight(run: &Run, n: u16, sha: &str) -> bool {
    (run.pending_ops.values()).any(
        |p| matches!(&p.kind, OpKind::Propagate(s) if s.from == 0 && s.to == n && s.from_head == sha),
    )
}

/// A base fetch is in flight (also read by `goal_rounds::awaits_base`, milestone 9.3).
pub(in crate::run::engine) fn fetching(run: &Run) -> bool {
    run.pending_ops.values().any(|p| {
        matches!(
            &p.kind,
            OpKind::Host {
                op: HostOp::Fetch { stage: None, .. },
                ..
            }
        )
    })
}

/// Decision 33's triggers at a view of stage `n`'s PR: when it is the lowest open one,
/// a conflict (`on_conflict`) or any view (`always`) makes a base fetch due.
pub(super) fn viewed(run: &mut Run, n: u16) {
    let open = (run.delivery.pr(n)).is_some_and(|p| p.state == proto::PrState::Open);
    if !open || target(run) != Some(n) {
        return;
    }
    let conflicting =
        (run.delivery.pr(n)).is_some_and(|p| p.watermark.mergeable == Some(Mergeable::Conflicting));
    let due = match run.delivery.limits.sync {
        SyncPolicy::OnConflict => conflicting,
        SyncPolicy::Always => true,
    };
    run.delivery.base_fetch_due |= due;
}

/// The merge commit of the oldest merged stage whose method is not known yet (ruling
/// R-4). A base fetch counts its parents, so one is due while any is left (fix round
/// 1, I1: two merges seen around one fetch each get theirs).
fn method_due(run: &Run) -> Option<String> {
    (run.delivery.stages.iter())
        .filter_map(|s| s.pr.as_ref())
        .find(|p| {
            p.state == proto::PrState::Merged
                && p.merge_method.is_none()
                && p.merge_commit.as_deref().is_some_and(is_object_id)
        })
        .and_then(|p| p.merge_commit.clone())
}

/// Milestone 9.7 decision 5: the question about the oldest undecided stage, which the
/// base fetch asks while any is left (one per fetch, as `method_due`).
fn contains_due(run: &Run) -> Option<Contains> {
    let (n, (head, merged)) = (1..=stage_count(run)).find_map(|n| {
        let pair = run.delivery.stage(n)?.undecided.clone()?;
        Some((n, pair))
    })?;
    Some(Contains {
        stage: n,
        branch: super::open::remote_branch(run, n),
        into: crate::host::stage_remote_ref(&run.id, n),
        head,
        merged,
        pr: run.delivery.pr(n).map(|p| p.number),
    })
}

/// The base fetch is due: after a merge, or for a merge commit's method or an
/// undecided stage's question (FW-5: its failures count, as `alerts::due` reads it).
pub(super) fn fetch_due(run: &Run) -> bool {
    run.delivery.base_fetch_due || method_due(run).is_some() || contains_due(run).is_some()
}

/// The pass: the due base fetch, one at a time, after a failed one's wait. It asks for
/// the parents of `method_due`'s merge commit and `contains_due`'s question.
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let waiting = run.delivery.base_fetch_retry_at.is_some_and(|t| t > now);
    let parents_of = method_due(run);
    let contains = contains_due(run);
    if !fetch_due(run) || waiting || fetching(run) {
        return;
    }
    let op = HostOp::Fetch {
        stage: None,
        branch: run.base_branch.clone(),
        into: base_ref(run),
        adopt: None,
        parents_of,
        contains: contains.map(Box::new),
    };
    emit(run, op, fx);
}

/// A base fetch failed (decision 11): it is tried again one poll interval later. Its
/// question counts a failed check (milestone 9.7 ruling R1).
pub(super) fn fetch_failed(run: &mut Run, contains: Option<&Contains>, now: u64, wait: u64) {
    run.delivery.base_fetch_retry_at = Some(now.saturating_add(wait));
    if let Some(c) = contains {
        super::land_judge::check_failed(run, c, now);
    }
}

/// The base fetch answered: the merge method it counted (ruling R-4), the verdict of
/// its question (milestone 9.7 decision 7: no answer decides as not delivered), then a
/// base sync of the lowest delivering stage when the base moved past what the stages
/// hold.
pub(super) fn fetched(
    run: &mut Run,
    parents_of: Option<String>,
    contains: Option<Contains>,
    outcome: FetchOutcome,
    now: u64,
) {
    run.delivery.base_fetch_due = false;
    run.delivery.base_fetch_retry_at = None;
    let parents = match &outcome {
        FetchOutcome::Fetched { parents, .. } => *parents,
        _ => None,
    };
    if let Some(oid) = parents_of {
        super::land::method(run, &oid, parents, now);
    }
    if let Some(c) = contains {
        let answer = match &outcome {
            FetchOutcome::Fetched { contains, .. } => *contains,
            _ => None,
        };
        super::land_judge::decided(run, &c, answer, now);
    }
    match outcome {
        FetchOutcome::Fetched { sha, .. } => queue(run, sha, now),
        FetchOutcome::Missing => {
            let text = format!(
                "the base branch {} is gone from the remote",
                run.base_branch
            );
            log(run, now, text);
        }
        _ => {}
    }
}

/// Decision 33: base commit `sha` is due into the lowest delivering stage, unless the
/// stages hold it already, it is due there already, or a base sync of it was red on
/// the stage's current head.
fn queue(run: &mut Run, sha: String, now: u64) {
    let Some(n) = target(run) else {
        return;
    };
    let head = run.stage_head(n).unwrap_or_default().to_string();
    let red = (run.delivery.stage(n)).and_then(|s| s.sync_red.clone());
    if sha == synced(run)
        || run.delivery.base_sync_due.get(&n) == Some(&sha)
        || in_flight(run, n, &sha)
        || resolving(run, n, &sha)
        || red == Some((sha.clone(), head))
    {
        return;
    }
    run.delivery.alerts.remove(&format!("{n}/sync"));
    let text = format!(
        "stage {n}: {} moved to {}; merging it into stage {n}",
        run.base_branch,
        sha7(&sha)
    );
    run.delivery.base_sync_due.insert(n, sha);
    log(run, now, text);
}

/// Called first by 9.1's `merge::start_merge` (decision 33, ruling R-7): the due base
/// sync of the lowest stage goes before any propagate. One waits for an adopt of its
/// stage and for the stage's open sync task; one whose stage no longer delivers is
/// dropped. `true` when an op was emitted.
pub(in crate::run::engine) fn start(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> bool {
    if !pr(run) || run.state != RunState::Running || run.cancelled {
        return false;
    }
    let due: Vec<(u16, String)> = (run.delivery.base_sync_due.iter())
        .map(|(n, s)| (*n, s.clone()))
        .collect();
    for (n, sha) in due {
        if target(run) != Some(n) {
            run.delivery.base_sync_due.remove(&n);
            let text = format!(
                "stage {n}: the merge of {} is dropped: the stage no longer delivers",
                sha7(&sha)
            );
            log(run, now, text);
            continue;
        }
        if stage_busy(run, n) || sync_open(run, n) || run.stage_head(n).is_none() {
            continue;
        }
        run.delivery.base_sync_due.remove(&n);
        merge_base(run, n, sha, fx);
        return true;
    }
    false
}

/// Decision 33's `PropagateSpec { from: 0 }`: base commit `sha` merged into stage `n`
/// as a merge candidate is, its parents `[stage head, sha]`, tier 2 on the result.
fn merge_base(run: &mut Run, n: u16, sha: String, fx: &mut Vec<Effect>) {
    let expected = run.stage_head(n).unwrap_or_default().to_string();
    let integration = run.integration_path();
    let tier = super::super::tiers::tier2_spec(run, n, &expected);
    let multi = run.stage_layout == StageLayout::Multi;
    let spec = PropagateSpec {
        root: run.root.clone(),
        env: profile_env(&run.profile, &integration),
        integration,
        from: 0,
        to: n,
        message: format!(
            "anthrex: merge {}@{} into stage-{n}",
            run.base_branch,
            sha7(&sha)
        ),
        from_head: sha,
        to_branch: run.stage_branch(n),
        expected_to_head: expected,
        also_integration: multi && n >= stages::highest(run),
        base_branch: run.base_branch.clone(),
        expected_base: run.base_sha.clone(),
        guarded: stages::guard_list(run),
        check: run.profile.check.clone().filter(|_| tier.is_none()),
        tier,
        timeout_secs: run.profile.check_timeout_secs,
        tasks: Default::default(),
    };
    let op = next_op(run);
    emit_op(run, op, None, OpKind::Propagate(Box::new(spec)), fx);
}

/// A lost base sync (reconciled `NotStarted`) is due again as a base sync, never as a
/// stage propagate (ruling R-7).
pub(in crate::run::engine) fn lost(run: &mut Run, spec: &PropagateSpec) {
    (run.delivery.base_sync_due).insert(spec.to, spec.from_head.clone());
}

/// A base sync's result (decisions 33–34; 9.1's `propagate::done` hands every `from: 0`
/// here, after its tier-2 records).
pub(in crate::run::engine) fn done(
    run: &mut Run,
    (_op, spec): (OpId, &PropagateSpec),
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let n = spec.to;
    let base = format!("{}@{}", run.base_branch, sha7(&spec.from_head));
    match result {
        OpResult::Merged { commit, .. } => {
            // 9.1's one writer of stage heads: the stage above is due its propagate.
            stages::task_merged(run, n, None, &commit);
            if let Some(record) = run.stages.iter_mut().find(|s| s.n == n) {
                record.merges.push(StageMerge::Propagate {
                    from: 0,
                    commit: commit.clone(),
                });
            }
            absorbed(run, n, &spec.from_head);
            log(
                run,
                now,
                format!("stage {n}: merged {base} ({})", sha7(&commit)),
            );
        }
        OpResult::AlreadyHeld => {
            absorbed(run, n, &spec.from_head);
            log(run, now, format!("stage {n}: already holds {base}"));
        }
        OpResult::Conflict { files, tree } => conflicted(run, spec, files, tree, now, fx),
        OpResult::CandidateRed { tier, .. } => {
            let tests = tier.map(|o| super::super::tiers::record(&o, now).failing);
            let tests = tests.unwrap_or_default();
            let names = if tests.is_empty() {
                NO_NAMES.to_string()
            } else {
                let shown: Vec<&str> = tests.iter().take(LINE_TESTS).map(String::as_str).collect();
                shown.join(", ")
            };
            let line = format!("the merge of {base} into stage {n} is red: {names}");
            red(run, spec, line, now);
        }
        OpResult::RefMoved { reason } => {
            lost(run, spec);
            halt(run, reason, now);
        }
        OpResult::Failed { message } => {
            lost(run, spec);
            halt(
                run,
                format!("could not merge {base} into stage {n}: {message}"),
                now,
            );
            run.halt_retryable = true;
        }
        _ => {}
    }
}

/// Stage `n` holds base commit `sha`: the stages are synced with it.
fn absorbed(run: &mut Run, n: u16, sha: &str) {
    run.delivery.base_synced = Some(sha.to_string());
    stage_mut(run, n).sync_red = None;
    run.delivery.alerts.remove(&format!("{n}/sync"));
}

/// A base sync task merged into stage `n` (decision 34): the stage holds its base.
pub(crate) fn task_merged(run: &mut Run, n: u16, base_sha: &str) {
    absorbed(run, n, base_sha);
}

/// A base sync that could not be made, or was red: it is not tried again on this base
/// and stage head (`sync_red`); an attention line and a wake note (invented, as 9.1's
/// red propagate's).
fn red(run: &mut Run, spec: &PropagateSpec, line: String, now: u64) {
    let n = spec.to;
    let current = run.stage_head(n) == Some(spec.expected_to_head.as_str());
    log(run, now, line.clone());
    if !current {
        return;
    }
    stage_mut(run, n).sync_red = Some((spec.from_head.clone(), spec.expected_to_head.clone()));
    run.delivery
        .alerts
        .insert(format!("{n}/sync"), line.clone());
    wake::note(run, format!("{line}; plan a fix in stage {n}"));
}

/// Decision 34: a conflict adds 9.1's `sync` fix task, with `FixOf::Base`, owning the
/// conflicted files exactly; its worktree gets the conflicted merge before its first
/// session (9.1's hand-back), and its spill is measured from the conflicted tree.
/// Decision 26's approval rule applies: a conflicted file outside the stage is held.
fn conflicted(
    run: &mut Run,
    spec: &PropagateSpec,
    files: Vec<String>,
    tree: Option<String>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let n = spec.to;
    let what = format!(
        "stage {n} conflicts with {}@{}",
        run.base_branch,
        sha7(&spec.from_head)
    );
    let Some(tree) = tree else {
        return red(
            run,
            spec,
            format!("{what}: the conflicted tree is unknown"),
            now,
        );
    };
    let mut owns = Vec::with_capacity(files.len());
    for file in &files {
        let entry = crate::run::globs::escape_path(file);
        if let Err(why) = crate::run::globs::validate_glob(&entry) {
            let line =
                format!("{what}: the conflicted path {file} cannot be owned exactly ({why})");
            return red(run, spec, line, now);
        }
        owns.push(entry);
    }
    let url = run.delivery.pr(n).map(|p| p.url.clone());
    let base = run.base_branch.clone();
    let fix = FixSpec {
        origin: TaskOrigin::Sync,
        fixes: FixOf::Base {
            stage: n,
            base_sha: spec.from_head.clone(),
        },
        stage: n,
        title: templates::sync_title(n, &base),
        brief: templates::sync_brief(n, &base, sha7(&spec.from_head), &files, url.as_deref()),
        acceptance: templates::sync_acceptance(),
        owns,
        size: Size::M,
        epic: None,
        route: RouteSpec::default(),
        test_mode: TestMode::Check,
        test_mode_reason: Some("a merge resolution; the tiers check it".to_string()),
        sync: Some(SyncState {
            onto: spec.from_head.clone(),
            base_tree: tree,
            tasks: Default::default(),
            handed_back: false,
            to_head: spec.expected_to_head.clone(),
            handed: Vec::new(),
        }),
    };
    match add_fix(run, fix, now, fx) {
        Ok(id) => {
            let text = format!("{what}: sync task {id} added");
            log(run, now, text.clone());
            wake::note(run, text);
            super::review_fix::hold_outside(run, n, &id, now);
        }
        Err(message) => red(
            run,
            spec,
            format!("{what}: its sync task was refused: {message}"),
            now,
        ),
    }
}
