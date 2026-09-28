//! What every run inspection shares (milestone 8c decisions 30 and 31): durations,
//! token counts, local `hh:mm`, progress bars and lines, bounded free text, the words
//! for routes, sizes and findings, and the `session` field. Pure: no clock is read.

use super::{Field, FieldLayout, Inspection};
use crate::app::App;
use crate::graph::paint::style::node_glyph;
use crate::tree::{NodeKey, Row, RowKind};
use proto::{
    BlockReason, Effort, Finding, Severity, Size, Strength, TaskInfo, TaskState, TestMode,
    TokenUsage, WindowKind,
};
use ratatui::style::Style;
use ratatui::text::Span;
use std::path::Path;

/// Free text from the snapshot (a block reason, a summary line, a finding) is cut to
/// this many characters, so one hostile value cannot make a row cost more than a row.
pub(super) const TEXT_MAX_CHARS: usize = 300;

/// Decision 31: `{s}s` under a minute, `{m}m` under an hour, else `{h}h{mm}m`.
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{:02}m", secs / 3600, secs % 3600 / 60)
    }
}

/// Decision 31: `{n}`, `{n/1000}k`, or `{n/1 000 000}.{tenths}M`.
pub fn format_tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{}k", n / 1000)
    } else {
        format!("{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000)
    }
}

/// Decision 31: a unix time as local `hh:mm`, the offset given, never read here.
pub fn local_hhmm(at: u64, utc_offset_secs: i64) -> String {
    let secs = (i128::from(at) + i128::from(utc_offset_secs)).rem_euclid(86_400);
    format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

/// Decision 30: `filled = (width × done + total / 2) / total` cells of `█`, then `░`.
/// A done count above the total fills the bar; a zero total leaves it empty.
pub fn progress_bar(done: u64, total: u64, width: usize) -> String {
    let filled = if total == 0 {
        0
    } else {
        let (done, total) = (u128::from(done), u128::from(total));
        let filled = (width as u128 * done + total / 2) / total;
        usize::try_from(filled).unwrap_or(width).min(width)
    };
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// `text` with every control character a space, cut to [`TEXT_MAX_CHARS`] with `…`.
pub(super) fn clean(text: &str) -> String {
    let mut out: String = text
        .chars()
        .take(TEXT_MAX_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if text.chars().nth(TEXT_MAX_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// The glyph a node of `kind` has on the canvas (decision 19), for its inspection.
pub(super) fn kind_glyph(kind: RowKind<'_>, app: &App) -> Span<'static> {
    let row = Row {
        key: NodeKey::Run(String::new()),
        guides: String::new(),
        depth: 0,
        kind,
    };
    let (glyph, color) = node_glyph(&row, app);
    Span::styled(glyph, Style::default().fg(color))
}

pub(super) fn rows(
    glyph: Span<'static>,
    name: String,
    right: String,
    fields: Vec<Field>,
) -> Inspection {
    Inspection {
        glyph,
        name,
        right: Some(right),
        fields,
        layout: FieldLayout::Rows,
    }
}

/// Tokens actually billed, as `TokenUsage::billable` counts them, but saturating: the
/// numbers come from a stream or an OTLP export and must never panic the view.
pub(super) fn billable(usage: &TokenUsage) -> u64 {
    usage
        .input
        .saturating_add(usage.cache_write)
        .saturating_add(usage.output)
}

/// `waiting`, `working`, … — decision 30's category of a task state, `None` for merged
/// and cancelled, which are counted apart.
fn category(state: TaskState) -> Option<usize> {
    match state {
        TaskState::Preparing | TaskState::Working => Some(0),
        TaskState::Proof | TaskState::Check => Some(1),
        TaskState::Review => Some(2),
        TaskState::MergeQueue => Some(3),
        TaskState::Blocked => Some(4),
        TaskState::Pending | TaskState::Queued => Some(5),
        TaskState::Merged | TaskState::Cancelled | TaskState::Reported => None,
    }
}

const CATEGORIES: [&str; 6] = [
    "working", "checking", "review", "merging", "blocked", "waiting",
];

/// Decision 30's progress line over `tasks`: the bar, `{merged}/{total} merged`, each
/// non-zero category, then the cancelled; `no tasks yet` when nothing counts.
pub(super) fn progress_text<'a>(tasks: impl Iterator<Item = &'a TaskInfo>, width: usize) -> String {
    let (mut merged, mut total, mut cancelled) = (0u64, 0u64, 0u64);
    let mut counts = [0u64; 6];
    for task in tasks {
        match task.state {
            TaskState::Cancelled => cancelled += 1,
            TaskState::Merged => merged += 1,
            state => counts[category(state).unwrap_or(5)] += 1,
        }
        total += u64::from(task.state != TaskState::Cancelled);
    }
    if total == 0 {
        return "no tasks yet".to_owned();
    }
    let mut text = format!(
        "{}  {merged}/{total} merged",
        progress_bar(merged, total, width)
    );
    for (count, name) in counts.iter().zip(CATEGORIES) {
        if *count > 0 {
            text.push_str(&format!(" · {count} {name}"));
        }
    }
    if cancelled > 0 {
        text.push_str(&format!(" · {cancelled} cancelled"));
    }
    text
}

/// Words for a block reason (Interfaces "Run", `attention`; "Task", the stage).
pub(super) fn reason_text(reason: BlockReason) -> &'static str {
    match reason {
        BlockReason::MisSized => "mis-sized",
        BlockReason::Human => "human",
        BlockReason::Conflict => "conflict",
        BlockReason::DepCancelled => "dependency cancelled",
        BlockReason::Question => "question",
        BlockReason::Environment => "environment",
        BlockReason::MessagePause => "paused(message)",
    }
}

/// `#{id} · window closed` when the id is not listed, else `#{id} · {kind} · {place} ·
/// Enter: conversation`; `no window yet` without an id.
pub(super) fn session_text(window_id: Option<u32>, place: &str, app: &App) -> String {
    let Some(id) = window_id else {
        return "no window yet".to_owned();
    };
    match app.windows.iter().find(|window| window.id == id) {
        None => format!("#{id} · window closed"),
        Some(window) => {
            let kind = match window.kind {
                WindowKind::Headless => "headless",
                WindowKind::Pty => "terminal",
            };
            format!("#{id} · {kind} · {place} · Enter: conversation")
        }
    }
}

pub(super) fn size_letter(size: Size) -> &'static str {
    match size {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

pub(super) fn test_mode_text(mode: TestMode) -> &'static str {
    match mode {
        TestMode::Tdd => "tdd",
        TestMode::Check => "check",
        TestMode::None => "none",
    }
}

pub(super) fn strength_text(strength: Strength) -> &'static str {
    match strength {
        Strength::Fast => "fast",
        Strength::Standard => "standard",
        Strength::Frontier => "frontier",
    }
}

pub(super) fn effort_text(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}

pub(super) fn severity_text(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::Important => "important",
        Severity::Minor => "minor",
    }
}

/// `{n} critical, {n} important, {n} minor`, the non-zero ones.
pub(super) fn counts_text(findings: &[Finding]) -> String {
    [Severity::Critical, Severity::Important, Severity::Minor]
        .into_iter()
        .filter_map(|severity| {
            let n = findings.iter().filter(|f| f.severity == severity).count();
            (n > 0).then(|| format!("{n} {}", severity_text(severity)))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn rank(severity: Severity) -> u8 {
    match severity {
        Severity::Critical => 0,
        Severity::Important => 1,
        Severity::Minor => 2,
    }
}

pub(super) fn most_severe(findings: &[Finding]) -> Option<&Finding> {
    findings.iter().min_by_key(|finding| rank(finding.severity))
}

/// `{file name}:{line}`, or the finding's `input` when it names no file.
pub(super) fn location(finding: &Finding) -> Option<String> {
    match (&finding.file, &finding.input) {
        (Some(file), _) => {
            let name = Path::new(file)
                .file_name()
                .map_or_else(|| file.clone(), |name| name.to_string_lossy().into_owned());
            Some(match finding.line {
                Some(line) => clean(&format!("{name}:{line}")),
                None => clean(&name),
            })
        }
        (None, Some(input)) => Some(clean(input)),
        (None, None) => None,
    }
}

/// `{location} {severity} — {text}`, as `fixing` and `findings` show a finding.
pub(super) fn finding_text(finding: &Finding) -> String {
    let rest = format!(
        "{} — {}",
        severity_text(finding.severity),
        clean(&finding.text)
    );
    match location(finding) {
        Some(place) => format!("{place} {rest}"),
        None => rest,
    }
}
