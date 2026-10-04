//! Milestone 9.0.7 decision 11: the Alerts view's detail — the selected alert's
//! header, its labelled rows (`phase`, `asked`/`reason`, `plan`, `merged`, `base`,
//! `run`, `worker`/`orchestrator`, `project`, `age`), its `actions` and its hint row.
//! Split from `ui/alerts_view.rs`, which lays the view out and scrolls these rows.
//! Every daemon string passes `safe_text` here. Pure.

use super::super::alerts::age_text;
use super::super::kit::{self, Hint};
use crate::app::alerts::tasks_text;
use crate::app::alerts_view::{alert_actions, alert_node, can_message, enter_label, run_of};
use crate::app::plan_review::review_tasks;
use crate::app::{Alert, AlertKey, AlertWho, App, ReviewTarget};
use crate::inspector::run_format::reason_text;
use crate::inspector::run_format::route_tag;
use crate::safe_text::{multi_line, one_line};
use crate::theme::{self, Palette, Role, fold};
use crate::tree::{self, format_elapsed};
use crate::ui::tree_view::truncate_in;
use proto::{RunInfo, TaskInfo, TaskState};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// `<n tasks>[ · <s> stages]`: stages only when there is more than one.
fn plan_text(n: usize, stages: usize) -> String {
    if stages > 1 {
        format!("{} · {stages} stages", tasks_text(n))
    } else {
        tasks_text(n)
    }
}

/// The plan's stages, as the plan review counts them: the highest planned stage or
/// the run's stage records, whichever is more.
fn run_stages(run: &RunInfo) -> usize {
    let planned = run.tasks.iter().map(|t| t.stage).max().unwrap_or(1);
    usize::from(planned).max(run.stages.len())
}

/// The distinct stages of `tasks`: a hold's own (Review focus 4: never the run's,
/// which the hold need not span), or a later round's at its gate.
fn distinct_stages<'a>(tasks: impl Iterator<Item = &'a TaskInfo>) -> usize {
    let mut stages: Vec<u16> = tasks.map(|task| task.stage).collect();
    stages.sort_unstable();
    stages.dedup();
    stages.len()
}

/// A hold's stages: the distinct stages of its own tasks.
fn hold_stages(run: &RunInfo, tasks: &[String]) -> usize {
    distinct_stages(run.tasks.iter().filter(|task| tasks.contains(&task.id)))
}

/// The gate's `plan` row: the tasks the gate reviews (`review_tasks`), and the plan's
/// stages; at a later round's gate (final fix wave C-I1) that round's tasks and their
/// own stages, as the gate alert's line counts them.
fn gate_plan(run: &RunInfo) -> String {
    let tasks = review_tasks(run, &ReviewTarget::Gate);
    let stages = if run.round > 1 {
        distinct_stages(tasks.iter().copied())
    } else {
        run_stages(run)
    };
    plan_text(tasks.len(), stages)
}

/// `<base>@<base7>`, the branch alone when the sha is unknown; none without a branch.
fn base_text(run: &RunInfo) -> Option<String> {
    let branch = one_line(&run.base_branch);
    if branch.trim().is_empty() {
        return None;
    }
    let sha: String = one_line(&run.base_sha).chars().take(7).collect();
    Some(if sha.trim().is_empty() {
        branch
    } else {
        format!("{branch}@{sha}")
    })
}

/// `<tag> <model> · <k> calls · <elapsed>` of the task's latest worker round (M9.0.5's
/// `worker_line` facts: the elapsed time frozen at the round's end).
fn worker_text(app: &App, task: &TaskInfo) -> Option<String> {
    // Whole-branch review D, I-1: a racer or test writer writes it as a worker does.
    let round = crate::inspector::writing_rounds(task)
        .max_by_key(|round| (round.started_at, round.session, round.round))?;
    let elapsed = match round.ended_at {
        Some(ended) => ended.saturating_sub(round.started_at),
        None => app.run_age(round.started_at),
    };
    let calls = match round.tool_calls {
        1 => "1 call".to_owned(),
        n => format!("{n} calls"),
    };
    // Final fix wave M5: the route as the panel's footer and the plan review name it.
    Some(format!(
        "{} · {calls} · {}",
        route_tag(&round.route),
        format_elapsed(elapsed)
    ))
}

/// `label` on the first non-blank line of `text`, the rest under it.
fn text_rows(rows: &mut Vec<(String, String)>, label: &str, text: &str) {
    let text = multi_line(text);
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.iter().position(|line| !line.trim().is_empty());
    let last = lines.iter().rposition(|line| !line.trim().is_empty());
    let (Some(first), Some(last)) = (first, last) else {
        return;
    };
    for (i, line) in lines[first..=last].iter().enumerate() {
        let label = if i == 0 { label } else { "" };
        rows.push((label.to_owned(), line.trim_end().to_owned()));
    }
}

fn run_name(run: &RunInfo, p: Palette) -> String {
    kit::run_name_in(tree::run_title(run), &run.run_id, u16::MAX, p)
}

/// Decision 11's labelled rows for `alert`, in order, each only with a value: `phase`,
/// `asked` or `reason`, `plan`, `merged`, `base`, `run`, `worker` or `orchestrator`,
/// `project`, `age`.
fn facts(app: &App, alert: &Alert) -> Vec<(String, String)> {
    let p = app.palette();
    let mut rows: Vec<(String, String)> = Vec::new();
    let push = |rows: &mut Vec<(String, String)>, label: &str, value: String| {
        rows.push((label.to_owned(), value));
    };
    let run = match &alert.key {
        AlertKey::Proposal(_) => None,
        AlertKey::Orchestrator(id)
        | AlertKey::Gate(id)
        | AlertKey::Halted(id)
        | AlertKey::Accept(id)
        | AlertKey::Hold { run: id, .. }
        | AlertKey::Blocked { run: id, .. }
        | AlertKey::Delivery { run: id, .. }
        | AlertKey::Stage { run: id, .. } => run_of(app, id),
    };
    let counted = |run: &RunInfo| {
        run.tasks
            .iter()
            .filter(|task| task.state != TaskState::Cancelled)
            .count()
    };
    match (&alert.key, run) {
        (AlertKey::Blocked { task, .. }, Some(run)) => {
            let task = run.tasks.iter().find(|t| t.id == *task);
            let block = task.and_then(|t| t.block.as_ref());
            let phase = match block {
                Some(block) => format!("blocked · {}", reason_text(block.reason)),
                None => "blocked".to_owned(),
            };
            push(&mut rows, "phase", phase);
            if let Some(block) = block {
                let label = match block.reason {
                    proto::BlockReason::Question => "asked",
                    _ => "reason",
                };
                text_rows(&mut rows, label, &block.text);
            }
            push(&mut rows, "run", run_name(run, p));
            if let Some(worker) = task.and_then(|t| worker_text(app, t)) {
                push(&mut rows, "worker", worker);
            }
        }
        (AlertKey::Gate(_), Some(run)) => {
            push(&mut rows, "phase", "awaiting approval".to_owned());
            push(&mut rows, "plan", gate_plan(run));
            if let Some(base) = base_text(run) {
                push(&mut rows, "base", base);
            }
        }
        (AlertKey::Hold { hold, .. }, Some(run)) => {
            push(&mut rows, "phase", format!("hold {hold} awaiting approval"));
            if let Some(h) = run.holds.iter().find(|h| h.id == *hold) {
                let plan = plan_text(h.tasks.len(), hold_stages(run, &h.tasks));
                push(&mut rows, "plan", plan);
            }
        }
        (AlertKey::Halted(_), Some(run)) => {
            push(&mut rows, "phase", "halted".to_owned());
            if let Some(reason) = &run.halted_reason {
                text_rows(&mut rows, "reason", reason);
            }
        }
        (AlertKey::Accept(_), Some(run)) => {
            push(&mut rows, "phase", "complete · ready to accept".to_owned());
            let merged = run
                .tasks
                .iter()
                .filter(|task| task.state == TaskState::Merged)
                .count();
            push(&mut rows, "merged", format!("{merged}/{}", counted(run)));
            if let Some(base) = base_text(run) {
                push(&mut rows, "base", base);
            }
        }
        (AlertKey::Orchestrator(_), Some(run)) => {
            let orch = run.orchestrator.as_ref();
            let window = orch
                .and_then(|orch| orch.window_id)
                .and_then(|id| app.windows.iter().find(|w| w.id == id));
            if let (Some(orch), Some(window)) = (orch, window) {
                let status = window.status.label();
                push(&mut rows, "phase", status.to_owned());
                let route = route_tag(&orch.route);
                push(&mut rows, "orchestrator", format!("{route} · {status}"));
            }
        }
        (AlertKey::Delivery { kind, stage, .. }, Some(run)) => {
            push(&mut rows, "phase", delivery_phase(*kind).to_owned());
            text_rows(&mut rows, "reason", &alert.detail);
            if let Some(n) = stage {
                push(&mut rows, "stage", format!("{n} of {}", run.stages.len()));
            }
            push(&mut rows, "run", run_name(run, p));
        }
        (AlertKey::Stage { stage, kind, .. }, Some(run)) => {
            push(&mut rows, "phase", stage_phase(*kind).to_owned());
            push(
                &mut rows,
                "stage",
                format!("{stage} of {}", run.stages.len()),
            );
            push(&mut rows, "run", run_name(run, p));
        }
        (AlertKey::Proposal(project), _) => {
            push(&mut rows, "phase", "proposal ready".to_owned());
            push(&mut rows, "project", project.to_string_lossy().into_owned());
        }
        (_, None) => {}
    }
    if let Some(age) = alert.age {
        push(&mut rows, "age", age_text(age));
    }
    rows
}

/// Milestone 9.5 decision 45: a stage alert's phase words.
fn stage_phase(kind: crate::app::StageAlert) -> &'static str {
    use crate::app::StageAlert::*;
    match kind {
        Held => "tier 3 held",
        PropagateRed => "propagate red",
        Red => "tier 3 red",
    }
}

/// Ruling R-13: a delivery alert's phase word.
fn delivery_phase(kind: proto::DeliveryAlertKind) -> &'static str {
    use proto::DeliveryAlertKind::*;
    match kind {
        CiHandedToUser => "delivery · CI handed to you",
        PrClosedUnmerged => "delivery · PR closed without merging",
        GhLoggedOut => "delivery · gh logged out",
        HostOpHeld => "delivery · held",
        ReviewRoundsOverCap => "delivery · review rounds over the cap",
    }
}

/// The header: `<t>  <title>` for a task alert, else the run's name (the project's
/// for a proposal), bold.
fn header(app: &App, alert: &Alert, width: usize) -> Line<'static> {
    let p = app.palette();
    let text = match (&alert.key, &alert.who) {
        (AlertKey::Blocked { run, task }, _) => {
            let title = run_of(app, run)
                .and_then(|r| r.tasks.iter().find(|t| t.id == *task))
                .map(|t| one_line(&t.title))
                .unwrap_or_default();
            format!("{}  {}", one_line(task), title.trim())
        }
        (_, AlertWho::Project(name)) => one_line(name),
        (_, AlertWho::Run { goal, id }) => kit::run_name_in(goal, id, u16::MAX, p),
    };
    let text = truncate_in(&fold(text.trim_end(), p.ascii), width, p.ascii);
    Line::from(Span::styled(
        text,
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

/// `actions  <label> · <label> …`, wrapped under its label by `kit::labelled_rows`
/// (fix round 1 ruling: every entry the alert allows is visible), a refused entry
/// muted (the menu gives its reason); none when the alert has no node.
fn actions_lines(app: &App, alert: &Alert, width: u16) -> Vec<Line<'static>> {
    let items = alert_actions(app, &alert.key);
    if items.is_empty() {
        return Vec::new();
    }
    let p = app.palette();
    let muted = theme::role(Role::Muted, p);
    // The joined value, folded as `labelled_rows` folds it, and each char's style.
    let separator = fold(" · ", p.ascii);
    let mut value = String::new();
    let mut styles = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            styles.extend(separator.chars().map(|_| muted));
            value.push_str(&separator);
        }
        let label = fold(&one_line(&item.label), p.ascii);
        let style = if item.refused_why.is_some() {
            muted
        } else {
            Style::default()
        };
        styles.extend(label.chars().map(|_| style));
        value.push_str(&label);
    }
    let chars: Vec<char> = value.chars().collect();
    let mut at = 0;
    let rows = [("actions".to_owned(), value.clone())];
    kit::labelled_rows(&rows, width, p)
        .into_iter()
        .map(|line| {
            let mut spans = line.spans.into_iter();
            let mut out: Vec<Span<'static>> = spans.next().into_iter().collect();
            let lead = out.len();
            // The wrapped value: each char takes its entry's style; a space the wrap
            // dropped at a break is skipped.
            for span in spans {
                for c in span.content.chars() {
                    while at < chars.len() && chars[at] != c && chars[at] == ' ' {
                        at += 1;
                    }
                    let style = styles.get(at).copied().unwrap_or_default();
                    at += 1;
                    let extend = out.len() > lead;
                    match out.last_mut() {
                        Some(last) if extend && last.style == style => {
                            last.content.to_mut().push(c);
                        }
                        _ => out.push(Span::styled(c.to_string(), style)),
                    }
                }
            }
            Line::from(out)
        })
        .collect()
}

/// The detail's own hint row (Interfaces "Hint priorities"): `⏎ <label>`, `. all
/// actions`, `m message` where offered, `o open <task|run>` (not on a proposal, whose
/// Enter opens the same screen).
fn detail_hints(app: &App, alert: &Alert) -> Vec<Hint> {
    let hint = |key: &str, word: &str, priority| Hint {
        key: key.to_owned(),
        word: word.to_owned(),
        priority,
    };
    let mut hints = vec![hint("⏎", &enter_label(app, &alert.key), 9)];
    if alert_node(&alert.key).is_some() {
        hints.push(hint(".", "all actions", 8));
    }
    if can_message(app, &alert.key) {
        hints.push(hint("m", "message", 7));
    }
    match alert.key {
        AlertKey::Blocked { .. } => hints.push(hint("o", "open task", 6)),
        AlertKey::Proposal(_) => {}
        _ => hints.push(hint("o", "open run", 6)),
    }
    hints
}

/// Every row of `alert`'s detail at `width`: the header, the labelled rows, a blank
/// row, the actions and the hints. The reducer pages over the same rows.
pub(crate) fn detail_lines(app: &App, alert: &Alert, width: u16) -> Vec<Line<'static>> {
    let w = usize::from(width);
    if w == 0 {
        return Vec::new();
    }
    let p = app.palette();
    let mut out = vec![header(app, alert, w)];
    out.extend(kit::labelled_rows(&facts(app, alert), width, p));
    out.push(Line::default());
    out.extend(actions_lines(app, alert, width));
    out.push(kit::hints(width, &detail_hints(app, alert), p));
    out
}

#[cfg(test)]
#[path = "alerts_detail_tests.rs"]
mod tests;
