//! Decision 26 for a red CI run (task M9.2.9): the fix task, made by 9.1's one function
//! for engine-made tasks (`fixes::add_fix`), from Interfaces' `ci` template (exact),
//! which quotes the log as data (decision 22); or the red handed to the user: decision
//! 27 step 6's cap, or a fix task the plan rules refuse on every route tried. A
//! culprit's fix copies its `owns` exactly and takes its route one rung up (9.1's
//! bisect rule); any other fix owns the union of the stage's approved tasks' `owns`
//! (so it needs no approval hold: every entry is covered by the stage) and takes the
//! stage's strongest route. Every CI fix counts against `ci_fix_max`, never against
//! 9.1's `bisect_fix_max`. Pure (design decision 2).

use proto::{CiCategory, Route, RouteSpec, Size, TaskOrigin, TaskState, TestMode};

use super::super::history::BisectResult;
use super::super::requests::log;
use super::super::{Effect, fixes, gate_holds, wake};
use super::ci::{log_text, record, record_mut};
use crate::decider::ci::category_label;
use crate::run::contract::sha7;
use crate::run::delivery::quote;
use crate::run::delivery::{CiPhase, CiRecord};
use crate::run::model::{BisectRecord, FixOf, Run, Task};
use crate::run::model_roles::Mover;
use crate::run::route_pick::escalate_for;

/// Decision 26's reason for a CI fix task's `check` test mode.
pub(crate) const CI_FIX_TEST_MODE_REASON: &str = "fix task: the failing checks are the proof";
/// TT §6.4's sentence for a red that does not reproduce locally.
pub(crate) const NOT_REPRODUCED: &str =
    "This failure does not reproduce locally; the difference is in CI's environment. Find it.";
/// The template quotes the log's last this many lines, and a culprit's `show --stat`
/// to this many lines.
pub(super) const LOG_LINES: usize = 200;
const STAT_LINES: usize = 60;
/// The characters of a CI key an attention line shows.
const KEY_SHOWN: usize = 120;

/// Whether, and how, the red reproduced locally (decision 27 step 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Repro {
    /// Red with this command.
    Reproduced(String),
    /// Green: TT's environment sentence.
    NotReproduced,
    /// Nothing could run it, for this reason (task M9.2.9: the template has no text
    /// for it, so this one is the brief's).
    Unavailable(String),
}

/// The task a bisect blamed, as the brief quotes it.
struct Culprit<'a> {
    id: &'a str,
    title: &'a str,
    brief: &'a str,
    show: Option<&'a str>,
}

fn category(rec: &CiRecord) -> &'static str {
    category_label(rec.category.unwrap_or(CiCategory::Unknown))
}

/// `Fix CI on stage <n>: <first failing test, else CI check 1>`. A test name passed
/// decision 18's filter; a check's name never reaches a title (the final fix wave's I-6).
fn title(n: u16, rec: &CiRecord) -> String {
    match rec.failing_tests.first() {
        Some(test) => format!("Fix CI on stage {n}: {test}"),
        None => format!("Fix CI on stage {n}: {}", quote::checks_ref(1)),
    }
}

/// I-6: the checks by number; their names are in the brief's fenced list.
fn acceptance(rec: &CiRecord) -> Vec<String> {
    vec![
        format!(
            "The failing checks pass: {}.",
            quote::checks_ref(rec.checks.len())
        ),
        "No test is deleted or skipped to make them pass.".to_string(),
    ]
}

/// The last `k` lines of `text`.
pub(super) fn last_lines(text: &str, k: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(k)..].join("\n")
}

/// Interfaces' `ci` template, exactly, then the two lines every fix brief ends with.
fn brief(run: &Run, n: u16, rec: &CiRecord, repro: &Repro, culprit: Option<&Culprit>) -> String {
    let checks = quote::checks_ref(rec.checks.len());
    let repro = match repro {
        Repro::Reproduced(command) => format!("It reproduces locally with: {command}."),
        Repro::NotReproduced => NOT_REPRODUCED.to_string(),
        Repro::Unavailable(why) => format!("It was not reproduced locally: {why}."),
    };
    let blame = match culprit {
        Some(c) => format!(
            "Bisect found the merge of task {} ({}) as the first red; its brief follows.",
            c.id, c.title
        ),
        None => "No single task's merge is the cause.".to_string(),
    };
    let source = rec.source.as_deref().unwrap_or("none");
    // I-6: the checks' names only inside their fenced list.
    let mut out = format!(
        "CI failed on stage {n}'s pull request, at {}, in: {checks}.\n{}Category: {}. {repro}\n{blame}\nSummary of the failure (a decider's summary of the log, {source}):\n",
        sha7(&rec.head),
        quote::checks(&rec.checks),
        category(rec)
    );
    // Decision 22 (fix round 1): a decider's own lines (its words, one-lined and cut)
    // stand outside the fence by design; a fallback's are the log's own lines, which
    // the quote below holds, so they are not repeated outside it.
    if rec.source.as_deref() == Some("decider") {
        for line in &rec.lines {
            let line: String = proto::safe_text::one_line(line).chars().take(300).collect();
            out.push_str(&format!("  {line}\n"));
        }
    } else if !rec.lines.is_empty() {
        out.push_str("  (none: see the quoted log)\n");
    }
    out.push_str(&quote::ci_log(
        &checks,
        &last_lines(&log_text(rec), LOG_LINES),
    ));
    if let Some(c) = culprit {
        out.push_str(&format!("Task {}'s brief:\n{}\n", c.id, c.brief));
        let stat = last_lines(c.show.unwrap_or("(not available)"), STAT_LINES);
        let stat: String = stat.lines().take(STAT_LINES).collect::<Vec<_>>().join("\n");
        out.push_str(&format!("git show --stat of its merge:\n{stat}\n"));
    }
    let url = run.delivery.pr(n).map_or("", |p| p.url.as_str());
    out.push_str(&format!(
        "Stage {n}'s pull request: {url}.\nYour commits reach the pull request after tier 1, tier 2 and the merge queue; do not push, open or merge anything yourself."
    ));
    out
}

pub(super) fn route_spec(route: &Route) -> RouteSpec {
    RouteSpec {
        runtime: Some(route.runtime),
        model: Some(route.model.clone()),
        strength: Some(route.strength),
        effort: Some(route.effort.clone()),
    }
}

/// The stage's approved tasks (plan-approved, or released from their hold), not
/// cancelled, in plan order.
pub(super) fn approved(run: &Run, n: u16) -> Vec<&Task> {
    (run.tasks.iter())
        .filter(|t| t.stage() == n && t.state != TaskState::Cancelled)
        .filter(|t| gate_holds::released(run, t))
        .collect()
}

/// The union of the stage's approved `owns`, in plan order.
pub(super) fn stage_owns(run: &Run, n: u16) -> Vec<String> {
    let mut owns: Vec<String> = Vec::new();
    for glob in approved(run, n).into_iter().flat_map(|t| &t.spec.owns) {
        if !owns.contains(glob) {
            owns.push(glob.clone());
        }
    }
    owns
}

/// The stage's strongest route: the highest strength, the first task in plan order on
/// a tie (decision 26).
pub(super) fn strongest(run: &Run, n: u16) -> Option<Route> {
    let mut best: Option<&Route> = None;
    for t in approved(run, n) {
        if best.is_none_or(|b| t.route.strength > b.strength) {
            best = Some(&t.route);
        }
    }
    best.cloned()
}

fn spec(n: u16, rec: &CiRecord, owns: Vec<String>, epic: Option<String>) -> fixes::FixSpec {
    fixes::FixSpec {
        origin: TaskOrigin::Ci,
        fixes: FixOf::Ci {
            stage: n,
            head: rec.head.clone(),
            ci_runs: rec.ci_runs.clone(),
            key: rec.key.clone(),
        },
        stage: n,
        title: title(n, rec),
        brief: String::new(),
        acceptance: acceptance(rec),
        owns,
        size: Size::S,
        epic,
        route: RouteSpec::default(),
        test_mode: TestMode::Check,
        test_mode_reason: Some(CI_FIX_TEST_MODE_REASON.to_string()),
        sync: None,
    }
}

/// `add_fix` on each route in turn (decision 26: a route the plan rules refuse is
/// followed by the next); the first message when every one is refused.
pub(super) fn add_on(
    run: &mut Run,
    spec: fixes::FixSpec,
    routes: &[RouteSpec],
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let mut first = None;
    for route in routes {
        let attempt = fixes::FixSpec {
            route: route.clone(),
            ..spec.clone()
        };
        match fixes::add_fix(run, attempt, now, fx) {
            Ok(id) => return Ok(id),
            Err(message) => {
                first.get_or_insert(message);
            }
        }
    }
    Err(first.unwrap_or_default())
}

/// A stage fix for record `i` of stage `n`: the union of the stage's approved `owns`,
/// its strongest route (then the policy's), and the template's brief.
pub(super) fn add_stage(
    run: &mut Run,
    n: u16,
    i: usize,
    repro: Repro,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(rec) = record(run, n, i).cloned() else {
        return;
    };
    let mut spec = spec(n, &rec, stage_owns(run, n), None);
    spec.brief = brief(run, n, &rec, &repro, None);
    let mut routes: Vec<RouteSpec> = strongest(run, n).iter().map(route_spec).collect();
    routes.push(RouteSpec::default());
    match add_on(run, spec, &routes, now, fx) {
        Ok(id) => tasked(run, n, i, id),
        Err(message) => refused(run, n, i, &message, now),
    }
}

/// The CI record a bisect of stage `n` served (decision 27's `BisectRecord.ci`).
fn bisected(run: &Run, n: u16, b: &BisectRecord) -> Option<usize> {
    let s = run.delivery.stage(n)?;
    s.ci.iter()
        .rposition(|r| r.phase == CiPhase::Bisecting && r.head == b.head)
}

/// How a CI bisect's culprit was handled: what its history line records (fix round 1).
pub(in crate::run::engine) enum CiCulprit {
    /// The culprit's fix task.
    Fixed(String),
    /// No culprit fix, for this reason: the red was not acted on, or a stage fix.
    NoCulprit(String),
    /// Every route refused the fix task: `fix task refused: <message>`.
    Refused(String),
}

impl CiCulprit {
    /// The bisect's history result, the culprit being `task`.
    pub(in crate::run::engine) fn result<'a>(&'a self, task: &'a str) -> BisectResult<'a> {
        match self {
            CiCulprit::Fixed(fix) => BisectResult::Culprit { task, fix },
            CiCulprit::NoCulprit(reason) => BisectResult::None(reason),
            CiCulprit::Refused(reason) => BisectResult::Refused { task, reason },
        }
    }
}

/// 9.1's bisect of a CI red blamed task `culprit` (decision 27 step 4): its fix task
/// owns the culprit's `owns` exactly, on its route one rung up, else its own route,
/// else the policy's. A culprit that is not a task gets the stage fix; a red that
/// stopped mattering gets nothing.
pub(in crate::run::engine) fn ci_culprit(
    run: &mut Run,
    n: u16,
    b: &BisectRecord,
    culprit: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> CiCulprit {
    let not_acted_on = |why: &str| CiCulprit::NoCulprit(format!("CI red not acted on: {why}"));
    let Some((i, rec)) = bisected(run, n, b).and_then(|i| Some((i, record(run, n, i)?.clone())))
    else {
        return not_acted_on("its CI record is gone");
    };
    if let Some(why) = super::ci::gone(run, n, &rec) {
        super::ci::drop_record(run, n, i, &why, now);
        return not_acted_on(&why);
    }
    let Some(at) = (run.tasks.iter()).position(|t| t.id() == culprit) else {
        let why = format!("the culprit {culprit} is not a task");
        no_culprit_at(run, n, i, &why, now, fx);
        return CiCulprit::NoCulprit(why);
    };
    let task = run.tasks[at].clone();
    let blamed = Culprit {
        id: culprit,
        title: &task.spec.title,
        brief: &task.spec.brief,
        show: b.show.as_deref(),
    };
    let command = rec.command.clone().unwrap_or_default();
    let mut spec = spec(n, &rec, task.spec.owns.clone(), task.spec.epic.clone());
    spec.brief = brief(run, n, &rec, &Repro::Reproduced(command), Some(&blamed));
    let up = escalate_for(run, at, &task.route, Mover::Worker);
    let mut routes = vec![route_spec(&up)];
    if up != task.route {
        routes.push(route_spec(&task.route));
    }
    routes.push(RouteSpec::default());
    match add_on(run, spec, &routes, now, fx) {
        Ok(id) => {
            tasked(run, n, i, id.clone());
            CiCulprit::Fixed(id)
        }
        Err(message) => {
            refused(run, n, i, &message, now);
            CiCulprit::Refused(format!("fix task refused: {message}"))
        }
    }
}

/// 9.1's bisect of a CI red ended without a culprit (`reason`): the stage fix, where
/// 9.1 would raise its "no culprit" line (decision 27 step 4).
pub(in crate::run::engine) fn ci_no_culprit(
    run: &mut Run,
    n: u16,
    b: &BisectRecord,
    reason: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if let Some(i) = bisected(run, n, b) {
        no_culprit_at(run, n, i, reason, now, fx);
    }
}

/// Record `i` of stage `n` reproduced, with no single culprit: the stage fix, unless
/// the red is no longer current or the run is ending.
pub(super) fn no_culprit_at(
    run: &mut Run,
    n: u16,
    i: usize,
    why: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(rec) = record(run, n, i).cloned() else {
        return;
    };
    if let Some(gone) = super::ci::gone(run, n, &rec) {
        return super::ci::drop_record(run, n, i, &gone, now);
    }
    log(
        run,
        now,
        format!(
            "stage {n}: CI red at {}: no culprit: {why}",
            sha7(&rec.head)
        ),
    );
    let command = rec.command.clone().unwrap_or_default();
    add_stage(run, n, i, Repro::Reproduced(command), now, fx);
}

/// The fix task was added: the record is tasked, its log text dropped (the brief
/// keeps the quote), and a planned run's orchestrator is told.
fn tasked(run: &mut Run, n: u16, i: usize, id: String) {
    let Some(r) = record_mut(run, n, i) else {
        return;
    };
    r.phase = CiPhase::Tasked;
    r.fix_task = Some(id.clone());
    r.text.clear();
    let line = format!("stage {n} CI red ({}): fix task {id} added", category(r));
    wake::note(run, line);
}

/// Hands record `i`'s red to the user with `line` (an attention line until the stage's
/// CI is green) and the wake note `wake_line`.
fn to_user(run: &mut Run, n: u16, i: usize, line: String, wake_line: String, now: u64) {
    let Some(r) = record_mut(run, n, i) else {
        return;
    };
    r.phase = CiPhase::ToUser;
    r.text.clear();
    let key = format!("{n}/ci/{}", r.key);
    run.delivery.alerts.insert(key, line.clone());
    log(run, now, line);
    wake::note(run, wake_line);
}

/// Decision 27 step 6: `made` fix tasks with this key already; over to the user.
pub(super) fn capped(run: &mut Run, n: u16, i: usize, made: usize, now: u64) {
    let Some(rec) = record(run, n, i) else {
        return;
    };
    // Fix round 1: the key (up to 50 names) is cut in the line; the record keeps it.
    let mut key: String = rec.key.chars().take(KEY_SHOWN).collect();
    if key.len() < rec.key.len() {
        key.push('…');
    }
    let line = format!("stage {n} CI still red on {key} after {made} fix tasks; over to you");
    to_user(run, n, i, line.clone(), line, now);
}

/// Decision 26: every route was refused; the red goes to the user with the rule's
/// message.
fn refused(run: &mut Run, n: u16, i: usize, message: &str, now: u64) {
    let Some(rec) = record(run, n, i) else {
        return;
    };
    let wake_line = format!("stage {n} CI red ({}): sent to you", category(rec));
    let line = format!("{wake_line}; its fix task was refused: {message}");
    to_user(run, n, i, line, wake_line, now);
}
