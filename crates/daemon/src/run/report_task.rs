//! One task's section of the run report (`## <id>: <title>`). Split out of `report.rs`
//! by AGENTS.md's file-size rule. Pure, same terms as `report.rs`.

use proto::{
    DeciderSource, Effort, LaneInfo, LaneState, PairPhase, Route, Runtime, Severity, Strength,
};

use super::contract::{mode_label, sha7, size_label};
use super::engine::{epoch_spend, ladder, race_cost};
use super::history::writer_failures;
use super::model::{SizeCheckState, Task};
use super::report::{format_utc, verdict_label};
use super::report_escape::{
    code_span, escape_cell, escape_heading, fenced, list_item_text, plain_text_line,
};

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
    race_lines(task, out);
    pair_line(task, out);
    budget_and_spend(task, now, out);
    for (i, p) in task.proofs.iter().enumerate() {
        // Milestone 9.5 minor m3: decision 25's red check of a test writer's claim.
        let kind = if p.red_only { "Red check" } else { "Proof" };
        out.push_str(&format!(
            "{kind} {}: test={} red={} red_failed={} head_passed={} matched={} ({})\n",
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
    // Milestone 9.1 decision 59: the last tier-1 line and the signals.
    super::report_tiers::task_lines(task, out);
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

/// Milestone 9.5 decision 30: the race, then each lane's route, state, salvage ref,
/// the stale locks removed before it, a kept checkout and why it left the race.
fn race_lines(task: &Task, out: &mut String) {
    let Some(race) = super::snapshot_patterns::race_info(task) else {
        return;
    };
    let live = |l: &LaneInfo| !matches!(l.state, LaneState::Lost | LaneState::Out);
    let head = match race.winner {
        Some(w) if race.adopted => {
            format!(
                "racer {} adopted after racer {} went out",
                w.label(),
                w.other().label()
            )
        }
        Some(w) => format!("racer {} won", w.label()),
        None if race.lanes.iter().any(live) => "racing".to_string(),
        None => "no winner: both racers went out".to_string(),
    };
    // The final fix wave's m5: once there is a winner, what the other lanes spent.
    let head = match race.winner {
        Some(_) => {
            let cost = race_cost(task);
            format!(
                "{head}; race cost: {} calls, {} tokens",
                cost.tool_calls, cost.tokens
            )
        }
        None => head,
    };
    out.push_str(&format!("Race: {head}\n"));
    let stored = task.race.iter().flat_map(|r| r.lanes.iter());
    for (lane, stored) in race.lanes.iter().zip(stored) {
        let mut parts = vec![route_line(&lane.route), lane.state.label().to_string()];
        if let Some(salvage) = &lane.salvage_ref {
            parts.push(format!("salvaged {}", code_span(salvage)));
        }
        if !stored.cleared_locks.is_empty() {
            parts.push(format!("removed stale {}", stored.cleared_locks.join(", ")));
        }
        if lane.kept {
            parts.push("checkout kept".to_string());
        }
        let prefix = format!("racer {}: {}", lane.lane.label(), parts.join("; "));
        let line = match &lane.reason {
            Some(reason) => list_item_text(&format!("{prefix}; reason: "), reason),
            None => list_item_text("", &prefix),
        };
        out.push_str(&format!("- {line}\n"));
    }
    out.push('\n');
}

/// Decision 30: a paired task's test writer, its test and red commit, and its failures.
/// Its test is `pair_info`'s: before the red check, the one the spec names (review D,
/// M-1; ruling FW-3), as the TUI and the snapshot show it.
fn pair_line(task: &Task, out: &mut String) {
    let Some(pair) = super::snapshot_patterns::pair_info(task) else {
        return;
    };
    let mut parts = vec![format!("test writer {}", route_line(&pair.writer_route))];
    if let Some(test) = &pair.test {
        parts.push(format!("test {}", test.replace(['\n', '\r'], " ")));
    }
    if let Some(red) = &pair.red {
        let checked = match pair.red_checked {
            Some(true) => "fails at red",
            Some(false) => "passed at red",
            None => "not checked yet",
        };
        parts.push(format!("red {}, {checked}", sha7(red)));
    }
    parts.push(format!("writer failures {}", writer_failures(task)));
    parts.push(match pair.phase {
        PairPhase::Writing => "writing the test".to_string(),
        PairPhase::Implementing => "implementing".to_string(),
    });
    let line = plain_text_line(&format!("Pair: {}", parts.join("; ")));
    out.push_str(&format!("{line}\n"));
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
