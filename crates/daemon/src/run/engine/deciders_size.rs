//! The size cross-check (M8b decision 19, spec §7.2 rule 5). `Start` asks it about
//! every task, and an accepted edit about the unstarted tasks it added or amended. A
//! task's evidence is the scout reports its `scout_refs` name, or the stored profile's
//! onboarding report when it names none; the evidenced tasks go into one `SizeCheck`
//! request per 50, and each waits for it, not runnable (`schedule::size_check_pending`).
//!
//! The answer can only raise: a larger size than the engine's raises S to M
//! ([`apply_raise`]) or blocks the task as mis-sized for L. It never lowers a size,
//! never approves, and never changes what a task owns or must meet. The engine names
//! the reports (`evidence_refs`); the driver reads them. Pure (design decision 2).

use proto::{BlockReason, DeciderMode, DeciderSource, Size, SizeCheckInfo, TaskState};

use super::dispatch::{block, history};
use super::{Effect, deciders, ladder};
use crate::decider::fallback::{OFF_REASON, SIZE_FALLBACK_REASON};
use crate::decider::{DeciderRequest, Decision, SizeCheckInput, SizeCheckTask, SizeVerdict};
use crate::run::contract::size_label;
use crate::run::model::{Run, SizeCheckState, Task};
use crate::scout::report::ONBOARDING_ALIAS;

/// The note of a task the check cannot be asked about.
pub(crate) const SKIPPED_NOTE: &str = "size cross-check skipped: no scout evidence";
/// The schema's `maxItems`: at most this many tasks per request.
const BATCH_MAX: usize = 50;

/// The reports task `task`'s evidence is: those its `scout_refs` name that the run
/// has (a run scout's id, or the alias `onboarding` when the repository has an
/// onboarding report), else the onboarding report. Anything else a plan names is
/// ignored, so no plan text ever becomes a path.
fn refs_of(run: &Run, task: &Task) -> Vec<String> {
    let onboarding = run.onboarding_report.is_some();
    if task.spec.scout_refs.is_empty() {
        return if onboarding {
            vec![ONBOARDING_ALIAS.to_string()]
        } else {
            Vec::new()
        };
    }
    let mut refs: Vec<String> = Vec::new();
    for r in &task.spec.scout_refs {
        let known =
            run.scout_reports.iter().any(|s| s == r) || (onboarding && r == ONBOARDING_ALIAS);
        if known && !refs.contains(r) {
            refs.push(r.clone());
        }
    }
    refs
}

fn task_of(task: &Task) -> SizeCheckTask {
    SizeCheckTask {
        id: task.id().to_string(),
        title: task.spec.title.clone(),
        brief: task.spec.brief.clone(),
        acceptance: task.spec.acceptance.clone(),
        owns: task.spec.owns.clone(),
        deps: task.spec.deps.clone(),
        size: task.size,
        interface_change: task.spec.interface_change,
        hub: task.hub,
    }
}

/// Decision 19: cross-checks tasks `ids`. Those with no evidence are noted (not with
/// the deciders off, which keep M8a's notes) and left runnable; the others wait for
/// one request per 50, queued and started (or answered by the fallback) at once.
pub(super) fn cross_check(run: &mut Run, ids: &[String], now: u64, fx: &mut Vec<Effect>) {
    let off = run.limits.decider_mode == DeciderMode::Off;
    let mut evidenced: Vec<(usize, Vec<String>)> = Vec::new();
    for id in ids {
        let Some(i) = run.tasks.iter().position(|t| t.id() == id) else {
            continue;
        };
        let refs = refs_of(run, &run.tasks[i]);
        if !refs.is_empty() {
            evidenced.push((i, refs));
            continue;
        }
        let task = &mut run.tasks[i];
        task.drop_pending_size_check();
        if !off && !task.notes.iter().any(|n| n == SKIPPED_NOTE) {
            task.notes.push(SKIPPED_NOTE.to_string());
        }
    }
    for chunk in evidenced.chunks(BATCH_MAX) {
        // Decision 19's order: run scouts first, then the alias `onboarding`.
        let mut evidence_refs: Vec<String> = Vec::new();
        let mut alias = false;
        for r in chunk.iter().flat_map(|(_, refs)| refs) {
            if r == ONBOARDING_ALIAS {
                alias = true;
            } else if !evidence_refs.contains(r) {
                evidence_refs.push(r.clone());
            }
        }
        if alias {
            evidence_refs.push(ONBOARDING_ALIAS.to_string());
        }
        let tasks: Vec<SizeCheckTask> =
            chunk.iter().map(|(i, _)| task_of(&run.tasks[*i])).collect();
        let task_ids = tasks.iter().map(|t| t.id.clone()).collect();
        let request = DeciderRequest::SizeCheck(SizeCheckInput {
            tasks,
            evidence_refs,
            evidence: Vec::new(),
            modules: run.profile.modules.clone(),
            hub: run.profile.hub.clone(),
            thresholds: run.limits.thresholds,
        });
        let decider_id = deciders::queue(run, task_ids, request, now);
        for (i, _) in chunk {
            run.tasks[*i].size_check = Some(SizeCheckState::Pending { decider_id });
        }
    }
    fx.extend(deciders::dispatch(run, now));
}

/// Decision 19: size check `decider_id` answered `verdicts` (already filtered to the
/// asked ids, the first per id: ruling R-T5-1). Each asked task still waiting for it
/// records the answer and is raised or blocked when the answer is larger; a task
/// missing from the answer, or every task of a fallback, keeps its size.
pub(super) fn sized(
    run: &mut Run,
    decider_id: u64,
    input: &SizeCheckInput,
    verdicts: &[SizeVerdict],
    decision: &Decision,
    now: u64,
) {
    let off = decision.fallback_reason.as_deref() == Some(OFF_REASON);
    for asked in &input.tasks {
        let Some(i) = run.tasks.iter().position(|t| t.id() == asked.id) else {
            continue;
        };
        let task = &run.tasks[i];
        if task.size_check != Some(SizeCheckState::Pending { decider_id }) {
            continue;
        }
        let engine = task.size;
        let verdict = verdicts
            .iter()
            .find(|v| v.id == asked.id)
            .filter(|_| decision.source == DeciderSource::Decider);
        let info = match verdict {
            Some(v) => SizeCheckInfo {
                engine,
                decided: Some(v.size),
                agreed: v.size == engine,
                reason: v.reason.clone(),
                source: DeciderSource::Decider,
            },
            None => SizeCheckInfo {
                engine,
                decided: None,
                agreed: true,
                reason: decision
                    .fallback_reason
                    .clone()
                    .unwrap_or_else(|| SIZE_FALLBACK_REASON.to_string()),
                source: DeciderSource::Fallback,
            },
        };
        run.tasks[i].size_check = Some(SizeCheckState::Done(info.clone()));
        if !off {
            history(run, i, now, history_text(&info));
        }
        let waiting = matches!(run.tasks[i].state, TaskState::Pending | TaskState::Queued);
        match info.decided {
            // Only a task still waiting to run is blocked: one already blocked (a
            // cancelled dependency, a failed setup) keeps its block, and the verdict
            // is only recorded (review I1). L is never a raise.
            Some(Size::L) if engine < Size::L => {
                if waiting {
                    let text = format!(
                        "the size cross-check judged this task L: {}; split it (rule 7.2.5)",
                        info.reason
                    );
                    block(run, i, BlockReason::MisSized, text, now);
                }
            }
            Some(size) if size > engine => apply_raise(run, &asked.id, size, &info.reason),
            _ => {}
        }
    }
}

fn history_text(info: &SizeCheckInfo) -> String {
    match (info.source, info.decided) {
        (DeciderSource::Decider, Some(size)) => format!(
            "size cross-check: {} by the decider, {} by the engine",
            size_label(size),
            size_label(info.engine)
        ),
        _ => format!("size cross-check: fallback ({})", info.reason),
    }
}

/// Decision 19's S → M raise: the size and `raised_size` (so an amend never lowers
/// it), what depends on the size re-resolved (`ladder::reresolve`: review level,
/// reviewer, a budget the plan did not set), and the re-resolved route: the new size's
/// row (milestone 9.8 decision 10), or the model the route names at the effort it
/// names, else the row's. The task has not been dispatched: a pending check blocks it.
pub(crate) fn apply_raise(run: &mut Run, task_id: &str, size: Size, reason: &str) {
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return;
    };
    let from = run.tasks[i].size;
    if size <= from {
        return;
    }
    run.tasks[i].size = size;
    run.tasks[i].raised_size = Some(size);
    let resolved = ladder::reresolve(run, i);
    let task = &mut run.tasks[i];
    // Milestone 9.8 decision 10 (controller ruling, M9.8.8): the raised size's row, its
    // model and effort; a route that names its model (a user's) keeps it, and an effort
    // the route names stays (`validate::resolve_route`).
    if task.route != resolved {
        task.route = resolved;
        let models = run.limits.models();
        task.review_route = (task.review_level).map(|_| models.reviewer_route(&task.route).0);
    }
    let note = format!(
        "size raised from {} to {}: decider cross-check (rule 7.2.5): {reason}",
        size_label(from),
        size_label(size)
    );
    if !task.notes.contains(&note) {
        task.notes.push(note);
    }
}
