//! The run report, `<data_dir>/REPORT.md` (`Run::report_path`). Pure — no `std::fs`,
//! `std::process`, `std::thread`, `tokio` or `std::time::SystemTime` (design decision
//! 2): [`render`] takes the model and `now` and returns the document text; `RunService`
//! (M8a.22) is the one that writes it, through a temp file and a rename, at most once
//! per 500 ms per run (`Effect::WriteReport`).
//!
//! Most of what the brief asks the report to say — a kept branch, an aborted accept's
//! files, a halt, a rebaseline — is already a line in [`Run::log`] by the time the
//! engine reaches [`OpKind::Finished`](super::engine::OpKind) or halts: those come
//! through for free once `## Log` renders every entry. What is not already logged text
//! — the header block, the tasks table, every task's own section, the base-moved line
//! in the exact wording the brief pins, and the containment lines — is built here.

use proto::{DoneSignal, RunState, Verdict};

use super::contract::{mode_label, sha7, size_label};
use super::model::Run;
use super::report_escape::{escape_cell, list_item_text, plain_text_line};
use super::report_task::render_task;
use crate::headless::argv::CodexProjectConfig;

/// The whole document.
pub fn render(run: &Run, now: u64) -> String {
    let mut out = String::new();
    header(run, &mut out);
    out.push_str("\n## Tasks\n\n");
    tasks_table(run, &mut out);
    for task in &run.tasks {
        out.push('\n');
        render_task(task, now, &mut out);
    }
    out.push_str("\n## Log\n\n");
    log_section(run, &mut out);
    out
}

fn header(run: &Run, out: &mut String) {
    out.push_str(&format!("# anthrex run {}\n\n", run.id));
    out.push_str(&format!("Goal: {}\n\n", plain_text_line(&run.goal)));
    out.push_str(&format!("State: {}\n", state_line(run)));
    out.push_str(&format!("Approved by: {}\n", approved_by_line(run)));
    out.push_str(&format!(
        "Base branch: {} at {}\n",
        run.base_branch,
        sha7(&run.base_sha)
    ));
    out.push_str(&format!("Run branch: {}\n", run.run_branch()));
    if let Some(moved) = &run.base_moved {
        out.push_str(&format!(
            "base {} moved during the run: {}..{}, {} commits, listed at accept\n",
            run.base_branch,
            sha7(&moved.from),
            sha7(&moved.to),
            moved.commits
        ));
    }
    out.push('\n');
    profile_summary(run, out);
    out.push('\n');
    limits(run, out);
    out.push('\n');
    containment(run, out);
}

fn state_line(run: &Run) -> String {
    match run.state {
        RunState::Halted => format!(
            "halted: {}",
            run.halted_reason.as_deref().unwrap_or("unknown reason")
        ),
        other => other.label().to_string(),
    }
}

fn approved_by_line(run: &Run) -> String {
    run.approved_by
        .clone()
        .unwrap_or_else(|| "not yet approved".to_string())
}

/// The profile's `check` command, or decision 53's "this run is unverified" line — kept
/// in sync with `Run.unverified`, which the plan builder sets from the same fact
/// (`profile.check.is_none()`).
fn profile_summary(run: &Run, out: &mut String) {
    match &run.profile.check {
        Some(check) => out.push_str(&format!("Check: {check}\n")),
        None => out.push_str("no check command: this run is unverified\n"),
    }
    if run.unverified && run.profile.check.is_some() {
        // Should not happen (the two facts are set together), but the report says so
        // plainly if it ever does, rather than silently disagreeing with `Run.unverified`.
        out.push_str("this run is unverified\n");
    }
}

fn limits(run: &Run, out: &mut String) {
    let l = &run.limits;
    out.push_str(&format!(
        "Limits: {} writers, {} readers, {} bounces, {} tasks, {} windows\n",
        l.max_writers, l.max_readers, l.max_bounces, l.max_tasks, l.max_windows
    ));
    out.push_str(&format!(
        "Budgets: S {}/{}m, M {}/{}m, L {}/{}m\n",
        l.budget_s.tool_calls,
        l.budget_s.minutes,
        l.budget_m.tool_calls,
        l.budget_m.minutes,
        l.budget_l.tool_calls,
        l.budget_l.minutes
    ));
    out.push_str(&format!(
        "Stall after {}s, {} denials before block, worker permission mode {}, Codex worker sandbox {}\n",
        l.stall_after_secs, l.denials_before_block, l.worker_permission_mode, l.worker_codex_sandbox
    ));
}

/// Decision 53's project-settings line and decision 54's sandbox line (only when the
/// sandbox is off — the brief gives no "on" wording).
fn containment(run: &Run, out: &mut String) {
    if run.trusted_project.is_empty() {
        out.push_str("project settings: excluded\n");
    } else {
        out.push_str(&format!(
            "project settings trusted by --trust-project: {}\n",
            run.trusted_project.join(", ")
        ));
    }
    // Ruling T23-C1: decision 53's Codex line (the third text is this brief's own).
    let codex = run.codex_project_config.map(|branch| match branch {
        CodexProjectConfig::NotLoaded => "not loaded by this CLI",
        CodexProjectConfig::Excluded => "excluded",
        CodexProjectConfig::Loaded => "loaded by this CLI",
    });
    if let Some(codex) = codex {
        out.push_str(&format!("codex project config: {codex}\n"));
    }
    if !run.limits.worker_sandbox {
        out.push_str("worker sandbox: off ([orchestrator] worker_sandbox = false)\n");
    }
}

fn tasks_table(run: &Run, out: &mut String) {
    out.push_str(
        "| id | title | size | mode | state | rung | bounces | done signal | merge commit |\n",
    );
    out.push_str("|---|---|---|---|---|---|---|---|---|\n");
    for t in &run.tasks {
        let bounces = t.bounces;
        let bounces_sum = u32::from(bounces.done)
            + u32::from(bounces.proof)
            + u32::from(bounces.check)
            + u32::from(bounces.review)
            + u32::from(bounces.merge);
        let done_signal = t.done.as_ref().map_or("-", |d| done_signal_label(d.signal));
        let merge_commit = t
            .merge_commit
            .as_deref()
            .map_or("-".to_string(), |c| sha7(c).to_string());
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            t.spec.id,
            escape_cell(&t.spec.title),
            size_label(t.size),
            mode_label(t.test_mode),
            t.state.label(),
            t.rung,
            bounces_sum,
            done_signal,
            merge_commit
        ));
    }
}

fn log_section(run: &Run, out: &mut String) {
    for entry in &run.log {
        out.push_str(&format!(
            "- {}\n",
            list_item_text(&format!("{} ", format_utc(entry.at)), &entry.text)
        ));
    }
}

pub(super) fn done_signal_label(signal: DoneSignal) -> &'static str {
    match signal {
        DoneSignal::TaskDone => "task_done",
        DoneSignal::TurnEndFallback => "turn-end fallback",
    }
}

pub(super) fn verdict_label(verdict: Option<Verdict>) -> &'static str {
    match verdict {
        Some(Verdict::Approve) => "approve",
        Some(Verdict::Changes) => "changes",
        None => "none",
    }
}

/// `YYYY-MM-DD HH:MM:SSZ` in UTC, days-from-civil arithmetic (Howard Hinnant's
/// algorithm), no new dependency.
pub fn format_utc(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
