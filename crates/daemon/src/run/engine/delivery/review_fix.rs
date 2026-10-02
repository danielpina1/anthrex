//! Decisions 26 and 31 (task M9.2.10): the engine's review fix task for one thread,
//! from Interfaces' `review` template (`run/delivery/templates.rs`), and the approval
//! hold of a review fix that owns files outside its stage (`HoldKind::Fix`), for the
//! template's tasks and for those a plan edit adds with `addresses`. A comment's text
//! is only quoted in the brief; its path, line and hunk are GitHub's metadata. Pure.

use proto::{HoldKind, HoldState, RouteSpec, Size, TaskOrigin, TestMode};

use super::super::requests::log;
use super::super::{Effect, fixes, gate_holds, wake};
use super::fix::{add_on, approved, route_spec, stage_owns, strongest};
use super::review::writes;
use crate::run::delivery::templates;
use crate::run::delivery::{PrRecord, ThreadRecord};
use crate::run::globs::{OwnsMatcher, escape_path, validate_glob};
use crate::run::model::{FixOf, Run};

/// Decision 22: a comment's path, when it is a literal path inside the repository
/// (M8a's `validate_glob` once escaped, on one line); else the comment is PR-level.
fn literal_path(t: &ThreadRecord) -> Option<String> {
    let path = t.path.as_deref()?;
    let one_line = proto::safe_text::one_line(path) == path;
    let ok = one_line && path.chars().count() <= 1_000 && !path.ends_with('/');
    (ok && validate_glob(&escape_path(path)).is_ok()).then(|| path.to_string())
}

/// Decision 31: a commented file's fix owns what the stage's approved tasks that own
/// it own; a file none of them owns is owned exactly (and held); a PR-level comment's
/// fix owns the union of the stage's approved `owns`.
fn review_owns(run: &Run, n: u16, path: Option<&str>) -> Vec<String> {
    let Some(path) = path else {
        return stage_owns(run, n);
    };
    let mut owns: Vec<String> = Vec::new();
    for task in approved(run, n) {
        let owner = OwnsMatcher::new(&task.spec.owns).is_ok_and(|m| m.matches(path));
        for glob in task.spec.owns.iter().filter(|_| owner) {
            if !owns.contains(glob) {
                owns.push(glob.clone());
            }
        }
    }
    if owns.is_empty() {
        owns.push(escape_path(path));
    }
    owns
}

/// The comments a fix brief quotes: those of the thread's authors that may write
/// (decision 29: nothing else reaches an agent), each with its author.
fn quoted(run: &Run, t: &ThreadRecord) -> Vec<(String, String)> {
    if t.comments.is_empty() {
        return vec![(t.author.clone(), t.text.clone())];
    }
    let writer = |login: &str| login == t.author || writes(run, login) == Some(true);
    (t.comments.iter())
        .filter(|c| !c.text.is_empty() && writer(&c.author))
        .map(|c| (c.author.clone(), c.text.clone()))
        .collect()
}

/// Adds the template's fix task for thread `t` (decision 26): stage `n`, the stage's
/// strongest route, then the policy's. Comment text is only quoted in the brief.
pub(super) fn add_review_fix(
    run: &mut Run,
    n: u16,
    pr: &PrRecord,
    t: &ThreadRecord,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let path = literal_path(t);
    let comments = quoted(run, t);
    let brief = templates::review_brief(n, &pr.url, path.as_deref(), t, &comments);
    let spec = fixes::FixSpec {
        origin: TaskOrigin::Review,
        fixes: FixOf::Review {
            stage: n,
            pr: pr.number,
            threads: vec![format!("{}:{}", pr.number, t.key)],
        },
        stage: n,
        title: templates::review_title(n, path.as_deref()),
        brief,
        acceptance: templates::review_acceptance(),
        owns: review_owns(run, n, path.as_deref()),
        size: Size::S,
        epic: None,
        route: RouteSpec::default(),
        test_mode: TestMode::Check,
        test_mode_reason: Some(templates::REVIEW_FIX_TEST_MODE_REASON.to_string()),
        sync: None,
    };
    let mut routes: Vec<RouteSpec> = strongest(run, n).iter().map(route_spec).collect();
    routes.push(RouteSpec::default());
    add_on(run, spec, &routes, now, fx)
}

/// Decision 26: a literal `owns` entry is covered when an approved task of the stage
/// (other than task `id`) owns a glob matching it; a glob only when one owns it exactly.
fn covered(run: &Run, n: u16, id: &str, entry: &str) -> bool {
    let others = approved(run, n).into_iter().filter(|t| t.id() != id);
    if entry.contains(['*', '?', '[']) {
        return others
            .into_iter()
            .any(|t| t.spec.owns.iter().any(|g| g == entry));
    }
    others
        .into_iter()
        .any(|t| OwnsMatcher::new(&t.spec.owns).is_ok_and(|m| m.matches(entry)))
}

/// Decision 26: review fix task `id` of stage `n` owns files outside the stage, so it
/// waits in hold `hold-<id>` (`HoldKind::Fix`, awaiting the user) with the attention
/// line and wake note. `Some(<hold id>)` when it was held.
pub(super) fn hold_outside(run: &mut Run, n: u16, id: &str, now: u64) -> Option<String> {
    let task = run.task(id)?;
    let outside: Vec<String> = (task.spec.owns.iter())
        .filter(|e| !covered(run, n, id, e))
        .cloned()
        .collect();
    let hold = format!("hold-{id}");
    if outside.is_empty() || run.orch.gate_holds.iter().any(|h| h.id == hold) {
        return None;
    }
    let kind = HoldKind::Fix {
        stage: n,
        paths: outside,
    };
    gate_holds::create(run, &hold, kind, now);
    if let Some(h) = run.orch.gate_holds.iter_mut().find(|h| h.id == hold) {
        h.state = HoldState::Awaiting;
        h.tasks.push(id.to_string());
    }
    if let Some(t) = run.tasks.iter_mut().find(|t| t.id() == id) {
        t.orch.gate_hold = Some(hold.clone());
    }
    let line = hold_line(run, &hold).unwrap_or_default();
    log(run, now, line.clone());
    wake::note(run, line);
    Some(hold)
}

/// Decision 26's attention line and wake note of fix hold `hold`.
pub(super) fn hold_line(run: &Run, hold: &str) -> Option<String> {
    let h = run.orch.gate_holds.iter().find(|h| h.id == hold)?;
    let HoldKind::Fix { stage, paths } = &h.kind else {
        return None;
    };
    let id = hold.strip_prefix("hold-").unwrap_or(hold);
    Some(format!(
        "fix task {id} for stage {stage} needs approval: it owns {}, outside the stage; anthrex run approve {} --hold {hold}",
        paths.join(", "),
        run.id
    ))
}

/// Decision 31: the review fix tasks a plan edit added (`addresses`) wait for the
/// user when they own files outside their stage, as the template's do; the first hold.
pub(in crate::run::engine) fn holds_for(
    run: &mut Run,
    added: &[String],
    now: u64,
) -> Option<String> {
    let mut first = None;
    for id in added {
        let Some(task) = run.task(id) else {
            continue;
        };
        if task.origin != TaskOrigin::Review || task.orch.gate_hold.is_some() {
            continue;
        }
        let n = task.spec.stage;
        if let Some(hold) = hold_outside(run, n, id, now) {
            first.get_or_insert(hold);
        }
    }
    first
}

/// The final fix wave's I-4: a fix hold whose every task ended cancelled (or is gone)
/// before anyone decided it resolves as `Moot`: there is nothing left to approve, its
/// attention line clears, and completion, which waits only on `Drafting` and
/// `Awaiting` holds, no longer waits on it.
pub(super) fn moot(run: &mut Run, now: u64) {
    let cancelled = |run: &Run, id: &str| {
        run.task(id)
            .is_none_or(|t| t.state == proto::TaskState::Cancelled)
    };
    let ids: Vec<String> = (run.orch.gate_holds.iter())
        .filter(|h| h.state == HoldState::Awaiting && matches!(h.kind, HoldKind::Fix { .. }))
        .filter(|h| h.tasks.iter().all(|t| cancelled(run, t)))
        .map(|h| h.id.clone())
        .collect();
    for id in ids {
        let Some(h) = run.orch.gate_holds.iter_mut().find(|h| h.id == id) else {
            continue;
        };
        h.state = HoldState::Moot;
        h.decided_at = Some(now);
        h.decided_by = Some("anthrex".into());
        let tasks = h.tasks.join(", ");
        let line = format!("hold {id} is moot: its fix task {tasks} was cancelled");
        log(run, now, line.clone());
        wake::note(run, line);
    }
}
