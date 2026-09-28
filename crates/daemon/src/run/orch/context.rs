//! `get_context` (decision 17, Interfaces "The context"): the run and its triage, who
//! is asking, the profile, the limits, the roster with `installed`, the scout reports,
//! the epics and the plan so far, at most [`CONTEXT_MAX_BYTES`]. Pure: the driver
//! reads the reports and the stored profile and passes them in.

use proto::{RepoProfile, ScoutKind, ScoutReport};
use serde_json::{Value, json};

use super::json::{cut, fold_all, label, shrink_strings, size};
use super::{PlannerPhase, RunScout};
use crate::run::globs::any_intersect;
use crate::run::model::Run;
use crate::scout::report::ONBOARDING_ALIAS;

/// The context's cap, in bytes of compact JSON.
pub const CONTEXT_MAX_BYTES: usize = 96 * 1024;
/// A report's summary, in characters, and what trimming leaves of a later one.
pub const SUMMARY_MAX: usize = 8000;
pub const SUMMARY_TRIMMED: usize = 1000;

/// Spec §7.1's size rubric, as the planners read it.
const SIZE_S: &str =
    "one file, no interface change, a mechanical check exists, about 20 changed lines";
const SIZE_M: &str =
    "one to three files inside one module, a clear spec, a check exists, about 100 changed lines";
const SIZE_L: &str = "never executed: split it";

/// Who calls `get_context`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asker {
    Orchestrator,
    Planner { epic: String },
}

/// What the driver read for one answer: the stored profile and the run's scout
/// reports (the onboarding one included), and the call's `scouts` list.
pub struct ContextInputs<'a> {
    pub run: &'a Run,
    pub asker: Asker,
    pub profile: Option<&'a RepoProfile>,
    pub reports: Vec<ScoutReport>,
    pub only: Option<Vec<String>>,
}

/// The context for `inputs.asker`, trimmed to [`CONTEXT_MAX_BYTES`].
pub fn context(inputs: &ContextInputs<'_>) -> Value {
    let run = inputs.run;
    let epic = match &inputs.asker {
        Asker::Orchestrator => None,
        Asker::Planner { epic } => run.orch.epics.iter().find(|e| &e.epic == epic),
    };
    let planner_epic = match &inputs.asker {
        Asker::Planner { epic } => Some(epic.as_str()),
        Asker::Orchestrator => None,
    };
    let limits = &run.limits;
    let mut answer = json!({
        "run": {
            "id": run.id,
            "goal": run.goal,
            "path": run.path.map(|p| label(&p)),
            "state": run.state.label(),
            "root": run.root,
            "base_branch": run.base_branch,
            "base_sha": run.base_sha,
            "triage": run.triage.as_ref().map(|t| json!({
                "kinds": t.kinds, "scale": t.scale, "reason": t.reason,
            })),
        },
        "you": {
            "role": if planner_epic.is_some() { "planner" } else { "orchestrator" },
            "epic": epic.map(|e| json!({
                "epic": e.epic, "title": e.title, "area": e.area, "brief": e.brief,
            })),
        },
        "profile": {
            "summary": inputs.profile.map(crate::profile::summary),
            "modules": run.profile.modules,
            "hub": run.profile.hub,
            "source": run.profile.source,
            "generated": run.profile.generated,
            "protected": run.profile.protected,
            "check": run.profile.check,
            "single_test": run.profile.single_test,
        },
        "limits": {
            "planner_task_cap": limits.orch.planner_task_cap,
            "max_tasks": limits.max_tasks,
            "max_writers": limits.max_writers,
            "max_readers": limits.max_readers,
            "max_bounces": limits.max_bounces,
            "max_scouts": limits.orch.max_scouts,
            "sizes": {"S": SIZE_S, "M": SIZE_M, "L": SIZE_L},
        },
        "roster": run.roster.iter().map(|m| json!({
            "runtime": m.runtime.label(),
            "model": m.model,
            "strength": label(&m.strength),
            "note": m.note,
            "installed": run.orch.installed.get(m.runtime.label()).copied().unwrap_or(false),
        })).collect::<Vec<_>>(),
        "scouts": scouts(inputs, epic.map(|e| (e.scout_refs.as_slice(), e.area.as_slice()))),
        "epics": run.orch.epics.iter().map(|e| json!({
            "epic": e.epic,
            "title": e.title,
            "area": e.area,
            "state": match e.phase {
                // `PlannerState` has no queued (Implementation notes, refresh 26).
                PlannerPhase::Queued | PlannerPhase::Planning => "planning",
                PlannerPhase::Finished => "finished",
                PlannerPhase::Failed { .. } => "failed",
            },
            "tasks": run.tasks.iter().filter(|t| t.spec.epic.as_deref() == Some(&e.epic)).count(),
        })).collect::<Vec<_>>(),
        "plan": run.tasks.iter()
            .filter(|t| planner_epic.is_none() || t.spec.epic.is_none() || t.spec.epic.as_deref() == planner_epic)
            .map(|t| json!({
                "id": t.spec.id,
                "title": t.spec.title,
                "epic": t.spec.epic,
                "kind": label(&t.spec.kind),
                "size": label(&t.size),
                "hub": t.hub,
                "owns": t.spec.owns,
                "deps": t.spec.deps,
                "state": t.state.label(),
            })).collect::<Vec<_>>(),
        "omitted": {"scouts": 0},
    });
    fold_all(&mut answer);
    trim(&mut answer);
    answer
}

/// The onboarding report as `onboarding`, then every run scout in order (with its
/// report when there is one), then any other report; for a planner, only its
/// `scout_refs`, the reports whose area meets its epic's, and the onboarding one; with
/// `only`, only those ids.
fn scouts(inputs: &ContextInputs<'_>, epic: Option<(&[String], &[String])>) -> Vec<Value> {
    let run_scouts = &inputs.run.orch.run_scouts;
    let mut entries: Vec<(String, Vec<String>, Value)> = Vec::new();
    for report in inputs
        .reports
        .iter()
        .filter(|r| r.kind == ScoutKind::Onboarding)
    {
        entries.push((
            ONBOARDING_ALIAS.to_string(),
            Vec::new(),
            entry(
                ONBOARDING_ALIAS,
                "reported",
                &report.question,
                &[],
                Some(report),
            ),
        ));
    }
    for scout in run_scouts {
        let report = inputs.reports.iter().find(|r| r.id == scout.id);
        entries.push((
            scout.id.clone(),
            scout.area.clone(),
            scout_entry(scout, report),
        ));
    }
    for report in inputs
        .reports
        .iter()
        .filter(|r| r.kind != ScoutKind::Onboarding && !run_scouts.iter().any(|s| s.id == r.id))
    {
        entries.push((
            report.id.clone(),
            Vec::new(),
            entry(&report.id, "reported", &report.question, &[], Some(report)),
        ));
    }
    entries
        .into_iter()
        .filter(|(id, area, _)| match epic {
            None => true,
            Some((refs, epic_area)) => {
                id == ONBOARDING_ALIAS || refs.contains(id) || any_intersect(area, epic_area)
            }
        })
        .filter(|(id, _, _)| inputs.only.as_ref().is_none_or(|only| only.contains(id)))
        .map(|(_, _, value)| value)
        .collect()
}

fn scout_entry(scout: &RunScout, report: Option<&ScoutReport>) -> Value {
    entry(
        &scout.id,
        scout.state.label(),
        &scout.question,
        &scout.area,
        report,
    )
}

fn entry(
    id: &str,
    state: &str,
    question: &str,
    area: &[String],
    report: Option<&ScoutReport>,
) -> Value {
    json!({
        "id": id,
        "state": state,
        "question": question,
        "area": area,
        "summary": report.map(|r| cut(&r.summary, SUMMARY_MAX)),
        "files": report.map(|r| r.files.as_slice()).unwrap_or_default(),
        "modules": report.map(|r| r.modules.as_slice()).unwrap_or_default(),
        "interfaces": report.map(|r| r.interfaces.as_slice()).unwrap_or_default(),
        "risks": report.map(|r| r.risks.as_slice()).unwrap_or_default(),
    })
}

/// Decision 17's bounds past the cap: later reports' summaries cut to 1000
/// characters, then their file lists dropped, each report so shortened counted in
/// `omitted.scouts`. Beyond the decision, so the cap always holds: then their
/// modules, interfaces and risks, then every string cut to 300 characters, then later
/// reports left out (counted too).
fn trim(answer: &mut Value) {
    let fits = |a: &Value| size(a) <= CONTEXT_MAX_BYTES;
    if fits(answer) {
        return;
    }
    let count = answer["scouts"].as_array().map_or(0, Vec::len);
    let mut shortened = vec![false; count];
    let steps: [fn(&mut Value) -> bool; 3] = [cut_summary, drop_files, drop_lists];
    for step in steps {
        for i in (0..count).rev() {
            if fits(answer) {
                break;
            }
            shortened[i] |= step(&mut answer["scouts"][i]);
        }
    }
    if !fits(answer) {
        shrink_strings(answer, 300);
    }
    let mut dropped = 0;
    while !fits(answer) {
        match answer.get_mut("scouts") {
            Some(Value::Array(scouts)) if !scouts.is_empty() => {
                scouts.pop();
                dropped += 1;
            }
            _ => break,
        }
    }
    let kept = count - dropped;
    let omitted = shortened[..kept].iter().filter(|s| **s).count() + dropped;
    answer["omitted"]["scouts"] = json!(omitted);
}

/// A report's summary cut to [`SUMMARY_TRIMMED`] characters; true if it was longer.
fn cut_summary(scout: &mut Value) -> bool {
    match scout.get_mut("summary") {
        Some(Value::String(text)) if text.chars().nth(SUMMARY_TRIMMED).is_some() => {
            *text = cut(text, SUMMARY_TRIMMED);
            true
        }
        _ => false,
    }
}

fn drop_files(scout: &mut Value) -> bool {
    clear(scout, &["files"])
}

fn drop_lists(scout: &mut Value) -> bool {
    clear(scout, &["modules", "interfaces", "risks"])
}

/// Empties the lists `keys` of `scout`; true if one held anything.
fn clear(scout: &mut Value, keys: &[&str]) -> bool {
    let mut changed = false;
    for key in keys {
        if let Some(Value::Array(items)) = scout.get_mut(*key) {
            changed |= !items.is_empty();
            items.clear();
        }
    }
    changed
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
