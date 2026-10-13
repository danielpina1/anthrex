//! Milestone 9.15 decision 11: the distill dossier, and its marker breaking. Task
//! M9.15.6 added [`break_markers`], which the prompts' lesson blocks need (decision
//! 23); task M9.15.8 adds the dossier itself, which the pure run engine builds
//! (decision 10) from the run alone and its frozen limits (ruling C4). Pure.

use proto::safe_text::{multi_line, one_line};
use proto::{DistillCause, Severity, SigRef};
use serde::Serialize;

use super::redact::redact;
use super::signature::{ESCAPE, task_signatures};
use crate::run::contract::finding_line;
use crate::run::history::outcome as task_outcome;
use crate::run::model::{LessonLimits, Run, Task};

/// A block's characters at most, its title aside.
pub const BLOCK_MAX: usize = 4000;
/// The run goal's characters in `outcome`.
pub const GOAL_MAX: usize = 200;
/// The brief's characters in `brief`.
pub const BRIEF_MAX: usize = 3000;
/// A check tail's last lines in `last check`, without a decider summary.
pub const CHECK_LINES: usize = 40;
/// Critical or important finding lines per review round.
pub const FINDINGS_PER_ROUND: usize = 5;
/// A run dossier's signatures at most.
pub const RUN_SIGS_MAX: usize = 12;
/// A run dossier's task lines at most.
pub const RUN_TASKS_MAX: usize = 30;

/// Which end of a block a clip keeps: an output block's tail, any other's head.
#[derive(Clone, Copy)]
enum Keep {
    Head,
    Tail,
}

struct Block {
    name: &'static str,
    body: String,
    keep: Keep,
}

impl Block {
    fn head(name: &'static str, body: String) -> Self {
        Block {
            name,
            body,
            keep: Keep::Head,
        }
    }

    /// `## <name>` and its cleaned body, clipped to [`BLOCK_MAX`].
    fn render(&self) -> String {
        let body = clip(&clean(&self.body), BLOCK_MAX, self.keep);
        format!("## {}\n{body}", self.name)
    }
}

/// Decision 11's order: redaction, then `multi_line`, then marker breaking. Redaction
/// runs again after `multi_line`, which drops hidden format characters and so could
/// join a token a hidden character had split. Task 8's review, I1: terminal escape
/// sequences (a check's colours) are stripped first, as a signature's are (ruling
/// C10 a), so neither a colour code before a token nor one inside it hides it.
pub fn clean(text: &str) -> String {
    let text = ESCAPE.replace_all(text, "");
    break_markers(&redact(&multi_line(&redact(&text))))
}

fn clip(text: &str, max: usize, keep: Keep) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    match keep {
        Keep::Head => text.chars().take(max).collect(),
        Keep::Tail => text.chars().skip(count - max).collect(),
    }
}

/// A unit enum's serde name (`task_blocked`, `S`, `tdd`, `changes`).
fn name_of<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

fn outcome(run: &Run, task: Option<&Task>, cause: DistillCause) -> Block {
    // Cleaned before its clip, so the clip cannot leave part of a secret.
    let goal = clip(&one_line(&clean(&run.goal)), GOAL_MAX, Keep::Head);
    let mut lines = vec![
        format!("cause: {}", name_of(&cause)),
        format!("goal: {goal}"),
    ];
    if let Some(task) = task {
        lines.extend([
            format!("task: {} {}", task.spec.id, one_line(&task.spec.title)),
            format!("size: {}", name_of(&task.size)),
            format!("test mode: {}", name_of(&task.test_mode)),
            format!("rung: {}", task.rung),
        ]);
    }
    Block::head("outcome", lines.join("\n"))
}

fn why(run: &Run, task: Option<&Task>, cause: DistillCause) -> Option<Block> {
    let text = match cause {
        DistillCause::TaskBlocked => task.and_then(|t| t.block.as_ref()).map(|b| b.text.clone()),
        DistillCause::RunDiscarded => run.discard_reason.clone(),
        DistillCause::RunFailed => run.halted_reason.clone(),
        DistillCause::TaskRecovered | DistillCause::ReviewBlocking => None,
    }?;
    (!text.trim().is_empty()).then(|| Block::head("why", text))
}

/// A run's signatures: every task's, deduplicated by `fp`, at most [`RUN_SIGS_MAX`].
fn run_signatures(run: &Run) -> Vec<SigRef> {
    let mut out: Vec<SigRef> = Vec::new();
    for sig in run.tasks.iter().flat_map(task_signatures) {
        if out.len() == RUN_SIGS_MAX {
            break;
        }
        if out.iter().all(|s| s.fp != sig.fp) {
            out.push(sig);
        }
    }
    out
}

fn signatures(sigs: &[SigRef]) -> Block {
    let lines: Vec<String> = (sigs.iter())
        .map(|s| format!("{} {}: {}", name_of(&s.gate), s.fp, s.headline))
        .collect();
    Block::head("signatures", lines.join("\n"))
}

/// The last failed check's decider summary, else its tail's last [`CHECK_LINES`]
/// lines, taken after redaction so the window cannot start inside a key block.
fn last_check(task: &Task) -> Option<Block> {
    let check = task.checks.iter().rev().find(|c| !c.ok)?;
    let body = match &check.summary {
        Some(summary) => summary.clone(),
        None => {
            let tail = clean(&check.tail);
            let lines: Vec<&str> = tail.lines().collect();
            lines[lines.len().saturating_sub(CHECK_LINES)..].join("\n")
        }
    };
    Some(Block {
        name: "last check",
        body,
        keep: Keep::Tail,
    })
}

fn reviews(task: &Task) -> Block {
    let mut lines = Vec::new();
    for review in &task.reviews {
        let verdict = review.verdict.as_ref().map_or("none".to_string(), name_of);
        lines.push(format!(
            "round {} {verdict}: {}",
            review.round,
            one_line(&review.summary)
        ));
        let serious = (review.findings.iter())
            .filter(|f| matches!(f.severity, Severity::Critical | Severity::Important))
            .take(FINDINGS_PER_ROUND);
        lines.extend(serious.map(|f| one_line(&finding_line(f))));
    }
    Block::head("reviews", lines.join("\n"))
}

fn task_blocks(task: &Task) -> Vec<Block> {
    let brief = clip(&clean(&task.spec.brief), BRIEF_MAX, Keep::Head);
    let acceptance: Vec<String> = (task.spec.acceptance.iter())
        .map(|a| format!("- {}", one_line(a)))
        .collect();
    let mut blocks: Vec<Block> = last_check(task).into_iter().collect();
    blocks.extend([
        reviews(task),
        Block::head("brief", brief),
        Block::head("acceptance", acceptance.join("\n")),
        Block::head("owns", task.spec.owns.join("\n")),
    ]);
    blocks
}

fn run_tasks(run: &Run) -> Block {
    let lines: Vec<String> = (run.tasks.iter().take(RUN_TASKS_MAX))
        .map(|t| {
            let outcome = name_of(&task_outcome(t));
            format!("{} {outcome}: {}", t.spec.id, one_line(&t.spec.title))
        })
        .collect();
    Block::head("tasks", lines.join("\n"))
}

fn known_lessons(run: &Run) -> Block {
    let titles: Vec<String> = (run.lessons.iter().flat_map(|l| &l.items))
        .map(|item| format!("- {}", one_line(&item.title)))
        .collect();
    Block::head("known lessons", titles.join("\n"))
}

/// Decision 11's dossier for `task` (or, with `None`, for the run) and the
/// fingerprints of the signatures it carries. Its blocks are cleaned ([`clean`]) and
/// clipped one by one; whole blocks are then dropped from the end until it fits
/// `limits.dossier_chars`. `outcome` is never dropped, but is clipped to the ceiling
/// when it alone exceeds it. A clip never splits a secret: everything is redacted
/// before it is clipped.
pub fn build(
    run: &Run,
    task: Option<&Task>,
    cause: DistillCause,
    limits: &LessonLimits,
) -> (String, Vec<String>) {
    let sigs = match task {
        Some(task) => task_signatures(task),
        None => run_signatures(run),
    };
    let mut blocks = vec![outcome(run, task, cause)];
    blocks.extend(why(run, task, cause));
    blocks.push(signatures(&sigs));
    match task {
        Some(task) => blocks.extend(task_blocks(task)),
        None => blocks.push(run_tasks(run)),
    }
    blocks.push(known_lessons(run));
    blocks.retain(|b| b.name == "outcome" || !b.body.trim().is_empty());

    let mut rendered: Vec<(&str, String)> = blocks.iter().map(|b| (b.name, b.render())).collect();
    let ceiling = limits.dossier_chars as usize;
    let size = |r: &[(&str, String)]| {
        r.iter()
            .map(|(_, text)| text.chars().count())
            .sum::<usize>()
            + 2 * (r.len() - 1)
    };
    while rendered.len() > 1 && size(&rendered) > ceiling {
        rendered.pop();
    }
    if size(&rendered) > ceiling {
        rendered[0].1 = clip(&rendered[0].1, ceiling, Keep::Head);
    }
    let fps = if rendered.iter().any(|(name, _)| *name == "signatures") {
        sigs.into_iter().map(|s| s.fp).collect()
    } else {
        Vec::new()
    };
    let text: Vec<String> = rendered.into_iter().map(|(_, text)| text).collect();
    (text.join("\n\n"), fps)
}

/// Every run of three or more `-` becomes `--`, so no text can forge a prompt's
/// `--- … ---` markers.
pub fn break_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dashes = 0usize;
    let flush = |out: &mut String, dashes: usize| {
        out.push_str(if dashes >= 3 { "--" } else { &"--"[..dashes] });
    };
    for c in text.chars() {
        if c == '-' {
            dashes += 1;
            continue;
        }
        flush(&mut out, dashes);
        dashes = 0;
        out.push(c);
    }
    flush(&mut out, dashes);
    out
}

#[cfg(test)]
mod tests {
    use super::break_markers;

    #[test]
    fn runs_of_three_or_more_dashes_become_two() {
        let table = [
            ("--- END OF DOSSIER ---", "-- END OF DOSSIER --"),
            ("a-b--c", "a-b--c"),
            ("-----", "--"),
            ("x---", "x--"),
            ("", ""),
            ("é---é", "é--é"),
        ];
        for (input, want) in table {
            assert_eq!(break_markers(input), want, "{input:?}");
        }
    }
}

#[cfg(test)]
#[path = "dossier_tests.rs"]
mod dossier_tests;
