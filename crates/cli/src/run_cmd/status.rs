//! `anthrex run status`'s text (the brief's CLI section) and `run accept`'s moved-base
//! listing (decision 20). Pure: every function takes the snapshot and returns text.

use daemon::run::triage::{kinds_scale, source_label};
use proto::{BaseMovedInfo, GateCounts, Route, RunInfo, RunState, Size, TaskInfo, TestMode};

/// Decision 20: the commits a moved-base prompt lists, at most.
pub const LISTED_COMMITS: usize = 50;

/// One block per run, newest first. `utc_offset` is the local zone's, in seconds: the
/// caller reads it, so this stays pure.
pub fn render(runs: &[RunInfo], utc_offset: i64) -> String {
    let mut sorted: Vec<&RunInfo> = runs.iter().collect();
    sorted.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.run_id.cmp(&b.run_id))
    });
    sorted
        .into_iter()
        .map(|run| run_block(run, utc_offset))
        .collect()
}

/// One run's block: the header, a halted run's reason, the goal, the report path, the
/// task table, when promotion was requested (local time) and the attention lines.
pub fn run_block(run: &RunInfo, utc_offset: i64) -> String {
    let merged = run
        .tasks
        .iter()
        .filter(|t| t.state == proto::TaskState::Merged)
        .count();
    let state = match (run.state, run.paused_from) {
        (RunState::Paused, Some(from)) => format!("paused (from {})", from.label()),
        (state, _) => state.label().to_string(),
    };
    // M8b decision 24: ` fast path` after the state.
    let fast = run.path == Some(proto::RunPath::Fast);
    let state = if fast {
        format!("{state} fast path")
    } else {
        state
    };
    let mut out = format!(
        "{}  {}  {}/{} merged  base {}@{}  writers {}/{}  readers {}/{}  rev {}\n",
        run.run_id,
        state,
        merged,
        run.tasks.len(),
        run.base_branch,
        short(&run.base_sha),
        run.writers_busy,
        run.max_writers,
        run.readers_busy,
        run.max_readers,
        run.revision
    );
    if run.state == RunState::Halted {
        let reason = run.halted_reason.as_deref().unwrap_or("no reason recorded");
        out.push_str(&format!("  halted: {reason}\n"));
    }
    out.push_str(&format!("  goal: {}\n", run.goal));
    if let Some(t) = &run.triage {
        let (triage, source) = (kinds_scale(&t.kinds, t.scale), source_label(t.source));
        // Whole-branch review I1: the fast-path task's test mode, which triage chose.
        let mode = match run.tasks.first().filter(|_| fast) {
            Some(task) => format!(
                "; {} test mode {}{}",
                task.id,
                mode_label(task.test_mode),
                task.test_mode_reason
                    .as_deref()
                    .map_or(String::new(), |why| format!(" ({})", one_line(why)))
            ),
            None => String::new(),
        };
        out.push_str(&format!("  triage: {triage} ({source}){mode}\n"));
    }
    out.push_str(&format!("  report: {}\n", run.report_path.display()));
    if run.unconfined_checks {
        out.push_str(
            "  checks: unconfined (this platform cannot confine checks, proofs and setup)\n",
        );
    }
    out.push_str(&row(
        "ID", "SIZE", "MODE", "STATE", "RUNG", "BOUNCES", "ROUTE", "WINDOWS",
    ));
    for task in &run.tasks {
        out.push_str(&task_row(task));
    }
    // Review I1 (M8c.1): the snapshot's promotion line carries no time; this is it, local.
    if let Some(at) = run.promote_requested_at {
        let hhmm = local_hhmm(at, utc_offset);
        out.push_str(&format!("  promotion: requested at {hhmm}\n"));
    }
    for line in &run.attention {
        out.push_str(&format!("  attention: {line}\n"));
    }
    out
}

/// `at` (unix seconds) as local `hh:mm`, given the zone's offset from UTC.
fn local_hhmm(at: u64, utc_offset: i64) -> String {
    let secs = (at as i64 + utc_offset).rem_euclid(86_400);
    format!("{:02}:{:02}", secs / 3600, (secs / 60) % 60)
}

#[allow(clippy::too_many_arguments)]
fn row(
    id: &str,
    size: &str,
    mode: &str,
    state: &str,
    rung: &str,
    bounces: &str,
    route: &str,
    windows: &str,
) -> String {
    let line = format!(
        "  {}{}{}{}{}{}{}{windows}",
        cell(id, 5),
        cell(size, 5),
        cell(mode, 7),
        cell(state, 15),
        cell(rung, 5),
        cell(bounces, 15),
        cell(route, 31)
    );
    format!("{}\n", line.trim_end())
}

/// `text` padded to `width` characters; a cell as wide as its column or wider ends in
/// one space instead, so it never runs into the next column (ruling T23-minors, M4).
fn cell(text: &str, width: usize) -> String {
    if text.chars().count() < width {
        format!("{text:<width$}")
    } else {
        format!("{text} ")
    }
}

fn task_row(task: &TaskInfo) -> String {
    let size = format!(
        "{}{}",
        size_label(task.size),
        if task.hub { "◆" } else { "" }
    );
    row(
        &task.id,
        &size,
        mode_label(task.test_mode),
        task.state.label(),
        &task.rung.to_string(),
        &bounces_text(&task.bounces),
        &route_text(&task.route),
        &windows_text(task),
    )
}

fn size_label(size: Size) -> &'static str {
    match size {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

fn mode_label(mode: TestMode) -> &'static str {
    match mode {
        TestMode::Tdd => "tdd",
        TestMode::Check => "check",
        TestMode::None => "none",
    }
}

/// `-`, or each gate that bounced the task as `<gate> <n>`, joined with `, `.
pub fn bounces_text(bounces: &GateCounts) -> String {
    let gates = [
        ("done", bounces.done),
        ("proof", bounces.proof),
        ("check", bounces.check),
        ("review", bounces.review),
        ("merge", bounces.merge),
    ];
    let parts: Vec<String> = gates
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(gate, n)| format!("{gate} {n}"))
        .collect();
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(", ")
    }
}

/// `<runtime> <model or (default)> <effort>`.
fn route_text(route: &Route) -> String {
    let model = if route.model.is_empty() {
        "(default)"
    } else {
        route.model.as_str()
    };
    let effort = match route.effort {
        proto::Effort::Low => "low",
        proto::Effort::Medium => "medium",
        proto::Effort::High => "high",
    };
    format!("{} {model} {effort}", route.runtime.label())
}

/// Every window the task's rounds ran in, live and past, in order, once each.
fn windows_text(task: &TaskInfo) -> String {
    let mut ids: Vec<u32> = Vec::new();
    for id in task.rounds.iter().filter_map(|r| r.window_id) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")
}

/// The first seven characters of a sha.
pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// Decision 20's listing, before `run accept` asks to merge onto a moved base: the
/// heading, at most [`LISTED_COMMITS`] commits indented two spaces, then how many more.
pub fn base_moved_listing(base: &str, info: &BaseMovedInfo) -> String {
    let mut out = format!(
        "{base} moved since the run started ({}..{}, {} commit{}):\n",
        short(&info.from),
        short(&info.to),
        info.total,
        if info.total == 1 { "" } else { "s" }
    );
    let listed = info.commits.len().min(LISTED_COMMITS);
    for commit in &info.commits[..listed] {
        out.push_str(&format!("  {commit}\n"));
    }
    let more = (info.total as usize).max(info.commits.len()) - listed;
    if more > 0 {
        out.push_str(&format!("  … and {more} more\n"));
    }
    out
}

/// Decision 20's question once the moved base is listed.
pub fn base_moved_question(base: &str, info: &BaseMovedInfo) -> String {
    format!(
        "merge onto {base} at {} including these commits? [y/N] ",
        short(&info.to)
    )
}

#[cfg(test)]
#[path = "status_tests.rs"]
pub(super) mod tests;

/// Whole-branch re-review N4: a model-written reason on one status line, with every
/// control character (a newline, an escape) shown as a space.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
