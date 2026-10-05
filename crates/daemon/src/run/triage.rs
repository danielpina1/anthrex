//! Triage and the fast path (M8b decisions 22–24, spec §5.1). Pure (design decision 2):
//! no file, process, thread, async-runtime or clock access.
//!
//! The triage decider only proposes. [`route`] decides, deterministically, whether a goal
//! takes the fast path: a fallback, a disabled fast path, a `plan` or `large` scale, or a
//! kind other than exactly `code` or `docs` all route to the planned path, which is
//! refused until milestone 9. A `single` goal becomes a one-task [`Plan`] that goes
//! through M8a's whole start path; [`check_fast`] then refuses a task M8a's validation
//! made a hub task or L. The run keeps every gate and the user's accept.

use proto::{
    DeciderSource, Plan, PlanTask, ProfileSpec, RouteSpec, RunPath, Scale, Size, TaskKind,
    TriageInfo,
};

use super::contract::{mode_label, size_label};
use super::globs;
use super::model::{Run, Task};
use super::plan::PlanError;
use crate::decider::{DeciderAnswer, Decision, TriageAnswer};

/// The fast-path task's id.
pub const FAST_TASK_ID: &str = "t1";

/// `Run.approved_by` of a fast-path run: there is no plan gate.
pub const FAST_APPROVED_BY: &str = "fast path";

/// Triage's goal is cut to this many characters (decision 22 step 3).
pub const GOAL_CHARS: usize = 4000;
/// At most this many tracked paths reach the triage prompt …
pub const FILES_MAX: usize = 1500;
/// … and at most this many bytes of them (each path and its newline).
pub const FILES_BYTES: usize = 48 * 1024;

/// The goal as triage sees it: its first [`GOAL_CHARS`] characters.
pub fn goal_input(goal: &str) -> String {
    goal.chars().take(GOAL_CHARS).collect()
}

/// Decision 22 step 3: the tracked paths of a `git ls-files -z` listing, sorted, cut to
/// [`FILES_MAX`] paths or [`FILES_BYTES`] bytes, and how many there are in all.
pub fn tracked_files(listing: &str) -> (Vec<String>, u32) {
    let mut all: Vec<&str> = listing.split('\0').filter(|p| !p.is_empty()).collect();
    all.sort_unstable();
    all.dedup();
    let total = u32::try_from(all.len()).unwrap_or(u32::MAX);
    let mut bytes = 0;
    let mut kept = Vec::new();
    for path in all.into_iter().take(FILES_MAX) {
        bytes += path.len() + 1;
        if bytes > FILES_BYTES {
            break;
        }
        kept.push(path.to_string());
    }
    (kept, total)
}

/// Where a goal goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriageRoute {
    Fast(Box<PlanTask>),
    Plan { reason: String },
    Large { reason: String },
}

/// `code`, `docs`, `research` or `review`.
pub fn kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Code => "code",
        TaskKind::Docs => "docs",
        TaskKind::Research => "research",
        TaskKind::Review => "review",
    }
}

pub fn scale_label(scale: Scale) -> &'static str {
    match scale {
        Scale::Single => "single",
        Scale::Plan => "plan",
        Scale::Large => "large",
    }
}

pub fn source_label(source: DeciderSource) -> &'static str {
    match source {
        DeciderSource::Decider => "decider",
        DeciderSource::Fallback => "fallback",
    }
}

/// `<kinds joined with ,>/<scale>`, as `code/single`.
pub fn kinds_scale(kinds: &[TaskKind], scale: Scale) -> String {
    let kinds: Vec<&str> = kinds.iter().map(|k| kind_label(*k)).collect();
    format!("{}/{}", kinds.join(","), scale_label(scale))
}

/// The triage answer of `decision`, or the deterministic fallback's shape when the
/// decision carries some other kind (never, from `decide`).
fn answer(decision: &Decision) -> TriageAnswer {
    match &decision.answer {
        DeciderAnswer::Triage(answer) => answer.clone(),
        _ => TriageAnswer {
            kinds: vec![TaskKind::Code],
            scale: Scale::Plan,
            reason: "the triage decision holds no triage answer".into(),
            task: None,
        },
    }
}

/// Decision 23, in order.
pub fn route(decision: &Decision, fast_path: bool) -> TriageRoute {
    if decision.source == DeciderSource::Fallback {
        let why = decision.fallback_reason.as_deref().unwrap_or("no reason");
        return TriageRoute::Plan {
            reason: format!("triage fell back ({why}); without a decider the path is plan"),
        };
    }
    if !fast_path {
        return TriageRoute::Plan {
            reason: "the fast path is disabled ([orchestrator] fast_path = false)".into(),
        };
    }
    let answer = answer(decision);
    match answer.scale {
        Scale::Large => {
            return TriageRoute::Large {
                reason: answer.reason,
            };
        }
        Scale::Plan => {
            return TriageRoute::Plan {
                reason: answer.reason,
            };
        }
        Scale::Single => {}
    }
    let kind = match answer.kinds.as_slice() {
        [kind @ (TaskKind::Code | TaskKind::Docs)] => *kind,
        kinds => {
            let labels: Vec<&str> = kinds.iter().map(|k| kind_label(*k)).collect();
            return TriageRoute::Plan {
                reason: format!(
                    "a single-task goal of kinds {} is not a fast-path goal",
                    labels.join(",")
                ),
            };
        }
    };
    let Some(task) = answer.task else {
        return TriageRoute::Plan {
            reason: "the triage answer names no task".into(),
        };
    };
    TriageRoute::Fast(Box::new(PlanTask {
        id: FAST_TASK_ID.into(),
        title: task.title,
        epic: None,
        kind,
        size: task.size,
        interface_change: task.interface_change,
        test_mode: Some(task.test_mode),
        test_mode_reason: task.test_mode_reason,
        owns: task.owns,
        deps: Vec::new(),
        priority: 0,
        brief: task.brief,
        acceptance: task.acceptance,
        test_to_write: task.test_to_write,
        scout_refs: Vec::new(),
        route: RouteSpec::default(),
        budget: None,
        review_target: None,
        stage: 1,
        atomic: false,
        atomic_reason: None,
        addresses: Vec::new(),
        race: false,
        pair: false,
        covers: Vec::new(),
    }))
}

/// What triage decided, with the route's path and reason (the decider's own reason for
/// the fast path).
pub fn info(decision: &Decision, route: &TriageRoute, now: u64) -> TriageInfo {
    let answer = answer(decision);
    let (path, reason) = match route {
        TriageRoute::Fast(_) => (RunPath::Fast, answer.reason),
        TriageRoute::Plan { reason } => (RunPath::Plan, reason.clone()),
        TriageRoute::Large { reason } => (RunPath::Large, reason.clone()),
    };
    TriageInfo {
        kinds: answer.kinds,
        scale: answer.scale,
        path,
        reason,
        source: decision.source,
        fallback_reason: decision.fallback_reason.clone(),
        at: now,
    }
}

/// The fast path's one-task plan (decision 22 step 5).
pub fn fast_plan(goal: &str, task: PlanTask, profile: ProfileSpec) -> Plan {
    Plan {
        goal: goal.to_string(),
        max_writers: None,
        max_readers: None,
        max_bounces: None,
        profile,
        tasks: vec![task],
    }
}

/// `the fast path does not apply: <why>`.
fn not_applicable(why: &str) -> String {
    format!("the fast path does not apply: {why}")
}

/// Decision 22 step 5's checks on what M8a's `build_run` made of the fast plan: its
/// first error, a hub task or an L task routes the goal to the planned path, with that
/// reason (`Err`).
pub fn check_fast(built: Result<Run, Vec<PlanError>>) -> Result<Run, String> {
    let run = match built {
        Ok(run) => run,
        Err(errors) => {
            let first = errors
                .first()
                .map_or_else(|| "the plan did not build".to_string(), ToString::to_string);
            return Err(not_applicable(&first));
        }
    };
    if let Some(reason) = fast_refusal(&run) {
        return Err(reason);
    }
    Ok(run)
}

/// The fast path's invariant (decision 24, review I1): exactly one task, neither hub nor
/// L. `None` when `tasks` meet it, else the reason. The driver's `build_plan`,
/// [`check_fast`] and the engine's `start` all apply it.
pub fn fast_refusal(run: &Run) -> Option<String> {
    let tasks = &run.tasks;
    let [task] = tasks.as_slice() else {
        return Some(not_applicable(&format!(
            "a fast-path run has exactly one task, not {}",
            tasks.len()
        )));
    };
    if task.hub {
        return Some(not_applicable(&format!(
            "task {} touches a hub file",
            task.id()
        )));
    }
    if task.size == Size::L {
        return Some(not_applicable(&format!("task {} is L", task.id())));
    }
    if let Some(path) = owned_protected(
        &task.spec.owns,
        &run.profile.protected,
        &run.protected_files,
    ) {
        return Some(not_applicable(&format!(
            "task {} owns a protected file ({path})",
            task.id()
        )));
    }
    None
}

/// Whole-branch review I1: the first protected path `owns` names or covers, else
/// `None`. A fast-path task has no plan the user approves, so decision 56's grant (a
/// protected file changes when `owns` names it) must not reach a worker unseen. In
/// order: a literal entry decision 56's matcher protects (ignoring case, as its done
/// gate does), then a tracked protected file (`protected_files`) an entry covers, then a
/// `protected` entry an entry may cover at the repository's root
/// ([`globs::may_cover_protected`]). `protected` is the run's frozen list: the
/// built-ins, then the configuration's, the stored profile's and the plan's additions.
pub fn owned_protected(
    owns: &[String],
    protected: &[String],
    protected_files: &[String],
) -> Option<String> {
    // Whole-branch re-review N1: a case-insensitive file system folds some non-ASCII
    // letters onto ASCII (`ſ` opens as `s`), which the ASCII-folding matcher cannot
    // see, so a fast-path task never owns a non-ASCII path.
    if let Some(entry) = owns.iter().find(|entry| !entry.is_ascii()) {
        return Some(entry.clone());
    }
    let literal = |entry: &&String| !entry.contains(['*', '?', '[']);
    if let Ok(matcher) = globs::ProtectedMatcher::new(protected)
        && let Some(entry) = owns
            .iter()
            .filter(literal)
            .find(|entry| matcher.matches(entry.trim_end_matches('/')))
    {
        return Some(entry.clone());
    }
    if let Ok(matcher) = globs::OwnsMatcher::new(owns)
        && let Some(path) = protected_files.iter().find(|path| matcher.matches(path))
    {
        return Some(path.clone());
    }
    protected
        .iter()
        .find(|entry| owns.iter().any(|o| globs::may_cover_protected(o, entry)))
        .cloned()
}

/// Review m2: a blank goal is refused before triage spends a call, with `build_run`'s
/// own wording (`goal: must not be blank`).
pub fn blank_goal(goal: &str) -> Option<String> {
    goal.trim()
        .is_empty()
        .then(|| PlanError::new(None, "goal", "fields", "must not be blank").to_string())
}

/// `info` for a goal the fast path turned out not to apply to: the planned path.
pub fn not_fast(mut info: TriageInfo, reason: String) -> TriageInfo {
    info.path = RunPath::Plan;
    info.reason = reason;
    info
}

/// Makes a built run the fast-path run (decisions 22 and 24): no plan gate.
pub fn mark_fast(run: &mut Run, info: TriageInfo, usage: Option<proto::TokenUsage>) {
    run.path = Some(RunPath::Fast);
    run.triage = Some(info);
    run.triage_usage = usage.unwrap_or_default();
    run.approved_by = Some(FAST_APPROVED_BY.to_string());
}

/// `decider` or `fallback: <reason>`.
fn source_text(info: &TriageInfo) -> String {
    match info.source {
        DeciderSource::Decider => "decider".to_string(),
        DeciderSource::Fallback => format!(
            "fallback: {}",
            info.fallback_reason.as_deref().unwrap_or("no reason")
        ),
    }
}

/// The fast path's answer to `run start --goal` (Interfaces, exact).
pub fn started_message(info: &TriageInfo, run_id: &str, task: &Task) -> String {
    format!(
        "triage: {} ({})\nfast path: one task, no plan gate\n  {}  {}  {}  {}\nwatch with: anthrex run status {run_id}",
        kinds_scale(&info.kinds, info.scale),
        source_text(info),
        task.id(),
        size_label(task.size),
        mode_label(task.test_mode),
        task.spec.title
    )
}

#[cfg(test)]
#[path = "triage_tests.rs"]
mod tests;
