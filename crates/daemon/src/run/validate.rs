//! Task resolution: decisions 8–10, 35's review level and 56's plan warning; the
//! cross-task rules (decisions 11–13) are in `validate_graph.rs`, re-exported here. Pure — no `std::fs`,
//! `std::process`, `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2).
//!
//! [`resolve_task`] works on one task alone; [`validate_tasks`] on the whole list, so a
//! plan edit (M8a.6) can re-run it over every unfinished task.

use std::collections::{BTreeMap, BTreeSet};

use proto::{Budget, PlanTask, Route, Runtime, Size, TaskKind, TaskState, TestMode};

use super::globs::{ModuleSpan, OwnsMatcher, any_intersect, modules_spanned, validate_glob};
use super::model::{Profile, ReviewLevel, RunLimits, Task};
use super::model_roles::RunModels;
use super::plan::PlanError;
use super::validate_kinds::{READER_TEST_MODE_NOTE, check_reader_fields, is_reader};
use super::validate_stages::check_stage_fields;
pub(super) use super::validate_stages::{reserved_new_id, split_child_stage};

pub use super::validate_graph::{
    EditScope, combined_cycles, implicit_deps, validate_tasks, validate_tasks_with,
};

const ID_PATTERN: &str = "^[a-z0-9][a-z0-9-]{0,15}$";
const ID_MAX: usize = 16;
/// The id of the run's own branch, `anthrex/<run>/integration` (decision 16).
const RESERVED_ID: &str = "integration";
/// Milestone 9's message target for every running task (`proto::MessageTarget::Running`,
/// M9.2 review ruling 5). `stage:<n>` needs no reservation: an id cannot hold `:`.
const RUNNING_ID: &str = "running";

fn size_label(s: Size) -> &'static str {
    match s {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

pub(crate) fn is_valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= ID_MAX
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// Ruling T13-minors (m1): the note on a tdd task whose profile has no `test_passed`.
/// The proof then asks for the test's name on a line of the head run's output
/// (M8a.13), which a runner that does not echo test names never shows.
pub const NO_TEST_PASSED_NOTE: &str = "the profile has no test_passed: the test proof will require the test's name in the single-test command's output (rule 8.1)";

/// Resolves one planned task: size rules (decision 9), route (milestone 9.8 decision
/// 10: the task's row of the run's role table), budget (decision 40), test mode
/// (decision 10) and review (decision 35; its reviewer is the reviewer row's). `branch` and
/// `worktree` are left empty for the caller, which knows the run id.
pub fn resolve_task(
    spec: PlanTask,
    profile: &Profile,
    limits: &RunLimits,
) -> Result<Task, Vec<PlanError>> {
    let (task, errors) = resolve_task_lenient(spec, profile, limits);
    if errors.is_empty() {
        Ok(task)
    } else {
        Err(errors)
    }
}

/// [`resolve_task`], but always yielding a best-effort task beside its errors, so the
/// cross-task rules can still see every task (a task with a bad route must still count
/// as a dependency target).
pub(super) fn resolve_task_lenient(
    spec: PlanTask,
    profile: &Profile,
    limits: &RunLimits,
) -> (Task, Vec<PlanError>) {
    let id = spec.id.clone();
    let mut errors = Vec::new();
    let mut notes = Vec::new();
    let e =
        |field: &str, rule: &str, message: String| PlanError::new(Some(&id), field, rule, message);

    check_fields(&spec, &mut errors);

    // Size rules, decision 9.
    let mut size = spec.size;
    let (module_count, spans_many) = module_span(&spec.owns, &profile.modules);
    let spans = if spans_many || module_count >= 2 {
        Some(if spans_many {
            "owns spans more than one module".to_string()
        } else {
            format!("owns spans {module_count} modules")
        })
    } else {
        None
    };
    if let Some(span) = &spans {
        // Decision 54: an atomic task keeps its size; it is a hub task instead.
        let (target, rule, why) = if spec.interface_change && !spec.atomic {
            (
                Size::L,
                "7.2.2",
                format!("{span} and interface_change is set"),
            )
        } else {
            (Size::M, "7.2.1", span.clone())
        };
        if size < target {
            notes.push(format!(
                "size raised from {} to {}: {why} (rule {rule})",
                size_label(size),
                size_label(target)
            ));
            size = target;
        }
    }
    let hub = spec.atomic || any_intersect(&spec.owns, &profile.hub);
    if hub && size < Size::M {
        let why = if spec.atomic {
            "an atomic task is a hub task (rule 4.3)"
        } else {
            "owns touch the hub globs (rule 7.2.3)"
        };
        notes.push(format!("size raised from {} to M: {why}", size_label(size)));
        size = Size::M;
    }

    // Test mode, decision 10.
    let touches_source = any_intersect(&spec.owns, &profile.source);
    let kind_default = if spec.kind == TaskKind::Code {
        TestMode::Tdd
    } else {
        TestMode::None
    };
    let declared = spec.test_mode.unwrap_or(kind_default);
    let mut test_mode = declared;
    if is_reader(spec.kind) {
        // Decision 24: forced, and a given reason is kept but none is required.
        test_mode = TestMode::None;
        notes.push(READER_TEST_MODE_NOTE.to_string());
    } else if hub && spec.kind == TaskKind::Code && declared != TestMode::Tdd {
        test_mode = TestMode::Tdd;
        notes.push("test mode forced to tdd: hub task (rule 8.2)".to_string());
    } else {
        let reason_blank = spec
            .test_mode_reason
            .as_deref()
            .is_none_or(|r| r.trim().is_empty());
        if declared != TestMode::Tdd && reason_blank {
            errors.push(e(
                "test_mode_reason",
                "8",
                "required when test_mode is check or none".to_string(),
            ));
        }
        if spec.kind == TaskKind::Code && declared == TestMode::None && touches_source {
            errors.push(e(
                "test_mode",
                "8.1",
                "a code task whose owns touch the profile's source globs cannot be none (rule 8.1)"
                    .to_string(),
            ));
        }
    }
    let mut rule_8_3 = false;
    if test_mode == TestMode::Tdd && profile.single_test.is_none() {
        test_mode = TestMode::Check;
        rule_8_3 = true;
        notes.push("test mode check: the profile has no single_test (rule 8.3)".to_string());
    }
    if test_mode == TestMode::Tdd && profile.test_passed.is_none() {
        notes.push(NO_TEST_PASSED_NOTE.to_string());
    }

    // Route, milestone 9.8 decision 10: the task's row of the run's role table.
    let models = limits.models();
    let row = models.route(RunModels::role_of(spec.kind, hub, size));
    let route = resolve_route(&spec, row, &mut errors);

    // Budget, decision 40.
    let budget = match spec.budget {
        Some(b) => {
            check_budget(&id, &b, &mut errors);
            b
        }
        None if size == Size::S && !hub => limits.budget_s,
        None if hub => limits.budget_hub.unwrap_or(limits.budget_m),
        None => limits.budget_m,
    };

    // Review, decision 35.
    let base = if hub {
        ReviewLevel::Frontier
    } else if size == Size::S {
        ReviewLevel::Small
    } else {
        ReviewLevel::Medium
    };
    let raise =
        profile.check.is_none() || (test_mode != TestMode::Tdd && touches_source) || rule_8_3;
    let level = if raise { base.raised() } else { base };
    let skipped = !limits.review_small && !hub && size == Size::S && level == ReviewLevel::Small;
    let review_level = (!skipped).then_some(level);
    // Milestone 9.8 decision 27: the reviewer row against the route (the level only
    // decides whether the task is reviewed).
    let review_route = review_level.map(|_| models.reviewer_route(&route).0);

    let task = new_task(
        spec,
        size,
        hub,
        test_mode,
        notes,
        review_level,
        route,
        review_route,
        budget,
    );
    (task, errors)
}

/// How many distinct modules `owns` names, and whether any one glob spans more than one
/// by itself (decision 9's "Modules").
fn module_span(owns: &[String], modules: &[String]) -> (usize, bool) {
    let mut names = BTreeSet::new();
    let mut many = false;
    for glob in owns {
        match modules_spanned(std::slice::from_ref(glob), modules) {
            ModuleSpan::One(name) => {
                names.insert(name);
            }
            ModuleSpan::Many => many = true,
        }
    }
    (names.len(), many)
}

fn check_fields(spec: &PlanTask, errors: &mut Vec<PlanError>) {
    let id = spec.id.as_str();
    let e =
        |field: &str, rule: &str, message: String| PlanError::new(Some(id), field, rule, message);
    if !is_valid_id(id) {
        errors.push(e("id", "id", format!("must match {ID_PATTERN}")));
    } else if id == RESERVED_ID {
        errors.push(e(
            "id",
            "id",
            "integration is reserved for the run branch".to_string(),
        ));
    } else if id == RUNNING_ID {
        errors.push(e(
            "id",
            "id",
            "running is reserved for the message target of every running task".to_string(),
        ));
    }
    // Milestone 9.1 decisions 44, 45 and 54.
    check_stage_fields(spec, errors);
    if spec.title.trim().is_empty() {
        errors.push(e("title", "fields", "must not be blank".to_string()));
    }
    if spec.brief.trim().is_empty() {
        errors.push(e("brief", "fields", "must not be blank".to_string()));
    }
    if spec.acceptance.is_empty() {
        errors.push(e(
            "acceptance",
            "fields",
            "at least one item is required".to_string(),
        ));
    }
    for (i, item) in spec.acceptance.iter().enumerate() {
        if item.trim().is_empty() {
            errors.push(e(
                "acceptance",
                "fields",
                format!("item {} must not be blank", i + 1),
            ));
        }
    }
    // M8a's `owns` requirement, for code and docs tasks only (decision 24).
    if spec.owns.is_empty() && !is_reader(spec.kind) {
        errors.push(e(
            "owns",
            "fields",
            "at least one glob is required".to_string(),
        ));
    }
    check_reader_fields(spec, errors);
    for glob in &spec.owns {
        if let Err(msg) = validate_glob(glob) {
            errors.push(e("owns", "globs", format!("{glob} {msg}")));
        }
    }
}

fn check_budget(id: &str, b: &Budget, errors: &mut Vec<PlanError>) {
    let low = |field: &str| PlanError::new(Some(id), field, "range", "must be at least 1");
    if b.tool_calls < 1 {
        errors.push(low("budget.tool_calls"));
    }
    if b.minutes < 1 {
        errors.push(low("budget.minutes"));
    }
    if b.tokens == Some(0) {
        errors.push(low("budget.tokens"));
    }
}

/// Milestone 9.8 decision 10: the task's row (`row`), unless the route names a model (a
/// user's amend, or an engine fix task's step up): then that model on the route's
/// runtime (the row's when it names none), as given: no roster checks it since
/// `Run.roster` went (task M9.8.13). Either way at the route's effort when it names one
/// (the task edit form's effort over the row's model), else the row's for the row's own
/// model, and the named model's default for another (M9.8.11 fix round 1).
fn resolve_route(spec: &PlanTask, row: Route, errors: &mut Vec<PlanError>) -> Route {
    let id = spec.id.as_str();
    let e = |field: &str, message: String| PlanError::new(Some(id), field, "route", message);
    let given = &spec.route;
    if given.runtime == Some(Runtime::Shell) {
        errors.push(e("route.runtime", "must be claude or codex".to_string()));
    }
    let Some(model) = &given.model else {
        let effort = given.effort.clone().unwrap_or(row.effort.clone());
        return Route { effort, ..row };
    };
    // M9.8.13 fix round 1 (I1): with no roster to check it, the name rule alone keeps a
    // bad model off `-m`/`--model`; `""` is the runtime's default model.
    if let Err(problem) = proto::models::model_id_problem(model)
        && !model.is_empty()
    {
        errors.push(e("route.model", problem));
    }
    let runtime = (given.runtime)
        .filter(|r| *r != Runtime::Shell)
        .unwrap_or(row.runtime);
    // M9.8.11 fix round 1 (controller ruling): another model than the row's, with no
    // effort, runs at its own default, never at the row's (it may not offer it); the
    // row's own model keeps the row's, as the goal form's choice does.
    let own = runtime == row.runtime && *model == row.model;
    let effort = given.effort.clone().unwrap_or(if own {
        row.effort.clone()
    } else {
        proto::Effort::DEFAULT
    });
    // M9.8.14: an old route's `strength` is read and ignored.
    Route {
        runtime,
        model: model.clone(),
        effort,
    }
}

#[allow(clippy::too_many_arguments)]
fn new_task(
    spec: PlanTask,
    size: Size,
    hub: bool,
    test_mode: TestMode,
    notes: Vec<String>,
    review_level: Option<ReviewLevel>,
    route: Route,
    review_route: Option<Route>,
    budget: Budget,
) -> Task {
    Task {
        spec,
        size,
        hub,
        test_mode,
        notes,
        review_level,
        route,
        review_route,
        budget,
        implicit_deps: Vec::new(),
        state: TaskState::Pending,
        block: None,
        rung: 0,
        raised_size: None,
        failures: 0,
        bounces: Default::default(),
        stalls: 0,
        budget_exceeded: 0,
        conflicts: 0,
        session: 0,
        spent_total: Default::default(),
        branch: String::new(),
        worktree: Default::default(),
        prewarmed: false,
        worktree_live: false,
        prepare_failed: false,
        removal_due: false,
        awaiting_deps: false,
        held_answered: false,
        gate_op: None,
        review_misses: 0,
        merge_op: None,
        cancel_deferred: false,
        ready_from: None,
        resolving: false,
        handback_due: false,
        gates_after_handback: false,
        override_count: None,
        clock: Default::default(),
        epoch: None,
        resolution: None,
        start_commit: None,
        head: None,
        done: None,
        claim: None,
        fresh_session: None,
        rounds: Vec::new(),
        reviews: Vec::new(),
        checks: Vec::new(),
        proofs: Vec::new(),
        handed_back: false,
        merge_commit: None,
        merged_without_approval: None,
        salvage_refs: Vec::new(),
        failure_log: Vec::new(),
        history: Vec::new(),
        pending_failure: None,
        pending_classification: None,
        block_source: None,
        decider_usage: Default::default(),
        size_check: None,
        phases: Default::default(),
        phase_since: 0,
        max_rung: 0,
        diff: None,
        history_written: false,
        routing_decisions: Vec::new(),
        escalated_from: None,
        orch: Default::default(),
        origin: proto::TaskOrigin::Plan,
        fixes: None,
        signals: Vec::new(),
        signals_more: 0,
        signal_refusals: 0,
        sync: None,
        round: proto::first_round(),
        race: None,
        pair: None,
        race_wait_since: None,
        race_decision: None,
        paused: Default::default(),
        lane_view: None,
        parked_readers: 0,
        lane_unclassified: false,
        salvage_seq: 0,
    }
}

/// Decision 56's plan warning: for each tracked protected file (in the order given)
/// that `owns` matches without naming it literally, one note naming the first `owns`
/// entry that matches it.
pub fn protected_notes(owns: &[String], protected_files: &[String]) -> Vec<String> {
    let matchers: BTreeMap<&str, OwnsMatcher> = owns
        .iter()
        .filter_map(|g| {
            OwnsMatcher::new(std::slice::from_ref(g))
                .ok()
                .map(|m| (g.as_str(), m))
        })
        .collect();
    let mut notes = Vec::new();
    for path in protected_files {
        if super::globs::names_literally(owns, path) {
            continue;
        }
        if let Some(glob) = owns
            .iter()
            .find(|g| matchers.get(g.as_str()).is_some_and(|m| m.matches(path)))
        {
            notes.push(format!(
                "owns {glob} covers protected {path}; name it exactly in owns if this task must change it (rule 6.protected)"
            ));
        }
    }
    notes
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
