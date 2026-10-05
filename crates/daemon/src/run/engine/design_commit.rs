//! Milestone 9.6 decision 23 (DF §5.3, task M9.6.12), engine side: the documents
//! commit. The plan gate's approval marks it due ([`approved`]) when the run commits
//! its documents (`[orchestrator.design] docs_dir` not empty; round 1 only, ruling
//! T12-2). While it is due the scheduler starts nothing that branches from the run
//! head: no worktree, no stage branch, no review checkout (Review focus 3; [`hold`],
//! called first in `dispatch::schedule`, and `stages::create_pass`), and it sends the
//! one `CommitDesignDocs` while the run runs and none is in flight. A restart sends a
//! lost one again as it was (`restore_lost`'s git arm). Its reply moves the run head
//! through `stages::set_stage_head`, so the bottom stage holds it in every layout. A
//! failure halts the run retryably, and `run resume` sends it again; a documents
//! folder through a symbolic link halts it for good (ruling T12-1, which checks it at
//! the start too). Ruling T1-O2: such a run skips 9.5's plan-gate pre-warm, so no
//! checkout is built on a head the commit moves.
//!
//! Task M9.6.15 (decision 24, DF §8.1): each later round that amends the spec commits
//! too, once. Its commit appends the amendment to the committed spec under
//! `## Round <k> amendment` and adds the round's plan as `…-round<k>.md`; it is the
//! round's first new stage: `stages::create_pass`, about to create that stage from the
//! stage below, asks for the commit instead ([`round_stage`]), whose stage branch is
//! created at it. Nothing of the round can branch before: its tasks run only in their
//! stages, which do not exist yet. Stage 1's record and ref never move.
//! Pure (design decision 2).

use proto::{DesignMode, DocKind, RoundDesign, RoundOutcome, RunState};

use super::requests::log;
use super::{Effect, OpKind, OpResult, emit_op, merge, next_op, stages};
use crate::run::contract::sha7;
use crate::run::design::commit::{DocSource, DocsCommitSpec, FOLDERS};
use crate::run::design::plan_md::goal_head;
use crate::run::design::state::{DesignRound, DesignState, design_dir};
use crate::run::design::template::kind_name;
use crate::run::model::{Run, StageLayout};
use crate::run::report::format_utc;

/// Whether the run commits its documents: a design run with a `docs_dir` whose current
/// round has not committed yet. Ruling T12-2, as task M9.6.15 lifts it: a later round
/// commits when it amended the spec (`amend` or `full`), never with the flow off.
fn commits(run: &Run) -> bool {
    let fresh = |d: &DesignState| committed_through(d) < run.round() && !d.round_off();
    run.design_mode == DesignMode::Full
        && run.orch.design.as_ref().is_some_and(fresh)
        && !run.limits.orch.design.docs_dir.is_empty()
}

/// The last round whose documents were committed (a commit from before task M9.6.15
/// records only `committed`: round 1's).
fn committed_through(design: &DesignState) -> u32 {
    match (design.committed_round, &design.committed) {
        (0, Some(_)) => 1,
        (k, _) => k,
    }
}

/// Task M9.6.15: the commit is a later round's first new stage (from round 2 on every
/// run has stages, `goal_rounds::widen`), not a commit at the run head.
fn on_stage(run: &Run) -> bool {
    run.round() > 1 && run.stage_layout == StageLayout::Multi
}

/// Whether stage creation waits for round 1's documents commit (a later round's is
/// its first stage, [`round_stage`]).
pub(crate) fn holds_stages(run: &Run) -> bool {
    due(run) && !on_stage(run)
}

/// Ruling T1-O2: the plan gate pre-warms nothing in a run that will commit documents.
pub(super) fn skips_prewarm(run: &Run) -> bool {
    commits(run)
}

/// Decision 23: the plan gate's approval; the commit is due when the run commits its
/// documents (with `docs_dir = ""` nothing is committed and the run starts at once).
pub(super) fn approved(run: &mut Run, now: u64) {
    if !commits(run) {
        return;
    }
    if let Some(design) = run.orch.design.as_mut() {
        design.commit_due = true;
    }
    let dir = run.limits.orch.design.docs_dir.clone();
    log(run, now, format!("committing the spec and plan to {dir}"));
}

/// The documents commit is due: nothing branches from the run head yet.
pub(crate) fn due(run: &Run) -> bool {
    run.orch.design.as_ref().is_some_and(|d| d.commit_due)
}

/// A `CommitDesignDocs` is in flight.
pub(super) fn in_flight(run: &Run) -> bool {
    (run.pending_ops.values()).any(|p| matches!(p.kind, OpKind::CommitDesignDocs(_)))
}

/// Rulings T15-14 and T15-15: stage `n` holds a later round's documents commit: the
/// stage the in-flight commit targets, or the one a round's landed commit created (its
/// round record's `committed_stage`; never inferred from a stage's `created_from`, which
/// a stage created above it at the same head shares). It is never skipped as unused.
pub(crate) fn holds_docs(run: &Run, n: u16) -> bool {
    let branch = run.stage_branch(n);
    let creating = (run.pending_ops.values()).any(|p| {
        matches!(&p.kind, OpKind::CommitDesignDocs(s) if s.stage_branch.as_deref() == Some(branch.as_str()))
    });
    creating || (run.rounds.iter()).any(|r| r.committed_stage == Some(n))
}

/// The scheduler's guard: whether the commit is due, in which case the run's running
/// passes wait. While the run runs and none is in flight, the commit is sent (the
/// approval's, or a retry after `run resume`).
pub(super) fn hold(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> bool {
    if !holds_stages(run) {
        return false;
    }
    if run.state == RunState::Running && !in_flight(run) {
        match spec(run, None) {
            Ok(spec) => {
                let op = next_op(run);
                emit_op(run, op, None, OpKind::CommitDesignDocs(Box::new(spec)), fx);
            }
            // Ruling T12-3 (m1): decision 23's text.
            Err(message) => halt(run, failure(&message), now),
        }
    }
    true
}

/// Task M9.6.15: `stages::create_pass` would create stage `next` from `from` while a
/// later round's commit is due. From the round's first stage on, the commit is that
/// stage (created at it) and nothing else is created meanwhile: true when it is sent,
/// in flight, or could not be built (the run halts with decision 23's text).
pub(super) fn round_stage(
    run: &mut Run,
    next: u16,
    from: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> bool {
    let first = run.current_round().map_or(1, |r| r.first_stage);
    if !due(run) || !on_stage(run) || next < first {
        return false;
    }
    if in_flight(run) {
        return true;
    }
    match spec(run, Some((run.stage_branch(next), from.to_string()))) {
        Ok(spec) => {
            let op = next_op(run);
            emit_op(run, op, None, OpKind::CommitDesignDocs(Box::new(spec)), fx);
        }
        Err(message) => halt(run, failure(&message), now),
    }
    true
}

/// The commit of the approved versions (decision 24): the approved spec, the plan
/// (its gate version, the one approved) and, with `commit_brainstorm`, the brainstorm
/// report, each as its index recorded it; on the run branch at the run head. A later
/// round's ([`round_files`]) is `stage`, `(branch, from)`: that stage's branch created
/// at the commit, whose one parent is `from`, the head of the stage below.
fn spec(run: &Run, stage: Option<(String, String)>) -> Result<DocsCommitSpec, String> {
    let design = run.orch.design.as_ref().ok_or("no design state")?;
    // Ruling T15-7 (m5): a later round's commit is only its approved amendment, as its
    // first stage; anything else halts with decision 23's text.
    let k = run.round();
    if k > 1 && stage.is_none() {
        return Err(format!(
            "round {k}'s documents commit is not a stage of the run"
        ));
    }
    let amended = |r: &DesignRound| {
        r.mode != RoundDesign::Off
            && design
                .approved_spec
                .is_some_and(|v| Some(v) != r.spec_before)
    };
    if k > 1 && !design.round.as_ref().is_some_and(amended) {
        return Err(format!("round {k} approved no amendment of the spec"));
    }
    let limits = &run.limits.orch.design;
    let dir = design_dir(run);
    let source = |folder: &str, kind: DocKind, n: Option<u32>| {
        let what = kind_name(kind);
        let version = (design.find(kind, n).filter(|v| v.n > 0))
            .ok_or_else(|| format!("the approved {what} has no stored version"))?;
        Ok::<_, String>(DocSource {
            folder: folder.to_string(),
            what: format!("{what} v{}", version.n),
            path: dir.join(design.file_name(version)),
            bytes: version.bytes,
            sha256: version.sha256.clone(),
            repo_path: None,
            append: None,
        })
    };
    let mut files = vec![
        source(FOLDERS[0], DocKind::Spec, design.approved_spec)?,
        source(FOLDERS[1], DocKind::Plan, None)?,
    ];
    // A later round's brainstorm is its own only when the round brainstormed.
    let brainstormed = (design.round.as_ref()).is_none_or(|r| r.mode == RoundDesign::Full);
    if limits.commit_brainstorm && brainstormed {
        files.push(source(FOLDERS[2], DocKind::Brainstorm, None)?);
    }
    let started = format_utc(run.created_at);
    let head = goal_head(&run.goal);
    let (expected_head, message, stage_branch) = match stage {
        None => (
            run.run_head.clone(),
            format!("docs: spec and plan for {head}"),
            None,
        ),
        Some((branch, from)) => {
            round_files(design, run.round(), &limits.docs_dir, &mut files)?;
            let k = run.round();
            let message = format!("docs: round {k} spec amendment and plan for {head}");
            (from, message, Some(branch))
        }
    };
    Ok(DocsCommitSpec {
        root: run.root.clone(),
        branch: run.run_branch(),
        expected_head,
        integration: run.integration_path(),
        docs_dir: limits.docs_dir.clone(),
        date: started.get(..10).unwrap_or(&started).to_string(),
        run_id: run.id.clone(),
        files,
        message,
        stage_branch,
    })
}

/// Decision 24, round `k`: the amendment is appended to the committed spec under
/// `## Round <k> amendment`, and the round's plan (and brainstorm) go beside round 1's,
/// named from the committed spec's file: `<docs_dir>/plans/<date>-<slug>-round<k>.md`.
fn round_files(
    design: &DesignState,
    k: u32,
    docs_dir: &str,
    files: &mut [DocSource],
) -> Result<(), String> {
    let spec = design
        .spec_path
        .clone()
        .ok_or("the spec of round 1 was never committed")?;
    let name = spec.rsplit('/').next().unwrap_or(&spec);
    let stem = name.strip_suffix(".md").unwrap_or(name);
    for file in files.iter_mut() {
        match file.folder == FOLDERS[0] {
            true => {
                file.repo_path = Some(spec.clone());
                file.append = Some(format!("## Round {k} amendment"));
            }
            false => {
                let path = format!("{docs_dir}/{}/{stem}-round{k}.md", file.folder);
                file.repo_path = Some(path);
            }
        }
    }
    Ok(())
}

/// `CommitDesignDocs`' result: the run head moves to the commit (stage 1's head in a
/// `Single` run; a `Multi` run's stage 1 is then created from it; a later round's first
/// stage is recorded at it, created from it and holding the stage below), and the work
/// starts from there; or the run halts with decision 23's text, retryably (a symbolic
/// link on the way, for good).
pub(super) fn done(run: &mut Run, sent: &DocsCommitSpec, result: OpResult, now: u64) {
    if run.state.is_terminal() || !due(run) {
        return;
    }
    match result {
        OpResult::DocsCommitted { head, spec } => {
            let round = run.round();
            if let Some(design) = run.orch.design.as_mut() {
                design.commit_due = false;
                design.committed = Some(head.clone());
                design.committed_round = round;
                design.spec_path = Some(spec);
            }
            if let Some(branch) = &sent.stage_branch {
                return round_committed(run, sent, branch, &head, now);
            }
            stages::set_stage_head(run, 1, &head);
            // A `Single` run's stage 1 exists from the start: the commit is on its line
            // but no merge of it, so it is the line's floor (ruling C-28 (1)), as a
            // `Multi` run's stage 1 is created from it.
            if let Some(record) = run.stages.iter_mut().find(|s| s.n == 1)
                && record.merges.is_empty()
            {
                record.floor = Some(head.clone());
            }
            let text = format!(
                "committed the spec and plan as {} on {}",
                sha7(&head),
                run.run_branch()
            );
            log(run, now, text);
        }
        // Ruling T15-16: a cancelled round's commit that failed landed nothing; the
        // round is dropped, and nothing halts.
        OpResult::DocsThroughSymlink { .. } | OpResult::Failed { .. } if cancelled(run) => {
            let why = match result {
                OpResult::Failed { message } => message,
                _ => symlink_halt(&run.limits.orch.design.docs_dir),
            };
            let k = run.round();
            super::design_round::dropped(run);
            let text = format!(
                "round {k}'s documents commit failed after its cancel: {why}; the round is dropped"
            );
            log(run, now, text);
        }
        OpResult::DocsThroughSymlink { .. } => {
            let dir = &run.limits.orch.design.docs_dir;
            let text = symlink_halt(dir);
            // Ruling T12-1: the frozen `docs_dir` means a resume cannot succeed.
            merge::halt(run, text, now);
        }
        OpResult::Failed { message } => halt(run, failure(&message), now),
        other => halt(run, failure(&format!("unexpected result {other:?}")), now),
    }
}

/// Task M9.6.15: a later round's commit made its first stage, `branch`, at `head`: the
/// stage is recorded as created there (`stages::created`), holding the stage below's
/// head (the commit's parent), so no propagate is due; the run head follows it.
fn round_committed(run: &mut Run, sent: &DocsCommitSpec, branch: &str, head: &str, now: u64) {
    let kind = OpKind::CreateStageBranch {
        root: sent.root.clone(),
        branch: branch.to_string(),
        from: head.to_string(),
    };
    stages::created(run, &kind, OpResult::StageCreated, now);
    let n = stages::highest(run);
    if let Some(record) = run.stages.iter_mut().find(|s| s.n == n) {
        record.synced_from = Some(sent.expected_head.clone());
    }
    // Ruling T15-15: the round record's own stage, which holds the amendment.
    if let Some(round) = run.rounds.last_mut() {
        round.committed_stage = Some(n);
    }
    let text = format!(
        "committed round {} spec amendment and plan as {} on {branch}",
        run.round(),
        sha7(head)
    );
    log(run, now, text);
}

/// The current round was cancelled (decision 16).
fn cancelled(run: &Run) -> bool {
    (run.current_round()).is_some_and(|r| r.n > 1 && r.outcome == Some(RoundOutcome::Cancelled))
}

/// Ruling T12-1's text, at a design run's start and at its commit.
pub(crate) fn symlink_halt(docs_dir: &str) -> String {
    format!(
        "design flow: the documents folder {docs_dir} goes through a symlink in the repository; change [orchestrator.design].docs_dir"
    )
}

fn failure(message: &str) -> String {
    format!("design flow: could not commit the spec and plan: {message}")
}

/// Decision 23: halted with `text`; `run resume` retries the commit.
fn halt(run: &mut Run, text: String, now: u64) {
    merge::halt(run, text, now);
    run.halt_retryable = true;
}
