//! `anthrex run status`'s text (the brief's CLI section) and `run accept`'s moved-base
//! listing (decision 20). Pure: every function takes the snapshot and returns text.

use daemon::run::triage::{kinds_scale, source_label};
use proto::{
    BaseMovedInfo, GateCounts, PairPhase, Route, RunInfo, RunState, Size, TaskInfo, TestMode,
};

use super::status_orch::{orchestrator_lines, state_text};

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
/// orchestrator's lines (milestone 9), the task table, when promotion was requested
/// (local time) and the attention lines.
pub fn run_block(run: &RunInfo, utc_offset: i64) -> String {
    let merged = run
        .tasks
        .iter()
        .filter(|t| t.state == proto::TaskState::Merged)
        .count();
    let state = match (run.state, run.paused_from) {
        (RunState::Paused, Some(from)) => format!("paused (from {})", from.label()),
        // Milestone 9.2 decision 36.
        _ if super::delivery::delivering(run) => "running (delivering)".to_string(),
        (state, _) => state.label().to_string(),
    };
    // M8b decision 24: ` fast path` after the state.
    let fast = run.path == Some(proto::RunPath::Fast);
    let state = if fast {
        format!("{state} fast path")
    } else {
        state
    };
    // Milestone 9.5 decision 30: ` (codex cap 1)` per capped runtime.
    let cap = |(runtime, cap): (&String, &u8)| format!(" ({} cap {cap})", one_line(runtime));
    let caps: String = run.writer_caps.iter().map(cap).collect();
    let mut out = format!(
        "{}  {}  {}/{} merged  base {}@{}  writers {}/{}{caps}  readers {}/{}  rev {}\n",
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
    out.push_str(&super::rounds::status_lines(run));
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
    out.push_str(&orchestrator_lines(run));
    out.push_str(&super::design::status_lines(run));
    out.push_str(&super::delivery::status_lines(run));
    // Milestone 9.1 (Interfaces "CLI"): a `Multi` run's stages, before its tasks.
    let multi = run.stages.len() > 1;
    if multi {
        for stage in &run.stages {
            out.push_str(&stage_line(stage, run.stages.len()));
        }
    }
    out.push_str(&row(
        "ID", "SIZE", "MODE", "STATE", "RUNG", "BOUNCES", "ROUTE", "WINDOWS",
    ));
    for task in &run.tasks {
        let line = task_row(run, task);
        let mut line = line.trim_end_matches('\n').to_string();
        if multi {
            line.push_str(&format!(" [stage {}]", task.stage));
        }
        if let Some(fixes) = &task.fixes {
            line.push_str(&format!(" (fix: {})", one_line(fixes)));
        }
        out.push_str(&line);
        out.push('\n');
        out.push_str(&pattern_lines(run, task));
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

/// Decision 30: under a racing task each lane's runtime and state; under a paired task
/// the test writer's progress and the implementer's state.
fn pattern_lines(run: &RunInfo, task: &TaskInfo) -> String {
    let mut out = String::new();
    if let Some(race) = &task.race {
        let lane = |l: &proto::LaneInfo| {
            let (lane, runtime) = (l.lane.label(), l.route.runtime.label());
            format!("{lane} {runtime} {}", l.state.label())
        };
        let lanes: Vec<String> = race.lanes.iter().map(lane).collect();
        out.push_str(&format!("    race: {}\n", lanes.join(" · ")));
    }
    if let Some(pair) = &task.pair {
        let (writer, implementer) = match pair.phase {
            PairPhase::Writing => ("working".to_string(), "not started".to_string()),
            PairPhase::Implementing => {
                let red =
                    (pair.red.as_deref()).map_or(String::new(), |r| format!(", red {}", short(r)));
                (format!("done{red}"), state_text(run, task))
            }
        };
        out.push_str(&format!(
            "    pair: test writer {writer}; implementer {implementer}\n"
        ));
    }
    out
}

/// `stage <n>/<of>  <branch>  <merged>/<tasks> merged  tier 3 <state>`, with the last
/// job's duration and the stage's fix tasks in parentheses; `not created` for a stage
/// with no branch yet.
fn stage_line(stage: &proto::StageInfo, of: usize) -> String {
    let n = stage.n;
    if stage.head.is_none() {
        return format!("stage {n}/{of}  not created\n");
    }
    let full = &stage.full;
    let state = match full.state {
        proto::FullState::None => "none",
        proto::FullState::Running => "running",
        proto::FullState::Green => "green",
        proto::FullState::Red => "red",
        proto::FullState::Bisecting => "bisecting",
    };
    let mut parts: Vec<String> = Vec::new();
    if matches!(full.state, proto::FullState::Green | proto::FullState::Red)
        && let Some(secs) = full.secs
    {
        parts.push(duration(secs));
    }
    parts.extend(stage.fix_tasks.iter().cloned());
    let detail = if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    };
    format!(
        "stage {n}/{of}  {}  {}/{} merged  tier 3 {state}{detail}\n",
        one_line(&stage.branch),
        stage.merged,
        stage.tasks
    )
}

/// `42s`, `41m 12s` or `3h 2m`.
fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
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

fn task_row(run: &RunInfo, task: &TaskInfo) -> String {
    let size = format!(
        "{}{}",
        size_label(task.size),
        if task.hub { "◆" } else { "" }
    );
    row(
        &task.id,
        &size,
        mode_label(task.test_mode),
        &state_text(run, task),
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
    let effort = route.effort.to_string();
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

/// M9.14 review fixes, item 3: a daemon's reply or refusal, which may echo what the user
/// typed, as the terminal is given it. Milestone 9.2 (task M9.2.14 fix round 1, m1):
/// `proto::safe_text::multi_line`'s rules, the client's: lines kept (`\r\n` and a lone
/// `\r` become `\n`), every other control character and U+2028/U+2029 a space, and
/// every hidden format character (bidi controls, zero-width joiners) dropped.
pub fn printable(text: &str) -> String {
    proto::safe_text::multi_line(text)
}

#[cfg(test)]
#[path = "status_tests_stages.rs"]
mod tests_stages;

#[cfg(test)]
#[path = "status_tests_patterns.rs"]
mod tests_patterns;
