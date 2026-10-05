//! Milestone 9.6 decision 23 (DF §5.3, task M9.6.12), engine side: the documents
//! commit. The plan gate's approval marks it due ([`approved`]) when the run commits
//! its documents (`[orchestrator.design] docs_dir` not empty). While it is due the
//! scheduler starts nothing that branches from the run head: no worktree, no stage
//! branch, no review checkout (Review focus 3; [`hold`], called first in
//! `dispatch::schedule`, and `stages::create_pass`), and it sends the one
//! `CommitDesignDocs` while the run runs and none is in flight. A restart sends a lost
//! one again as it was (`restore_lost`'s git arm). Its reply moves the run head through
//! `stages::set_stage_head`, so the bottom stage holds it in every layout; a failure
//! halts the run retryably, and `run resume` sends it again. Ruling T1-O2: such a run
//! skips 9.5's plan-gate pre-warm, so no checkout is built on a head the commit moves.
//! Pure (design decision 2).

use proto::{DesignMode, DocKind, RunState};

use super::requests::log;
use super::{Effect, OpKind, OpResult, emit_op, merge, next_op, stages};
use crate::run::contract::sha7;
use crate::run::design::commit::{DocSource, DocsCommitSpec};
use crate::run::design::plan_md::goal_head;
use crate::run::design::state::design_dir;
use crate::run::design::template::kind_name;
use crate::run::model::Run;
use crate::run::report::format_utc;

/// Whether the run commits its documents: a design run with a `docs_dir`.
fn commits(run: &Run) -> bool {
    run.design_mode == DesignMode::Full
        && run.orch.design.is_some()
        && !run.limits.orch.design.docs_dir.is_empty()
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

fn in_flight(run: &Run) -> bool {
    (run.pending_ops.values()).any(|p| matches!(p.kind, OpKind::CommitDesignDocs(_)))
}

/// The scheduler's guard: whether the commit is due, in which case the run's running
/// passes wait. While the run runs and none is in flight, the commit is sent (the
/// approval's, or a retry after `run resume`).
pub(super) fn hold(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> bool {
    if !due(run) {
        return false;
    }
    if run.state == RunState::Running && !in_flight(run) {
        match spec(run) {
            Ok(spec) => {
                let op = next_op(run);
                emit_op(run, op, None, OpKind::CommitDesignDocs(Box::new(spec)), fx);
            }
            Err(message) => halt(run, message, now),
        }
    }
    true
}

/// The commit of the approved versions (decision 24): the approved spec, the plan
/// (its gate version, the one approved) and, with `commit_brainstorm`, the brainstorm
/// report, each as its index recorded it; on the run branch at the run head.
fn spec(run: &Run) -> Result<DocsCommitSpec, String> {
    let design = run.orch.design.as_ref().ok_or("no design state")?;
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
        })
    };
    let mut files = vec![
        source("specs", DocKind::Spec, design.approved_spec)?,
        source("plans", DocKind::Plan, None)?,
    ];
    if limits.commit_brainstorm {
        files.push(source("brainstorms", DocKind::Brainstorm, None)?);
    }
    let started = format_utc(run.created_at);
    Ok(DocsCommitSpec {
        root: run.root.clone(),
        branch: run.run_branch(),
        expected_head: run.run_head.clone(),
        integration: run.integration_path(),
        docs_dir: limits.docs_dir.clone(),
        date: started.get(..10).unwrap_or(&started).to_string(),
        run_id: run.id.clone(),
        files,
        message: format!("docs: spec and plan for {}", goal_head(&run.goal)),
    })
}

/// `CommitDesignDocs`' result: the run head moves to the commit (stage 1's head in a
/// `Single` run; a `Multi` run's stage 1 is then created from it), and the work starts
/// from there; or the run halts with decision 23's text, retryably.
pub(super) fn done(run: &mut Run, result: OpResult, now: u64) {
    if run.state.is_terminal() || !due(run) {
        return;
    }
    match result {
        OpResult::DocsCommitted { head, spec } => {
            if let Some(design) = run.orch.design.as_mut() {
                design.commit_due = false;
                design.committed = Some(head.clone());
                design.spec_path = Some(spec);
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
        OpResult::DocsThroughSymlink { .. } => {
            let dir = &run.limits.orch.design.docs_dir;
            let text = format!(
                "design flow: the documents folder {dir} goes through a symlink in the repository; change [orchestrator.design].docs_dir"
            );
            halt(run, text, now);
        }
        OpResult::Failed { message } => halt(run, failure(&message), now),
        other => halt(run, failure(&format!("unexpected result {other:?}")), now),
    }
}

fn failure(message: &str) -> String {
    format!("design flow: could not commit the spec and plan: {message}")
}

/// Decision 23: halted with `text`; `run resume` retries the commit.
fn halt(run: &mut Run, text: String, now: u64) {
    merge::halt(run, text, now);
    run.halt_retryable = true;
}
