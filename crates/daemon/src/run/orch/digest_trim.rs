//! The digest's trimming past [`DIGEST_MAX_BYTES`] (decision 16, and the M9.6 review
//! fixes that make the cap hold whatever the run holds). Pure.

use serde_json::{Value, json};

use super::{BLOCK_TEXT_TRIMMED, DIGEST_MAX_BYTES, LISTS_TRIMMED, NOTES_TRIMMED};
use crate::run::model::Run;
use crate::run::orch::json::{cut, shrink_strings, size};

/// Attention lines kept once the attention cut runs.
const ATTENTION_TRIMMED: usize = 10;
/// Scouts', planners', integration reviews' and wake notes' entries kept by the first
/// list cut, and scouts' and planners' by the second.
const ENTRIES_TRIMMED: usize = 10;
const ENTRIES_TRIMMED_AGAIN: usize = 3;
/// Every string's length after the general cut, and scouts' and planners' after the
/// second one, in characters.
const STRINGS_TRIMMED: usize = 120;
const ENTRY_STRINGS_TRIMMED: usize = 40;

/// Decision 16's trimming past the cap, in order: finished tasks dropped oldest first
/// (counted in `omitted_tasks`), `edits` cut to 3, `task_notes` to 3, `block.text` to
/// 200 characters (and the attention lines with it), `notes` to 5. Beyond the
/// Interfaces, so the cap always holds, texts and lists are cut before an unfinished
/// task goes: `attention` cut to 10 lines (those of tasks already dropped first, and
/// the run-wide lines kept ahead of the blocked tasks' ones), every string cut to 120
/// characters, `scouts`, `planners`, `integration` and `notes` cut to 10, then the
/// scouts' and planners' strings cut to 40 characters and both lists to 3; only then
/// unfinished tasks dropped from the end of the plan (counted too), and the attention
/// lines of the tasks dropped last removed with them.
pub(super) fn trim(digest: &mut Value, run: &Run) {
    let fits = |d: &Value| size(d) <= DIGEST_MAX_BYTES;
    if fits(digest) {
        return;
    }
    // Finished tasks, oldest first: by their newest history entry, then plan order.
    let mut finished: Vec<(u64, usize)> = run
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.state.is_finished())
        .map(|(i, t)| (t.history.last().map_or(0, |e| e.at), i))
        .collect();
    finished.sort();
    let order: Vec<&str> = finished.iter().map(|(_, i)| run.tasks[*i].id()).collect();
    for id in order {
        if fits(digest) {
            return;
        }
        drop_task(digest, id);
    }
    for key in ["edits", "task_notes"] {
        if fits(digest) {
            return;
        }
        truncate(digest, key, LISTS_TRIMMED);
    }
    if fits(digest) {
        return;
    }
    if let Some(Value::Array(tasks)) = digest.get_mut("tasks") {
        for text in tasks
            .iter_mut()
            .filter_map(|t| t.pointer_mut("/block/text"))
        {
            if let Value::String(s) = text {
                *s = cut(s, BLOCK_TEXT_TRIMMED);
            }
        }
    }
    if let Some(Value::Array(lines)) = digest.get_mut("attention") {
        for line in lines.iter_mut() {
            if let Value::String(s) = line {
                *s = cut(s, BLOCK_TEXT_TRIMMED);
            }
        }
    }
    if fits(digest) {
        return;
    }
    truncate(digest, "notes", NOTES_TRIMMED);
    if fits(digest) {
        return;
    }
    drop_attention_of_dropped_tasks(digest, run);
    cut_attention(digest, run);
    if fits(digest) {
        return;
    }
    shrink_strings(digest, STRINGS_TRIMMED);
    for key in ["scouts", "planners", "integration", "notes"] {
        if fits(digest) {
            return;
        }
        truncate(digest, key, ENTRIES_TRIMMED);
    }
    // Second review, Minor 1: a second shortening pass before any unfinished task goes.
    for key in ["scouts", "planners"] {
        if fits(digest) {
            return;
        }
        if let Some(entries) = digest.get_mut(key) {
            shrink_strings(entries, ENTRY_STRINGS_TRIMMED);
        }
    }
    for key in ["scouts", "planners"] {
        if fits(digest) {
            return;
        }
        truncate(digest, key, ENTRIES_TRIMMED_AGAIN);
    }
    while !fits(digest) {
        let last = match digest.get("tasks") {
            Some(Value::Array(tasks)) => tasks.last().and_then(|t| t["id"].as_str()),
            _ => None,
        };
        match last.map(str::to_string) {
            Some(id) => drop_task(digest, &id),
            None => break,
        }
    }
    drop_attention_of_dropped_tasks(digest, run);
}

/// The task a blocked task's attention line names (`<id> blocked (`), if the line is
/// one; every other line is run-wide (a moved base, a failed final check, a stale
/// profile, a promotion).
fn task_of<'a>(line: &'a str, run: &Run) -> Option<&'a str> {
    let (id, _) = line.split_once(" blocked (")?;
    run.tasks.iter().any(|t| t.id() == id).then_some(id)
}

/// `attention` cut to [`ATTENTION_TRIMMED`] lines, the run-wide lines first: only the
/// blocked tasks' lines are cut to fit (second review, Important).
fn cut_attention(digest: &mut Value, run: &Run) {
    let Some(Value::Array(lines)) = digest.get_mut("attention") else {
        return;
    };
    let (tasks, run_wide): (Vec<Value>, Vec<Value>) = std::mem::take(lines)
        .into_iter()
        .partition(|l| l.as_str().is_some_and(|l| task_of(l, run).is_some()));
    *lines = run_wide.into_iter().chain(tasks).collect();
    lines.truncate(ATTENTION_TRIMMED);
}

/// Removes the attention lines of the run's tasks that `tasks` no longer shows.
fn drop_attention_of_dropped_tasks(digest: &mut Value, run: &Run) {
    let shown: Vec<String> = match digest.get("tasks") {
        Some(Value::Array(tasks)) => tasks
            .iter()
            .filter_map(|t| t["id"].as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    };
    let dropped = |line: &str| task_of(line, run).is_some_and(|id| !shown.iter().any(|s| s == id));
    if let Some(Value::Array(lines)) = digest.get_mut("attention") {
        lines.retain(|l| !l.as_str().is_some_and(dropped));
    }
}

/// Removes task `id` from `tasks` and counts it in `omitted_tasks`.
fn drop_task(digest: &mut Value, id: &str) {
    if let Some(Value::Array(tasks)) = digest.get_mut("tasks") {
        tasks.retain(|t| t["id"].as_str() != Some(id));
    }
    if let Some(n) = digest.get_mut("omitted_tasks") {
        *n = json!(n.as_u64().unwrap_or(0) + 1);
    }
}

fn truncate(digest: &mut Value, key: &str, keep: usize) {
    if let Some(Value::Array(items)) = digest.get_mut(key) {
        items.truncate(keep);
    }
}
