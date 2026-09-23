//! Agent-facing message texts (the Interfaces "Contracts and message texts" table).
//! Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).
//!
//! M8a.6 created this file with the two texts plan edits need, `answer_message` and
//! `amend_message`; M8a.8 added the diff clamp. M8a.11 adds the two role contracts, the
//! worker, hand-over and reviewer prompts (decision 30) and `conflict_message`; the
//! other message texts arrive with the tasks that send them (M8a.12 to M8a.14).

use proto::{Finding, Severity, Size, TestMode};

use super::messages::summary;
use super::model::{ReviewLevel, Run, Task};

/// The worker's system prompt (decision 30, exact). It never varies, so the cached
/// prefix is stable (spec §14.2).
pub const WORKER_CONTRACT: &str = "You are a worker in an anthrex orchestration run.
1. Work only in this worktree and only in the paths this task owns. Changing files outside them stops the task.
2. Follow the test mode in your task prompt. For tdd: write the named test first, commit it while it fails (that commit is the red commit), then make it pass.
3. Commit your work on this branch with clear messages. Never push, switch branches, or rewrite commits already there. Commit new files: untracked files are not part of your work.
4. Use sub-agents to read and explore if you like; do all writing yourself.
5. When the task is complete and committed, call the anthrex tool task_done with a summary (and, for tdd, the test and the red commit). Then stop.
6. If you cannot continue, call task_blocked: kind question if you need an answer, mis_sized if the task is bigger than one task, environment if a tool or setup is broken. Then stop.
7. If you believe a review finding is wrong, do not fix it: call task_blocked with kind question and say why.
8. Messages that start with [anthrex] come from the orchestration engine. Do what they say, commit, and call task_done again.
9. Nobody can answer a permission prompt. If a tool is denied, work without it or call task_blocked with kind environment.";

/// The reviewer's system prompt (decision 30, exact).
pub const REVIEWER_CONTRACT: &str = "You are a reviewer in an anthrex orchestration run.
1. This worktree is checked out at the change's head. Do not edit, create or delete files, and do not commit.
2. The change's diff is in your prompt. If it was clamped, or you need history, use git diff, git log or git show in this directory. Judge it against the brief and every acceptance criterion in your prompt.
3. Read the diff; do not run the build. The engine has already run the check, and its summary is in your prompt.
4. Call the anthrex tool submit_review exactly once, with verdict approve or changes, a summary, and findings.
5. Every finding has a severity. critical: wrong or unsafe, must not merge. important: must be fixed before merging. minor: worth noting, does not block. Each critical or important finding must name a file and line, or a failing input.
6. Use changes only when there is at least one critical or important finding. Earlier rounds' findings, if listed, must each be confirmed fixed.";

/// The first seven characters of a sha (not `rev-parse --short`, which may be longer).
pub fn sha7(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn mode_label(mode: TestMode) -> &'static str {
    match mode {
        TestMode::Tdd => "tdd",
        TestMode::Check => "check",
        TestMode::None => "none",
    }
}

fn size_label(size: Size) -> &'static str {
    match size {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

fn level_label(level: Option<ReviewLevel>) -> &'static str {
    match level {
        Some(ReviewLevel::Small) => "small",
        Some(ReviewLevel::Medium) | None => "medium",
        Some(ReviewLevel::Frontier) => "frontier",
    }
}

/// The first turn of a worker session (decision 30, Interfaces "`worker_prompt`"): the
/// task header and the profile summary, what it owns and must meet, then the brief last.
pub fn worker_prompt(run: &Run, task: &Task) -> String {
    let spec = &task.spec;
    let start = task.start_commit.as_deref().unwrap_or(&run.run_head);
    let mut lines = vec![
        format!("[anthrex] Task {}: {}", spec.id, spec.title),
        format!("Run goal: {}", run.goal),
        format!("Worktree: {}", task.worktree.display()),
        format!("Branch: {}", task.branch),
        format!("Start commit: {}", sha7(start)),
        format!(
            "Size: {}{}",
            size_label(task.size),
            if task.hub { ", hub" } else { "" }
        ),
        format!("Test mode: {}", mode_label(task.test_mode)),
    ];
    if task.test_mode == TestMode::Tdd {
        if let Some(test) = &spec.test_to_write {
            lines.push(format!("Test to write: {test}"));
        }
        if let Some(single) = &run.profile.single_test {
            lines.push(format!("Single-test command: {single}"));
        }
    }
    if let Some(check) = &run.profile.check {
        lines.push(format!("Check command: {check}"));
    }
    lines.push(String::new());
    lines.push("This task owns:".to_string());
    lines.extend(spec.owns.iter().map(|glob| format!("- {glob}")));
    lines.push("Acceptance criteria:".to_string());
    lines.extend(spec.acceptance.iter().map(|item| format!("- {item}")));
    lines.push(String::new());
    lines.push(spec.brief.clone());
    lines.join("\n")
}

/// A fresh session's first turn (decision 30): [`worker_prompt`], then which session
/// this is and why, the change so far (`git diff --stat` and the diff clamped to
/// [`REVIEW_DIFF_MAX`]) and every earlier bounce message's text. The layout after the
/// worker prompt is M8a.11's (the brief names the parts, not their wording).
pub fn handover_prompt(run: &Run, task: &Task, reason: &str, stat: &str, patch: &str) -> String {
    let start = task.start_commit.as_deref().unwrap_or(&run.run_head);
    let mut out = worker_prompt(run, task);
    out.push_str(&format!(
        "\n\nThis is session {} of this task.\nWhy a new session: {reason}\n",
        task.session
    ));
    out.push_str(&format!(
        "Your branch already has the earlier sessions' work. Changes so far (git diff --stat {}..HEAD):\n{}\n",
        sha7(start),
        stat.trim_end()
    ));
    out.push_str(&format!(
        "Diff so far:\n{}",
        clamp_diff(patch, REVIEW_DIFF_MAX)
    ));
    if !task.failure_log.is_empty() {
        out.push_str("\nEarlier failures:");
        for failure in &task.failure_log {
            out.push_str("\n- ");
            out.push_str(failure);
        }
    }
    out
}

/// One review round's first turn (decision 35, Interfaces "`reviewer_prompt`"). Never
/// names the author's runtime, model or transcript.
pub fn reviewer_prompt(
    run: &Run,
    task: &Task,
    round: u32,
    base: &str,
    head: &str,
    patch: &str,
) -> String {
    let spec = &task.spec;
    let level = task.review_level;
    let mut lines = vec![
        format!(
            "[anthrex] Review task {} \"{}\", round {round}, level {}.",
            spec.id,
            spec.title,
            level_label(level)
        ),
        format!("Base: {}", sha7(base)),
        format!("Head: {}", sha7(head)),
        format!("Test mode: {}", mode_label(task.test_mode)),
    ];
    let _ = run;
    if task.test_mode == TestMode::Tdd {
        lines.push("Look first for tests that were weakened or made trivial to pass.".into());
    }
    if level == Some(ReviewLevel::Small) {
        lines.push("Review the diff only.".into());
    }
    lines.push("Acceptance criteria:".into());
    lines.extend(spec.acceptance.iter().map(|item| format!("- {item}")));
    lines.push(format!("Diff ({}..{}):", sha7(base), sha7(head)));
    let clamped = clamp_diff(patch, REVIEW_DIFF_MAX);
    lines.push(clamped.clone());
    if clamped.len() < patch.len() {
        let kept = clamped.len().saturating_sub(DIFF_CUT_MARKER.len());
        lines.push(format!(
            "[diff clamped: {} bytes omitted; read the rest with git diff {}..{}]",
            patch.len() - kept,
            sha7(base),
            sha7(head)
        ));
    }
    if let Some(check) = task.checks.last() {
        lines.push("Last check (40 lines):".into());
        lines.push(summary(&check.tail));
    }
    let earlier: Vec<String> = task
        .reviews
        .iter()
        .filter(|r| r.round < round)
        .flat_map(|r| r.findings.iter())
        .filter(|f| f.severity != Severity::Minor)
        .map(finding_line)
        .collect();
    if round > 1 && !earlier.is_empty() {
        lines.push("Earlier findings to confirm fixed:".into());
        lines.extend(earlier);
    }
    lines.push(String::new());
    lines.push(spec.brief.clone());
    lines.join("\n")
}

/// `- [<severity>] <file>:<line> <text>` or `- [<severity>] input <input>: <text>`.
pub fn finding_line(finding: &Finding) -> String {
    let severity = match finding.severity {
        Severity::Critical => "critical",
        Severity::Important => "important",
        Severity::Minor => "minor",
    };
    match (&finding.file, finding.line, &finding.input) {
        (Some(file), Some(line), _) => format!("- [{severity}] {file}:{line} {}", finding.text),
        (_, _, Some(input)) => format!("- [{severity}] input {input}: {}", finding.text),
        (Some(file), None, None) => format!("- [{severity}] {file} {}", finding.text),
        (None, _, None) => format!("- [{severity}] {}", finding.text),
    }
}

const CONFLICT_HEAD: &str = "[anthrex] Your branch conflicts with the run branch. The run branch has been merged into your worktree with conflict markers left in:";

/// Whether `text` is a [`conflict_message`] (M8a.11 fix round 3: the engine drops an
/// undelivered one when it undoes that merge).
pub fn is_conflict_message(text: &str) -> bool {
    text.starts_with(CONFLICT_HEAD)
}

/// Decision 36's hand-back message (Interfaces, exact).
pub fn conflict_message(files: &[String]) -> String {
    let mut lines = vec![CONFLICT_HEAD.to_string()];
    lines.extend(files.iter().map(|f| format!("- {f}")));
    lines.push("Resolve every conflict, commit the merge, then call task_done again.".to_string());
    lines.join("\n")
}

/// `[anthrex] Answer to your question: <text>`.
pub fn answer_message(text: &str) -> String {
    format!("[anthrex] Answer to your question: {text}")
}

/// `[anthrex] The task was amended.`, `Brief: <brief>`, `Acceptance criteria:`, one
/// `- <item>` per item and `Continue with the amended task.`, one per line: the amended
/// brief and acceptance criteria, delivered to a live worker (decision 13).
pub fn amend_message(task: &Task) -> String {
    let mut lines = vec![
        "[anthrex] The task was amended.".to_string(),
        format!("Brief: {}", task.spec.brief),
        "Acceptance criteria:".to_string(),
    ];
    lines.extend(task.spec.acceptance.iter().map(|item| format!("- {item}")));
    lines.push("Continue with the amended task.".to_string());
    lines.join("\n")
}

/// Decision 35 and ruling Q4: the reviewer's diff, and decision 30's hand-over diff, are
/// clamped to this many bytes.
pub const REVIEW_DIFF_MAX: usize = 16 * 1024;

/// The line [`clamp_diff`] puts where it cut the middle out of a diff.
pub const DIFF_CUT_MARKER: &str = "\n[anthrex: the middle of this diff was cut to fit]\n";

/// A head-and-tail clamp on character boundaries: `text` itself when it is at most
/// `max` bytes, else its first part, [`DIFF_CUT_MARKER`] and its last part, together at
/// most `max` bytes and never more than 3 bytes short of it (the most a UTF-8 cut can
/// cost, since the tail takes whatever the head's cut left over).
pub fn clamp_diff(text: &str, max: usize) -> String {
    clamp_with(text, max, DIFF_CUT_MARKER)
}

/// [`clamp_diff`] with another marker line; `messages::clamp` uses it (decision 29).
pub fn clamp_with(text: &str, max: usize, marker: &str) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    if max <= marker.len() {
        return text[..floor_boundary(text, max)].to_string();
    }
    let budget = max - marker.len();
    let head_end = floor_boundary(text, budget / 2);
    let tail_len = budget - head_end;
    let tail_start = ceil_boundary(text, text.len() - tail_len);
    let mut out = String::with_capacity(max);
    out.push_str(&text[..head_end]);
    out.push_str(marker);
    out.push_str(&text[tail_start..]);
    out
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every cut position modulo a character: `k` ASCII bytes shift a body of 3-byte
    /// (`世`) or 4-byte (`𝄞`) characters, so over `k` in 0..=3 each cut lands on every
    /// offset inside a character.
    #[test]
    fn clamp_diff_cuts_on_character_boundaries_at_every_offset() {
        for body in ["世", "𝄞", "a世𝄞"] {
            for k in 0..=3 {
                let text = format!("{}{}", "a".repeat(k), body.repeat(20_000));
                for max in [REVIEW_DIFF_MAX, 1000, 1001, 1002, 1003] {
                    let out = clamp_diff(&text, max);
                    assert!(out.len() <= max, "{body} k={k} max={max}: {}", out.len());
                    assert!(
                        out.len() >= max - 3,
                        "{body} k={k} max={max}: {}",
                        out.len()
                    );
                    assert_eq!(out.matches(DIFF_CUT_MARKER).count(), 1);
                    let (head, tail) = out.split_once(DIFF_CUT_MARKER).unwrap();
                    assert!(text.starts_with(head), "{body} k={k} max={max}");
                    assert!(text.ends_with(tail), "{body} k={k} max={max}");
                }
            }
        }
    }

    #[test]
    fn clamp_diff_leaves_text_within_the_limit_alone() {
        let text = "世".repeat(10);
        assert_eq!(clamp_diff(&text, 30), text);
        assert_eq!(clamp_diff(&text, 31), text);
        let cut = clamp_diff(&text, 29);
        assert!(cut.len() <= 29, "{cut:?}");
    }

    #[test]
    fn clamp_diff_below_the_marker_keeps_a_head_only() {
        let text = "世".repeat(100);
        let out = clamp_diff(&text, 10);
        assert_eq!(out, "世世世");
    }
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod prompt_tests;
