//! One task's section of the run report (`## <id>: <title>`). Split out of `report.rs`
//! by AGENTS.md's file-size rule. Pure, same terms as `report.rs`.

use proto::{Effort, Route, Runtime, Severity, Strength};

use super::contract::{mode_label, sha7, size_label};
use super::engine::{epoch_spend, ladder};
use super::model::Task;
use super::report::{format_utc, verdict_label};

pub(super) fn render_task(task: &Task, now: u64, out: &mut String) {
    out.push_str(&format!("## {}: {}\n\n", task.spec.id, task.spec.title));
    out.push_str(&format!(
        "State: {} (size {}, {} mode, rung {})\n",
        task.state.label(),
        size_label(task.size),
        mode_label(task.test_mode),
        task.rung
    ));
    if !task.notes.is_empty() {
        out.push_str("Notes:\n");
        for note in &task.notes {
            out.push_str(&format!("- {note}\n"));
        }
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
            p.test,
            sha7(&p.red),
            p.red_failed,
            p.head_passed,
            p.matched,
            format_utc(p.at)
        ));
    }
    for (i, c) in task.checks.iter().enumerate() {
        out.push_str(&format!(
            "Check {}: ok={} code={:?} timed_out={} {}s ({}){}\n",
            i + 1,
            c.ok,
            c.code,
            c.timed_out,
            c.secs,
            format_utc(c.at),
            if c.on_candidate { ", on candidate" } else { "" }
        ));
        out.push_str("```\n");
        out.push_str(&c.tail);
        if !c.tail.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n");
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
            out.push_str(&format!("{}\n", r.summary));
        }
        findings_by_severity(r.findings.iter(), out);
    }
    if !task.salvage_refs.is_empty() {
        out.push_str(&format!("Salvage refs: {}\n", task.salvage_refs.join(", ")));
    }
    if let Some(reason) = &task.merged_without_approval {
        out.push_str(&format!("merged without approval: {reason}\n"));
    }
    if !task.history.is_empty() {
        out.push_str("History:\n");
        for e in &task.history {
            out.push_str(&format!("- {} {}\n", format_utc(e.at), e.text));
        }
    }
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
            let where_ = match (&f.file, f.line) {
                (Some(file), Some(line)) => format!("{file}:{line}: "),
                (Some(file), None) => format!("{file}: "),
                _ => String::new(),
            };
            out.push_str(&format!("- {where_}{}\n", f.text));
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

fn strength_label(strength: Strength) -> &'static str {
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
