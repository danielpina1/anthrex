//! Colours and glyphs. Spec section 6.5: inherit the terminal background, one accent, unicode-only glyphs.

use proto::{Status, SubagentInfo, SubagentState};
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
