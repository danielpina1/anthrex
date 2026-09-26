//! Decision 17's prompts, exactly as the brief's Interfaces give them. A section whose
//! input is empty is omitted. A prompt never passes [`PROMPT_MAX_BYTES`]: inputs are cut
//! in a fixed order, each cut leaving [`CUT_MARKER`] in its place, so the same input
//! always gives the same prompt. Pure.

use super::{
    BlockedReasonInput, CheckSummaryInput, DeciderRequest, Evidence, SizeCheckInput, SizeCheckTask,
    TriageInput,
};
use proto::Size;

/// The most a decider prompt may carry.
pub const PROMPT_MAX_BYTES: usize = 128 * 1024;
/// What a cut leaves in place of what it removed.
pub const CUT_MARKER: &str = "[anthrex] … cut …";

/// Triage's report summary is cut to this many characters (after every path).
const TRIAGE_SUMMARY_CHARS: usize = 8000;
/// A size check's evidence summaries are cut to this many characters first.
const EVIDENCE_SUMMARY_CHARS: usize = 4000;
/// A size check's briefs are cut to this many characters last.
const BRIEF_CHARS: usize = 2000;

const TRIAGE_HEAD: &str = "[anthrex decider] triage v1
You label a coding goal for an orchestration engine. Answer with one JSON object that matches the schema, and nothing else.
kinds: every kind the goal needs. code changes behaviour; docs changes documentation, comments or configuration nothing executes; research investigates and reports without changing code; review reviews an existing branch or commit range.
scale: single when one task of size S or M does the whole goal; plan when it needs 2 to 12 tasks; large when it needs more, or two or more separate areas that each need several tasks.
Size S: one file, no interface change, about 20 changed lines or fewer. Size M: 1 to 3 files inside one module, about 100 changed lines or fewer. Anything bigger is not single.
When scale is single, give task: a short title; a brief a worker can follow without asking anything; acceptance criteria; the paths it owns, as globs relative to the repository root and as narrow as possible; its size; whether it changes an interface other code uses; its test mode (tdd for any change in behaviour, check for behaviour-preserving work already covered by tests, none for docs) with a one-line reason unless tdd; and for tdd the name of the test to write. Otherwise task is null.";

const SIZE_CHECK_HEAD: &str = "[anthrex decider] size_check v1
You check the size of planned coding tasks against what scouts found in the repository. Answer with one JSON object that matches the schema, and nothing else, with one entry per task id below.
Size S: one file, no interface change, a mechanical check exists, about 20 changed lines or fewer. Size M: 1 to 3 files inside one module, a clear spec, about 100 changed lines or fewer. Size L: files in more than one module plus an interface change, more than about 100 lines, an unclear spec, or a new dependency.
Judge each task from the evidence, not from its stated size. Give the size you believe and a one-sentence reason naming the evidence.";

const CHECK_SUMMARY_HEAD: &str = "[anthrex decider] check_summary v1
A check command failed in a coding task's checkout. Summarise the failure for the agent who must fix it, in at most 40 lines. Keep failing test names, error messages, file:line locations and assertion values exactly as they appear. Leave out passing tests, progress output and anything repeated. Answer with one JSON object that matches the schema, and nothing else.";

const BLOCKED_REASON_HEAD: &str = "[anthrex decider] blocked_reason v1
A coding agent stopped its task and gave the reason below without saying what kind of block it is. Classify it. question: it needs an answer or a decision about the task. mis_sized: the task is bigger than one task, or needs changes outside the paths it owns. environment: a tool, command, permission, dependency or setup is broken or missing. Answer with one JSON object that matches the schema, and nothing else.";

/// The prompt for `request`, at most [`PROMPT_MAX_BYTES`].
pub fn render(request: &DeciderRequest) -> String {
    let prompt = match request {
        DeciderRequest::Triage(input) => triage(input),
        DeciderRequest::SizeCheck(input) => size_check(input),
        DeciderRequest::CheckSummary(input) => check_summary(input),
        DeciderRequest::BlockedReason(input) => blocked_reason(input),
    };
    clamp(prompt)
}

fn fits(prompt: &str) -> bool {
    prompt.len() <= PROMPT_MAX_BYTES
}

/// The largest `n` in `0..=max` for which `make(n)` fits, if any; `make`'s length must
/// grow with `n`.
fn most_that_fit(max: usize, make: impl Fn(usize) -> String) -> Option<usize> {
    if !fits(&make(0)) {
        return None;
    }
    let (mut lo, mut hi) = (0, max);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(&make(mid)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Some(lo)
}

/// The last resort, when every ordered cut still leaves too much (an enormous goal,
/// command or reason): the prompt's head, cut on a character boundary, and the marker.
fn clamp(prompt: String) -> String {
    if fits(&prompt) {
        return prompt;
    }
    let keep = PROMPT_MAX_BYTES - CUT_MARKER.len() - 1;
    format!("{}\n{CUT_MARKER}", head_bytes(&prompt, keep))
}

/// The longest prefix of `s` of at most `max` bytes that ends on a character boundary.
fn head_bytes(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The longest suffix of `s` of at most `max` bytes that starts on a character boundary.
fn tail_bytes(s: &str, max: usize) -> &str {
    let mut start = s.len().saturating_sub(max);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

/// `s` cut to `max` characters, followed by the marker when anything was cut.
fn cut_chars(s: &str, max: Option<usize>) -> String {
    match max {
        Some(max) if s.chars().count() > max => {
            format!("{}{CUT_MARKER}", s.chars().take(max).collect::<String>())
        }
        _ => s.to_string(),
    }
}

/// The head and the non-empty sections, separated by blank lines.
fn join(head: &str, sections: Vec<String>) -> String {
    let mut out = head.to_string();
    for section in sections.into_iter().filter(|s| !s.is_empty()) {
        out.push_str("\n\n");
        out.push_str(&section);
    }
    out
}

// ---- triage -------------------------------------------------------------------------

/// How much of triage's cuttable input a rendering keeps: `None` keeps all.
#[derive(Clone, Copy, Default)]
struct TriageCut {
    files: Option<usize>,
    report_files: Option<usize>,
    summary: Option<usize>,
}

/// Triage's cutting order: tracked files (from the end), then the report's file list,
/// then the report summary to 8000 characters. After the last cut, the refill (ruling
/// M2) re-adds what was dropped in reverse cut order, each item whole, while it fits:
/// the report's files first, then tracked paths.
fn triage(input: &TriageInput) -> String {
    let full = triage_with(input, TriageCut::default());
    if fits(&full) {
        return full;
    }
    let files = |n| TriageCut {
        files: Some(n),
        ..TriageCut::default()
    };
    if let Some(n) = most_that_fit(input.files.len(), |n| triage_with(input, files(n))) {
        return triage_with(input, files(n));
    }
    let report = |n| TriageCut {
        report_files: Some(n),
        ..files(0)
    };
    let max = input.report_files.len();
    let mut cut = match most_that_fit(max, |n| triage_with(input, report(n))) {
        Some(n) => report(n),
        None => TriageCut {
            summary: Some(TRIAGE_SUMMARY_CHARS),
            ..report(0)
        },
    };
    // The refill: every list's count only grows, from what the cuts left.
    let with_report = |cut: TriageCut, n| TriageCut {
        report_files: Some(n),
        ..cut
    };
    if let Some(n) = most_that_fit(max, |n| triage_with(input, with_report(cut, n))) {
        cut = with_report(cut, n);
    }
    let with_files = |cut: TriageCut, n| TriageCut {
        files: Some(n),
        ..cut
    };
    let paths = input.files.len();
    if let Some(n) = most_that_fit(paths, |n| triage_with(input, with_files(cut, n))) {
        cut = with_files(cut, n);
    }
    triage_with(input, cut)
}

/// The first `keep` items (all when `None`), then the marker when any were cut.
fn kept(items: &[String], keep: Option<usize>) -> (Vec<&str>, bool) {
    let n = keep.unwrap_or(items.len()).min(items.len());
    (
        items[..n].iter().map(String::as_str).collect(),
        n < items.len(),
    )
}

fn triage_with(input: &TriageInput, cut: TriageCut) -> String {
    let goal = if input.goal.is_empty() {
        String::new()
    } else {
        format!("Goal:\n{}", input.goal)
    };
    let profile = if input.profile_summary.is_empty() {
        String::new()
    } else {
        format!("Repository profile:\n{}", input.profile_summary)
    };
    let summary = input.report_summary.as_deref().unwrap_or("");
    let report = if summary.is_empty() && input.report_files.is_empty() {
        String::new()
    } else {
        let mut lines = vec!["Onboarding scout report:".to_string()];
        if !summary.is_empty() {
            lines.push(cut_chars(summary, cut.summary));
        }
        if !input.report_files.is_empty() {
            let (mut names, was_cut) = kept(&input.report_files, cut.report_files);
            if was_cut {
                names.push(CUT_MARKER);
            }
            lines.push(format!("Files it named: {}", names.join(", ")));
        }
        lines.join("\n")
    };
    let tracked = if input.files.is_empty() {
        String::new()
    } else {
        let (mut paths, was_cut) = kept(&input.files, cut.files);
        let shown = paths.len();
        if was_cut {
            paths.push(CUT_MARKER);
        }
        format!(
            "Tracked files ({shown} of {}):\n{}",
            input.files_total,
            paths.join("\n")
        )
    };
    join(TRIAGE_HEAD, vec![goal, profile, report, tracked])
}

// ---- size check ---------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct SizeCut {
    summary: Option<usize>,
    reports: Option<usize>,
    brief: Option<usize>,
}

/// The size check's cutting order: evidence summaries to 4000 characters, then whole
/// reports from the last, then briefs to 2000 characters. After the last cut, the
/// refill (ruling M2) re-adds dropped reports, whole and in order, while they fit, so
/// no evidence is lost while room remains.
fn size_check(input: &SizeCheckInput) -> String {
    let full = size_check_with(input, SizeCut::default());
    if fits(&full) {
        return full;
    }
    let summaries = SizeCut {
        summary: Some(EVIDENCE_SUMMARY_CHARS),
        ..SizeCut::default()
    };
    let with_summaries = size_check_with(input, summaries);
    if fits(&with_summaries) {
        return with_summaries;
    }
    let reports = |n| SizeCut {
        reports: Some(n),
        ..summaries
    };
    let max = input.evidence.len();
    if let Some(n) = most_that_fit(max, |n| size_check_with(input, reports(n))) {
        return size_check_with(input, reports(n));
    }
    let briefs = |n| SizeCut {
        brief: Some(BRIEF_CHARS),
        ..reports(n)
    };
    let n = most_that_fit(max, |n| size_check_with(input, briefs(n))).unwrap_or(0);
    size_check_with(input, briefs(n))
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn size_letter(size: Size) -> &'static str {
    match size {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

fn task_block(task: &SizeCheckTask, brief_max: Option<usize>) -> String {
    let mut lines = vec![
        format!(
            "- id {}, stated size {}, hub {}, interface change {}",
            task.id,
            size_letter(task.size),
            yes_no(task.hub),
            yes_no(task.interface_change)
        ),
        format!("  title: {}", task.title),
    ];
    let mut line = |label: &str, value: String| {
        if !value.is_empty() {
            lines.push(format!("  {label}: {value}"));
        }
    };
    line("owns", task.owns.join(", "));
    line("depends on", task.deps.join(", "));
    line("acceptance", task.acceptance.join("; "));
    line("brief", cut_chars(&task.brief, brief_max));
    lines.join("\n")
}

fn evidence_block(e: &Evidence, summary_max: Option<usize>) -> String {
    let mut lines = vec![format!("## {}", e.id)];
    if !e.summary.is_empty() {
        lines.push(cut_chars(&e.summary, summary_max));
    }
    for (label, items) in [
        ("Files", &e.files),
        ("Modules", &e.modules),
        ("Interfaces", &e.interfaces),
    ] {
        if !items.is_empty() {
            lines.push(format!("{label}: {}", items.join(", ")));
        }
    }
    lines.join("\n")
}

fn size_check_with(input: &SizeCheckInput, cut: SizeCut) -> String {
    let mut areas = Vec::new();
    if !input.modules.is_empty() {
        areas.push(format!("Modules: {}", input.modules.join(", ")));
    }
    if !input.hub.is_empty() {
        areas.push(format!("Hub: {}", input.hub.join(", ")));
    }
    let tasks = if input.tasks.is_empty() {
        String::new()
    } else {
        let blocks: Vec<String> = input
            .tasks
            .iter()
            .map(|t| task_block(t, cut.brief))
            .collect();
        format!("Tasks:\n{}", blocks.join("\n"))
    };
    let evidence = if input.evidence.is_empty() {
        String::new()
    } else {
        let n = cut.reports.unwrap_or(input.evidence.len());
        let mut blocks: Vec<String> = input.evidence[..n.min(input.evidence.len())]
            .iter()
            .map(|e| evidence_block(e, cut.summary))
            .collect();
        if n < input.evidence.len() {
            blocks.push(CUT_MARKER.to_string());
        }
        format!("Scout evidence:\n{}", blocks.join("\n\n"))
    };
    join(SIZE_CHECK_HEAD, vec![areas.join("\n"), tasks, evidence])
}

// ---- check summary ------------------------------------------------------------------

/// The check summary cuts the tail from its start: whole lines first, then, when even
/// its last line is too long, that line's end.
fn check_summary(input: &CheckSummaryInput) -> String {
    let full = check_summary_with(input, &input.tail, false);
    if fits(&full) || input.tail.is_empty() {
        return full;
    }
    let lines: Vec<&str> = input.tail.split('\n').collect();
    let tail_from = |dropped: usize| lines[dropped..].join("\n");
    // Keeping more lines only grows the prompt: find the most that fit.
    let keep = most_that_fit(lines.len(), |k| {
        check_summary_with(input, &tail_from(lines.len() - k), true)
    });
    match keep {
        Some(k) if k > 0 => check_summary_with(input, &tail_from(lines.len() - k), true),
        Some(_) => {
            let bare = check_summary_with(input, "", true);
            // The kept end follows the marker on its own line: count that `\n`.
            let room = PROMPT_MAX_BYTES.saturating_sub(bare.len() + 1);
            let last = lines.last().copied().unwrap_or("");
            check_summary_with(input, tail_bytes(last, room), true)
        }
        None => full,
    }
}

fn check_summary_with(input: &CheckSummaryInput, tail: &str, was_cut: bool) -> String {
    let result = match (input.timed_out, input.code) {
        (true, _) => "timed out".to_string(),
        (false, Some(code)) => format!("exit {code}"),
        (false, None) => "killed by a signal".to_string(),
    };
    let mut section = format!("Command: {}\nResult: {result}", input.command);
    if !input.tail.is_empty() {
        let shown = if tail.is_empty() {
            0
        } else {
            tail.split('\n').count()
        };
        section.push_str(&format!("\nOutput (last {shown} lines):\n"));
        if was_cut {
            section.push_str(CUT_MARKER);
            if !tail.is_empty() {
                section.push('\n');
            }
        }
        section.push_str(tail);
    }
    join(CHECK_SUMMARY_HEAD, vec![section])
}

// ---- blocked reason -----------------------------------------------------------------

fn blocked_reason(input: &BlockedReasonInput) -> String {
    let task = format!("Task {}: {}", input.task_id, input.title);
    let reason = if input.reason.is_empty() {
        String::new()
    } else {
        format!("Reason:\n{}", input.reason)
    };
    let body = [task, reason]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    join(BLOCKED_REASON_HEAD, vec![body])
}
