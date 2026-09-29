//! One task's section of the run report (`## <id>: <title>`). Split out of `report.rs`
//! by AGENTS.md's file-size rule. Pure, same terms as `report.rs`.

use proto::{DeciderSource, Effort, Route, Runtime, Severity, Strength};

use super::contract::{mode_label, sha7, size_label};
use super::engine::{epoch_spend, ladder};
use super::model::{SizeCheckState, Task};
use super::report::{format_utc, verdict_label};
use super::report_escape::{escape_cell, escape_heading, fenced, list_item_text, plain_text_line};

pub(super) fn render_task(task: &Task, now: u64, out: &mut String) {
    out.push_str(&format!(
        "## {}: {}\n\n",
        task.spec.id,
        escape_heading(&task.spec.title)
    ));
    out.push_str(&format!(
        "State: {} (size {}, {} mode, rung {})\n",
        task.state.label(),
        size_label(task.size),
        mode_label(task.test_mode),
        task.rung
    ));
    // M8b decision 19: a size cross-check that disagreed with the engine.
    if let Some(SizeCheckState::Done(info)) = &task.size_check
        && let (false, Some(decided)) = (info.agreed, info.decided)
    {
        out.push_str(&format!(
            "Size cross-check: {} by the engine, {} by the decider: {}\n",
            size_label(info.engine),
            size_label(decided),
            plain_text_line(&info.reason)
        ));
    }
    if !task.notes.is_empty() || !task.orch.worker_notes.is_empty() {
        out.push_str("Notes:\n");
        for note in &task.notes {
            out.push_str(&format!("- {}\n", list_item_text("", note)));
        }
        // Milestone 9 decision 42i: the worker's own `task_note`s follow.
        for note in &task.orch.worker_notes {
            let kind = crate::run::orch::json::label(&note.kind);
            let prefix = format!("{} ({kind}, from the worker) ", format_utc(note.at));
            out.push_str(&format!("- {}\n", list_item_text(&prefix, &note.text)));
        }
        // A blank line closes the list: without it, CommonMark's lazy continuation
        // would fold the next plain line (`Route:`) into the last note's own
        // paragraph instead of starting a new one.
        out.push('\n');
    }
    out.push_str(&format!("Route: {}\n", route_line(&task.route)));
    out.push_str(&format!(
        "Review route: {}\n",
        task.review_route
            .as_ref()
            .map_or("none".to_string(), route_line)
    ));
    budget_and_spend(task, now, out);
    for (i, p) in task.proofs.iter().enumerate() {
        out.push_str(&format!(
            "Proof {}: test={} red={} red_failed={} head_passed={} matched={} ({})\n",
            i + 1,
            escape_cell(&p.test),
            sha7(&p.red),
            p.red_failed,
            p.head_passed,
            p.matched,
            format_utc(p.at)
        ));
    }
    for (i, c) in task.checks.iter().enumerate() {
        // M8b decision 18: the summary's source, and the decider's own lines.
        let source = match c.summary_source {
            Some(DeciderSource::Decider) => ", summary by the decider",
            Some(DeciderSource::Fallback) => ", summary by its fallback",
            None => "",
        };
        out.push_str(&format!(
            "Check {}: ok={} code={:?} timed_out={} {}s ({}){}{source}\n",
            i + 1,
            c.ok,
            c.code,
            c.timed_out,
            c.secs,
            format_utc(c.at),
            if c.on_candidate { ", on candidate" } else { "" }
        ));
        out.push_str(&fenced(&c.tail));
        if let (Some(summary), Some(DeciderSource::Decider)) = (&c.summary, c.summary_source) {
            out.push_str("Summary:\n");
            out.push_str(&fenced(summary));
        }
    }
    for r in &task.reviews {
        out.push_str(&format!(
            "Review round {} ({}): verdict={} {}..{}\n",
            r.round,
            route_line(&r.route),
            verdict_label(r.verdict),
            sha7(&r.base),
            sha7(&r.head)
        ));
        if !r.summary.is_empty() {
            out.push_str(&format!("{}\n", plain_text_line(&r.summary)));
        }
        findings_by_severity(r.findings.iter(), out);
        // Closes the findings list, the same way the notes list is closed above.
        out.push('\n');
    }
    if !task.salvage_refs.is_empty() {
        out.push_str(&format!("Salvage refs: {}\n", task.salvage_refs.join(", ")));
    }
    if let Some(reason) = &task.merged_without_approval {
        out.push_str(&format!(
            "merged without approval: {}\n",
            plain_text_line(reason)
        ));
    }
    messages(task, out);
    if !task.history.is_empty() {
        out.push_str("History:\n");
        for e in &task.history {
            out.push_str(&format!(
                "- {}\n",
                list_item_text(&format!("{} ", format_utc(e.at)), &e.text)
            ));
        }
        out.push('\n');
    }
}

/// Milestone 9 decision 42i: every message the task was sent, with its source.
fn messages(task: &Task, out: &mut String) {
    if task.orch.messages.is_empty() {
        return;
    }
    out.push_str("Messages:\n");
    for m in &task.orch.messages {
        let kind = crate::run::orch::json::label(&m.kind);
        let waiting = if m.delivered {
            ""
        } else {
            ", not delivered yet"
        };
        let prefix = format!(
            "{} ({kind}, from {}{waiting}) ",
            format_utc(m.at),
            m.source.label()
        );
        out.push_str(&format!("- {}\n", list_item_text(&prefix, &m.text)));
    }
    out.push('\n');
}

fn budget_and_spend(task: &Task, now: u64, out: &mut String) {
    let budget = task.budget;
    out.push_str(&format!(
        "Budget: {} tool calls, {}m{}\n",
        budget.tool_calls,
        budget.minutes,
        budget
            .tokens
            .map_or(String::new(), |t| format!(", {t} tokens"))
    ));
    let total = ladder::total_spend(task, now);
    out.push_str(&format!(
        "Spent: {} tool calls, {}m, {} tokens (billable)\n",
        total.tool_calls,
        total.secs / 60,
        total.tokens
    ));
    if task.epoch.is_some() {
        let since_retry = epoch_spend(task, now);
        if since_retry.tool_calls != total.tool_calls || since_retry.tokens != total.tokens {
            out.push_str(&format!(
                "Spent since last retry: {} tool calls, {}m, {} tokens (billable)\n",
                since_retry.tool_calls,
                since_retry.secs / 60,
                since_retry.tokens
            ));
        }
    }
    for round in &task.rounds {
        out.push_str(&format!(
            "- {:?} session {} round {}: {} turns, {} tool calls, {} tokens (billable), {} denials\n",
            round.role,
            round.session,
            round.round,
            round.turns,
            round.tool_calls,
            round.usage.billable(),
            round.denials
        ));
    }
    if !task.rounds.is_empty() {
        out.push('\n');
    }
}

fn findings_by_severity<'a>(findings: impl Iterator<Item = &'a proto::Finding>, out: &mut String) {
    let all: Vec<&proto::Finding> = findings.collect();
    for (label, sev) in [
        ("Critical", Severity::Critical),
        ("Important", Severity::Important),
        ("Minor", Severity::Minor),
    ] {
        let group: Vec<&&proto::Finding> = all.iter().filter(|f| f.severity == sev).collect();
        if group.is_empty() {
            continue;
        }
        out.push_str(&format!("{label}:\n"));
        for f in group {
            // Final review A-8: the failing input is a locator too (decision 35).
            let file = match (&f.file, f.line) {
                (Some(file), Some(line)) => Some(format!("{}:{line}", escape_cell(file))),
                (Some(file), None) => Some(escape_cell(file)),
                _ => None,
            };
            let input = f
                .input
                .as_ref()
                .map(|i| format!("input {}", escape_cell(i)));
            let parts: Vec<String> = file.into_iter().chain(input).collect();
            let where_ = if parts.is_empty() {
                String::new()
            } else {
                format!("{}: ", parts.join(", "))
            };
            out.push_str(&format!("- {}\n", list_item_text(&where_, &f.text)));
        }
    }
}

fn route_line(route: &Route) -> String {
    format!(
        "{} {} ({}/{})",
        runtime_label(route.runtime),
        route.model,
        strength_label(route.strength),
        effort_label(route.effort)
    )
}

fn runtime_label(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Claude => "claude",
        Runtime::Codex => "codex",
        Runtime::Shell => "shell",
    }
}

pub(super) fn strength_label(strength: Strength) -> &'static str {
    match strength {
        Strength::Fast => "fast",
        Strength::Standard => "standard",
        Strength::Frontier => "frontier",
    }
}

fn effort_label(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}
