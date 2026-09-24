//! Task resolution: decisions 8–10, 35's review level and 56's plan warning; the
//! cross-task rules (decisions 11–13) are in `validate_graph.rs`, re-exported here. Pure — no `std::fs`,
//! `std::process`, `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2).
//!
//! [`resolve_task`] works on one task alone; [`validate_tasks`] on the whole list, so a
//! plan edit (M8a.6) can re-run it over every unfinished task.

use std::collections::{BTreeMap, BTreeSet};

use proto::{
    Budget, Effort, ModelEntry, PlanTask, Route, Runtime, Size, Strength, TaskKind, TaskState,
    TestMode,
};

use super::globs::{ModuleSpan, OwnsMatcher, any_intersect, modules_spanned, validate_glob};
use super::model::{Profile, ReviewLevel, RunLimits, Task};
use super::plan::PlanError;
use super::roster;

pub use super::validate_graph::{
    EditScope, combined_cycles, implicit_deps, validate_tasks, validate_tasks_with,
};

const ID_PATTERN: &str = "^[a-z0-9][a-z0-9-]{0,15}$";
const ID_MAX: usize = 16;
/// The id of the run's own branch, `anthrex/<run>/integration` (decision 16).
const RESERVED_ID: &str = "integration";

pub(super) fn strength_label(s: Strength) -> &'static str {
    match s {
        Strength::Fast => "fast",
        Strength::Standard => "standard",
        Strength::Frontier => "frontier",
    }
}

fn size_label(s: Size) -> &'static str {
    match s {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

fn kind_label(k: TaskKind) -> &'static str {
    match k {
        TaskKind::Code => "code",
        TaskKind::Docs => "docs",
        TaskKind::Research => "research",
        TaskKind::Review => "review",
    }
}

fn is_valid_id(id: &str) -> bool {
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

/// Resolves one planned task: size rules (decision 9), route (decision 8), budget
/// (decision 40), test mode (decision 10) and review (decision 35). `branch` and
/// `worktree` are left empty for the caller, which knows the run id.
pub fn resolve_task(
    spec: PlanTask,
    profile: &Profile,
    limits: &RunLimits,
    roster: &[ModelEntry],
    default_runtime: Runtime,
) -> Result<Task, Vec<PlanError>> {
    let (task, errors) = resolve_task_lenient(spec, profile, limits, roster, default_runtime);
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
    roster: &[ModelEntry],
    default_runtime: Runtime,
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
        let (target, rule, why) = if spec.interface_change {
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
    let hub = any_intersect(&spec.owns, &profile.hub);
    if hub && size < Size::M {
        notes.push(format!(
            "size raised from {} to M: owns touch the hub globs (rule 7.2.3)",
            size_label(size)
        ));
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
    if hub && spec.kind == TaskKind::Code && declared != TestMode::Tdd {
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

    // Route, decision 8.
    let (class_strength, class_effort) = if hub {
        (Strength::Frontier, Effort::High)
    } else if size == Size::S {
        (Strength::Standard, Effort::Low)
    } else {
        (Strength::Standard, Effort::Medium)
    };
    let route = resolve_route(
        &spec,
        roster,
        default_runtime,
        class_strength,
        class_effort,
        &mut errors,
    );

    // Budget, decision 40.
    let budget = match spec.budget {
        Some(b) => {
            check_budget(&id, &b, &mut errors);
            b
        }
        None if size == Size::S && !hub => limits.budget_s,
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
    let review_route = review_level.map(|l| roster::pick_reviewer(roster, &route, l));

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
    }
    if matches!(spec.kind, TaskKind::Research | TaskKind::Review) {
        errors.push(e(
            "kind",
            "kind",
            format!(
                "{} tasks are executed from milestone 9; use code or docs",
                kind_label(spec.kind)
            ),
        ));
    }
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
    if spec.owns.is_empty() {
        errors.push(e(
            "owns",
            "fields",
            "at least one glob is required".to_string(),
        ));
    }
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

/// Decision 8: the planner's value wins when valid; policy fills the rest. On an error
/// the returned route is a best effort (the policy's model, or `""`).
fn resolve_route(
    spec: &PlanTask,
    roster: &[ModelEntry],
    default_runtime: Runtime,
    class_strength: Strength,
    class_effort: Effort,
    errors: &mut Vec<PlanError>,
) -> Route {
    let id = spec.id.as_str();
    let e = |field: &str, message: String| PlanError::new(Some(id), field, "route", message);
    let given = &spec.route;
    let mut runtime = given.runtime.unwrap_or(default_runtime);
    if runtime == Runtime::Shell {
        errors.push(e("route.runtime", "must be claude or codex".to_string()));
        runtime = default_runtime;
    }
    let effort = given.effort.unwrap_or(class_effort);
    let (model, strength) = match &given.model {
        Some(model) => match roster::find(roster, runtime, model) {
            Some(entry) => {
                if let Some(s) = given.strength
                    && s != entry.strength
                {
                    errors.push(e(
                        "route.strength",
                        format!(
                            "{model} is {} in the roster, not {}",
                            strength_label(entry.strength),
                            strength_label(s)
                        ),
                    ));
                }
                (model.clone(), entry.strength)
            }
            None => {
                errors.push(e(
                    "route.model",
                    format!("{model} is not in the roster for {runtime}"),
                ));
                (model.clone(), given.strength.unwrap_or(class_strength))
            }
        },
        None => {
            let strength = given.strength.unwrap_or(class_strength);
            match roster::first_at(roster, runtime, strength) {
                Some(entry) => (entry.model.clone(), strength),
                None => {
                    errors.push(e(
                        "route",
                        format!(
                            "the roster has no {runtime} model at {} strength",
                            strength_label(strength)
                        ),
                    ));
                    (String::new(), strength)
                }
            }
        }
    };
    Route {
        runtime,
        model,
        strength,
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
