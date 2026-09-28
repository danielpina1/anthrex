//! Colours and glyphs. Spec section 6.5: inherit the terminal background, one accent, unicode-only glyphs.

use proto::{RunState, Status, SubagentInfo, SubagentState, TaskState};
use ratatui::style::{Color, Modifier, Style};

/// The built-in accent, used until `config.toml`'s `accent` (decision 4) says
/// otherwise. Every renderer takes its accent from `app.settings.accent`
/// instead of this constant directly (decision 38); it survives only as
/// `UiSettings::default`'s source of truth and this module's own tests.
pub const DEFAULT_ACCENT: Color = Color::Rgb(0x89, 0xb4, 0xfa);
pub const DIM: Color = Color::DarkGray;
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn status_color(status: Status) -> Color {
    match status {
        Status::Starting => Color::Gray,
        Status::Working => Color::Yellow,
        Status::Idle => Color::Gray,
        Status::Attention => Color::Rgb(0xfa, 0xb3, 0x87),
        Status::Done => Color::Green,
        Status::Exited => Color::DarkGray,
    }
}

pub fn status_glyph(status: Status, spinner_frame: usize) -> &'static str {
    match status {
        Status::Starting => "◌",
        Status::Working => SPINNER[spinner_frame % SPINNER.len()],
        Status::Idle => "○",
        Status::Attention => "◆",
        Status::Done => "✓",
        Status::Exited => "✕",
    }
}

/// Milestone 8c: a run's node glyph, drawn in `run_color`.
pub const RUN_GLYPH: &str = "◉";

/// Milestone 8c: a run's colour by its state.
pub fn run_color(state: RunState) -> Color {
    match state {
        RunState::AwaitingApproval | RunState::Paused | RunState::Halted => {
            status_color(Status::Attention)
        }
        RunState::Running | RunState::Planning => status_color(Status::Working),
        RunState::Complete | RunState::Accepted => status_color(Status::Done),
        RunState::Discarded | RunState::Failed => DIM,
    }
}

/// Milestone 8c: a task's glyph (Interfaces "Glyphs"). At the plan gate every task
/// is drawn as planned, `○`; `working` spins only while `animating`, which the caller
/// sets while the task's live worker round's window is `Working` (decision 19).
pub fn task_glyph(
    state: TaskState,
    gate_open: bool,
    animating: bool,
    spinner_frame: usize,
) -> &'static str {
    if gate_open {
        return "○";
    }
    match state {
        TaskState::Pending => "◌",
        TaskState::Queued => "▫",
        TaskState::Preparing => "●",
        TaskState::Working if animating => status_glyph(Status::Working, spinner_frame),
        TaskState::Working => "●",
        TaskState::Proof | TaskState::Check => "◇",
        TaskState::Review => "◐",
        TaskState::MergeQueue => "▸",
        TaskState::Merged | TaskState::Reported => "✓",
        TaskState::Blocked => "⊘",
        TaskState::Cancelled => "–",
    }
}

/// Milestone 8c: a task's glyph colour by its state; the gate's `○` is
/// `status_color(Idle)`, which the caller picks.
pub fn task_color(state: TaskState) -> Color {
    match state {
        TaskState::Pending => status_color(Status::Starting),
        TaskState::Queued => status_color(Status::Idle),
        TaskState::Preparing
        | TaskState::Working
        | TaskState::Proof
        | TaskState::Check
        | TaskState::Review
        | TaskState::MergeQueue => status_color(Status::Working),
        TaskState::Merged | TaskState::Reported => status_color(Status::Done),
        TaskState::Blocked => status_color(Status::Attention),
        TaskState::Cancelled => DIM,
    }
}

pub fn border() -> Style {
    Style::default().fg(DIM)
}

pub fn subagent_glyph(info: &SubagentInfo, spinner_frame: usize) -> &'static str {
    if info.needs_permission {
        return status_glyph(Status::Attention, spinner_frame);
    }
    match info.state {
        SubagentState::Running => status_glyph(Status::Working, spinner_frame),
        SubagentState::Done => "✓",
        SubagentState::Failed => "✕",
    }
}

pub fn subagent_color(info: &SubagentInfo) -> Color {
    if info.needs_permission {
        return status_color(Status::Attention);
    }
    match info.state {
        SubagentState::Running => status_color(Status::Working),
        SubagentState::Done => status_color(Status::Done),
        SubagentState::Failed => Color::Red,
    }
}

pub fn border_focused(accent: Color) -> Style {
    Style::default().fg(accent)
}

pub fn title(accent: Color) -> Style {
    Style::default().fg(accent).add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::default().fg(DIM)
}
