//! Milestone 9.1 decision 59: the report's "Testing" section (per stage, its tier 3, a
//! red propagate (decision 52) and every bisect; the fix tasks; the unknown-graph note)
//! and each task's last tier-1 line and test-weakening signals. Pure. A run that is untiered, has one stage and no
//! fix task gets none of it, so its report stays milestone 9's.

use proto::{FullState, TaskOrigin};

use super::contract::sha7;
use super::model::{Run, Task};
use super::report_escape::{list_item_text, plain_text_line};

/// Failing test names a tier-3 line shows.
const FAILING_SHOWN: usize = 5;

/// Whether the run has anything for the "Testing" section.
fn has_testing(run: &Run) -> bool {
    run.profile.tiers.is_tiered()
        || run.stage_layout == super::model::StageLayout::Multi
        || run.graph_note.is_some()
        || run.tasks.iter().any(|t| t.origin != TaskOrigin::Plan)
}

fn plural(n: u32, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn names(list: &[String], k: usize) -> String {
    let shown: Vec<&str> = list.iter().take(k).map(String::as_str).collect();
    plain_text_line(&shown.join(", "))
}

/// The `## Testing` section, or nothing.
pub(super) fn section(run: &Run, out: &mut String) {
    if !has_testing(run) {
        return;
    }
    out.push_str("\n## Testing\n\n");
    if let Some(note) = &run.graph_note {
        out.push_str(&format!("{}\n\n", plain_text_line(note)));
    }
    // Ruling C-27 (M-2), the "Risks" section's cache staleness.
    if super::engine::cache_enabled(run) {
        out.push_str("Cached results assume tests read only the checkout.\n\n");
    }
    let stages = super::snapshot_stages::stage_infos(run);
    for (i, s) in stages.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("### Stage {}\n\n", s.n));
        match &s.head {
            Some(head) => out.push_str(&format!("Branch: {} at {}\n", s.branch, sha7(head))),
            None => out.push_str(&format!("Branch: {} (not created)\n", s.branch)),
        }
        out.push_str(&tier3_line(&s.full));
        if let Some(head) = &s.propagate_red {
            out.push_str(&format!(
                "Propagate of stage {} at {}: red\n",
                s.n.saturating_sub(1),
                sha7(head)
            ));
        }
        for b in run
            .stage(s.n)
            .map(|r| r.full.ended.as_slice())
            .unwrap_or_default()
        {
            let result = match (&b.culprit, &b.fix_task, &b.reason) {
                (Some(c), Some(fix), _) => format!("culprit {c}, fix task {fix}"),
                (Some(c), None, Some(reason)) => {
                    format!("culprit {c}, {}", plain_text_line(reason))
                }
                (_, _, reason) => format!(
                    "no culprit: {}",
                    plain_text_line(reason.as_deref().unwrap_or_default())
                ),
            };
            out.push_str(&format!(
                "Bisect of {}: {}, {}, {result}\n",
                sha7(&b.head),
                plural(b.range, "merge"),
                plural(b.probes, "probe")
            ));
        }
    }
    let fixes: Vec<&Task> = run
        .tasks
        .iter()
        .filter(|t| t.origin != TaskOrigin::Plan)
        .collect();
    if !fixes.is_empty() {
        out.push_str("\n### Fix tasks\n\n");
        for t in fixes {
            let what = t.fixes.as_ref().map(|f| super::engine::fix_text(run, f));
            let prefix = format!("{} ({}): ", t.id(), what.unwrap_or_default());
            out.push_str(&format!("- {}\n", list_item_text(&prefix, t.state.label())));
        }
    }
}

/// `Tier 3: <state>` and, once a job ran, where, how long, how many shards, and its
/// flaky and first failing tests.
fn tier3_line(full: &proto::FullInfo) -> String {
    let state = match full.state {
        FullState::None => "none",
        FullState::Running => "running",
        FullState::Green => "green",
        FullState::Red => "red",
        FullState::Bisecting => "bisecting",
    };
    let mut line = format!("Tier 3: {state}");
    if let (Some(commit), Some(secs)) = (&full.commit, full.secs) {
        line.push_str(&format!(
            " at {}, {secs}s, {}",
            sha7(commit),
            plural(u32::from(full.shards), "shard")
        ));
        if !full.flaky.is_empty() {
            line.push_str(&format!(
                ", flaky: {}",
                names(&full.flaky, full.flaky.len())
            ));
        }
        if !full.failing.is_empty() {
            line.push_str(&format!(
                ", failing: {}",
                names(&full.failing, FAILING_SHOWN)
            ));
        }
    }
    line.push('\n');
    line
}

/// A task's last tier-1 record and its signals with their answers, after its checks.
pub(super) fn task_lines(task: &Task, out: &mut String) {
    let last = task
        .checks
        .iter()
        .rev()
        .find_map(|c| c.tier.as_ref().filter(|t| t.tier == 1));
    if let Some(t) = last {
        let verdict = if t.ok { "green" } else { "red" };
        let mut line = format!(
            "Tier 1: {}, {} steps, {} cached, {}s, {verdict}",
            plain_text_line(&t.affected),
            t.steps,
            t.cached,
            t.secs
        );
        if !t.flaky.is_empty() {
            line.push_str(&format!(", flaky: {}", names(&t.flaky, t.flaky.len())));
        }
        out.push_str(&line);
        out.push('\n');
    }
    let signals = super::engine::weakening::signal_infos(task);
    if signals.is_empty() {
        return;
    }
    out.push_str("Test changes:\n");
    for s in signals {
        let answer = s.answered.unwrap_or_else(|| "not answered".to_string());
        let item = format!("{}: {answer}", s.text);
        out.push_str(&format!(
            "- {}\n",
            list_item_text(&format!("{} ", s.id), &item)
        ));
    }
    // Closes the list, as the task's other lists are closed.
    out.push('\n');
}
