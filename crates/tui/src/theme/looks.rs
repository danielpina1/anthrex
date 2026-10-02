//! Milestone 9.0.7 decisions 3–5: every look is a glyph and a role, never an RGB
//! value, and every glyph, border, guide, bar and punctuation mark has its ASCII twin.
//! Callers style a look's role with `theme::role(r, app.palette())`.

use super::{Glyph, Palette, Role, glyph, role, spinner};
use proto::{FullState, RunState, StageInfo, Status, SubagentInfo, SubagentState, TaskState};
use ratatui::style::Style;

/// Decision 3: a window's glyph and role. `frame` drives the spinner.
pub fn status_look(status: Status, frame: usize, ascii: bool) -> (&'static str, Role) {
    let g = |mark| glyph(mark, ascii);
    match status {
        Status::Starting => (g(Glyph::NotStarted), Role::Muted),
        Status::Idle => (g(Glyph::Idle), Role::Muted),
        Status::Exited => (g(Glyph::Ended), Role::Muted),
        Status::Working => (spinner(frame, ascii), Role::Working),
        Status::Attention => (g(Glyph::NeedsYou), Role::Attention),
        Status::Done => (g(Glyph::Passed), Role::Done),
    }
}

/// What a task's look depends on. `gate_open`: its run awaits plan approval; `held`:
/// a hold awaits approval for it; `paused`: a `paused(message)` task; `animating`:
/// its live worker round's window is `Working` (milestone 8c decision 19);
/// `needs_you`: `app::alerts::task_needs_you` (decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskLook {
    pub state: TaskState,
    pub gate_open: bool,
    pub held: bool,
    pub paused: bool,
    pub animating: bool,
    pub needs_you: bool,
}

/// Decision 3: a task's glyph and role. A task at the gate or held is drawn as
/// planned (`○`); a paused one `‖`; a blocked one `⚑` only when it needs the user.
pub fn task_look(t: TaskLook, frame: usize, ascii: bool) -> (&'static str, Role) {
    let g = |mark| glyph(mark, ascii);
    if t.gate_open || t.held {
        return (g(Glyph::Idle), Role::Muted);
    }
    if t.paused {
        return (g(Glyph::Paused), Role::Paused);
    }
    match t.state {
        TaskState::Pending => (g(Glyph::NotStarted), Role::Muted),
        TaskState::Queued => (g(Glyph::Queued), Role::Muted),
        TaskState::Working if t.animating => (spinner(frame, ascii), Role::Working),
        TaskState::Preparing | TaskState::Working => (g(Glyph::Live), Role::Working),
        TaskState::Proof | TaskState::Check => (g(Glyph::Checking), Role::Working),
        TaskState::Review => (g(Glyph::Review), Role::Working),
        TaskState::MergeQueue => (g(Glyph::Merging), Role::Working),
        TaskState::Merged | TaskState::Reported => (g(Glyph::Passed), Role::Done),
        TaskState::Blocked if t.needs_you => (g(Glyph::NeedsYou), Role::Attention),
        TaskState::Blocked => (g(Glyph::Blocked), Role::Paused),
        TaskState::Cancelled => (g(Glyph::Ended), Role::Muted),
    }
}

/// Decision 3: a run's glyph and role.
pub fn run_look(state: RunState, ascii: bool) -> (&'static str, Role) {
    let g = |mark| glyph(mark, ascii);
    match state {
        RunState::AwaitingApproval | RunState::Halted => (g(Glyph::NeedsYou), Role::Attention),
        RunState::Planning | RunState::Running => (g(Glyph::Run), Role::Working),
        RunState::Paused => (g(Glyph::Run), Role::Paused),
        RunState::Complete | RunState::Accepted => (g(Glyph::Passed), Role::Done),
        RunState::Discarded => (g(Glyph::Ended), Role::Muted),
        RunState::Failed => (g(Glyph::Failed), Role::Failed),
    }
}

/// Decision 3: a sub-agent's glyph and role; one asking for permission needs you.
pub fn subagent_look(info: &SubagentInfo, frame: usize, ascii: bool) -> (&'static str, Role) {
    let g = |mark| glyph(mark, ascii);
    if info.needs_permission {
        return (g(Glyph::NeedsYou), Role::Attention);
    }
    match info.state {
        SubagentState::Running => (spinner(frame, ascii), Role::Working),
        SubagentState::Done => (g(Glyph::Passed), Role::Done),
        SubagentState::Failed => (g(Glyph::Failed), Role::Failed),
    }
}

/// Decision 3: a delivery stage's tier 3: `◌` before its branch exists or its first
/// run, the spinner while running, `✓` green, `✗` red or bisecting (never a spinner).
pub fn stage_look(stage: &StageInfo, frame: usize, ascii: bool) -> (&'static str, Role) {
    let g = |mark| glyph(mark, ascii);
    match stage.full.state {
        _ if stage.head.is_none() => (g(Glyph::NotStarted), Role::Muted),
        FullState::None => (g(Glyph::NotStarted), Role::Muted),
        FullState::Running => (spinner(frame, ascii), Role::Working),
        FullState::Green => (g(Glyph::Passed), Role::Done),
        FullState::Red | FullState::Bisecting => (g(Glyph::Failed), Role::Failed),
    }
}

/// P1 `Attention` bold, P2 and P3 `Attention` without bold, P4 `Done` (decision 3).
pub fn alert_style(priority: u8, p: Palette) -> Style {
    match priority {
        1 => role(Role::Attention, p),
        2 | 3 => Style {
            fg: role(Role::Attention, p).fg,
            ..Style::default()
        },
        _ => role(Role::Done, p),
    }
}

/// An alert's mark: `⚑` for priorities 1 to 3, `✓` for 4 (decision 3).
pub fn alert_glyph(priority: u8, ascii: bool) -> &'static str {
    match priority {
        1..=3 => glyph(Glyph::NeedsYou, ascii),
        _ => glyph(Glyph::Passed, ascii),
    }
}

/// Decision 5: the tree guides `│ `, `├─`, `└─` as `| `, `|-`, `` `- `` in ASCII; the
/// tree builder keeps its own glyphs and the renderers map them at draw time.
pub fn guides(text: &str, ascii: bool) -> String {
    if !ascii {
        return text.to_owned();
    }
    text.chars()
        .map(|c| match c {
            '│' | '├' => '|',
            '└' => '`',
            '─' => '-',
            other => other,
        })
        .collect()
}

/// Decision 5: a progress bar of `filled` and `empty` cells, `█░` or `#.`.
pub fn bar(filled: usize, empty: usize, ascii: bool) -> String {
    let (full, none) = if ascii { ("#", ".") } else { ("█", "░") };
    format!("{}{}", full.repeat(filled), none.repeat(empty))
}

/// Every glyph of the table (`Glyph`), the spinner's frames and the git marks as
/// their one-column ASCII twins: for text composed from many glyph literals at once
/// (an inspection), folded in one place rather than at each literal.
pub fn ascii_twins(text: &str) -> String {
    const GLYPHS: [Glyph; 21] = [
        Glyph::Passed,
        Glyph::Failed,
        Glyph::NotStarted,
        Glyph::Idle,
        Glyph::NeedsYou,
        Glyph::Collapsed,
        Glyph::Selection,
        Glyph::Separator,
        Glyph::Warning,
        Glyph::Expanded,
        Glyph::Live,
        Glyph::Checking,
        Glyph::Review,
        Glyph::Queued,
        Glyph::Merging,
        Glyph::Blocked,
        Glyph::Ended,
        Glyph::Paused,
        Glyph::Run,
        Glyph::Hub,
        Glyph::Focus,
    ];
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let mut buf = [0; 4];
        let as_str: &str = c.encode_utf8(&mut buf);
        let twin = GLYPHS
            .iter()
            .find(|g| glyph(**g, false) == as_str)
            .map(|g| glyph(*g, true))
            .or_else(|| super::SPINNER.contains(&as_str).then_some("-"))
            .or(match c {
                '✕' => Some("x"),
                '⇡' => Some("^"),
                '⇣' => Some("v"),
                _ => None,
            });
        match twin {
            Some(twin) => out.push_str(twin),
            None => out.push(c),
        }
    }
    out
}

/// Decision 5: the client's own punctuation in ASCII; the identity when `!ascii`.
pub fn fold(text: &str, ascii: bool) -> String {
    if !ascii {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '·' | '−' | '—' => out.push('-'),
            '…' => out.push_str("..."),
            '→' => out.push_str("->"),
            '←' => out.push_str("<-"),
            '›' => out.push('>'),
            '‹' => out.push('<'),
            '↑' => out.push('^'),
            '↓' => out.push('v'),
            '⏎' => out.push_str("enter"),
            '☐' => out.push_str("[ ]"),
            other => out.push(other),
        }
    }
    out
}
