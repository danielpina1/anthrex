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

/// Milestone 9 decision 42i: a `paused(message)` task's own colour, beside `‖`.
pub const PAUSED_COLOR: Color = Color::Rgb(0xcb, 0xa6, 0xf7);

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

/// Milestone 9 (decisions 28 and 42i): a task's glyph and colour, `task_glyph` and
/// `task_color` with two marks of its own. A task held for approval is drawn as
/// planned (`○`, the idle colour); a `paused(message)` task is `‖` in
/// [`PAUSED_COLOR`].
pub fn task_look(
    state: TaskState,
    gate_open: bool,
    held: bool,
    paused: bool,
    animating: bool,
    spinner_frame: usize,
) -> (&'static str, Color) {
    if gate_open || held {
        return ("○", status_color(Status::Idle));
    }
    if paused {
        return ("‖", PAUSED_COLOR);
    }
    (
        task_glyph(state, false, animating, spinner_frame),
        task_color(state),
    )
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

/// Milestone 9.0.5 decision 19: an alert's `●` and label, by priority: 1 red, 2 yellow,
/// 3 magenta, 4 green.
pub fn alert_color(priority: u8) -> Color {
    match priority {
        1 => Color::Red,
        2 => Color::Yellow,
        3 => Color::Magenta,
        _ => Color::Green,
    }
}

/// Milestone 9.0.6 decision 1: the seven colour roles every new widget draws in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Accent,
    Attention,
    Working,
    Done,
    Failed,
    Muted,
    Paused,
}

/// What `role` needs from the settings: the configured accent and whether 24-bit
/// colour may be used (built by `App::palette`).
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub accent: Color,
    pub truecolor: bool,
    /// ASCII-only glyphs (`UiSettings.badges.ascii`): every kit widget honours it.
    pub ascii: bool,
}

/// A role's style. With truecolor off every role is one of the 16 ANSI colours,
/// whatever the configured accent; `Working` is never `Attention` (principle 1).
pub fn role(r: Role, p: Palette) -> Style {
    let (ansi, rgb) = match r {
        Role::Accent => (Color::LightBlue, p.accent),
        Role::Attention => (Color::LightMagenta, Color::Rgb(0xfa, 0xb3, 0x87)),
        Role::Working => (Color::Yellow, Color::Rgb(0xf9, 0xe2, 0xaf)),
        Role::Done => (Color::Green, Color::Rgb(0xa6, 0xe3, 0xa1)),
        Role::Failed => (Color::Red, Color::Rgb(0xf3, 0x8b, 0xa8)),
        Role::Muted => (Color::DarkGray, Color::Rgb(0x6c, 0x70, 0x86)),
        Role::Paused => (Color::Cyan, PAUSED_COLOR),
    };
    let style = Style::default().fg(if p.truecolor { rgb } else { ansi });
    if r == Role::Attention {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// Decision 2: `on` forces truecolor, `off` forbids it, `auto` asks `COLORTERM`
/// (`truecolor` or `24bit`, ASCII case-insensitive). The CLI reads the environment;
/// this stays pure.
pub fn truecolor(setting: config::Truecolor, colorterm: Option<&str>) -> bool {
    match setting {
        config::Truecolor::On => true,
        config::Truecolor::Off => false,
        config::Truecolor::Auto => colorterm.is_some_and(|v| {
            v.eq_ignore_ascii_case("truecolor") || v.eq_ignore_ascii_case("24bit")
        }),
    }
}

/// Decision 3: the nine marks of §5.1 principle 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Passed,
    Failed,
    NotStarted,
    Idle,
    NeedsYou,
    Collapsed,
    Selection,
    Separator,
    Warning,
}

/// A glyph, or its ASCII twin when `ascii` (`UiSettings.badges.ascii`).
pub fn glyph(g: Glyph, ascii: bool) -> &'static str {
    let (unicode, plain) = match g {
        Glyph::Passed => ("✓", "+"),
        Glyph::Failed => ("✗", "x"),
        Glyph::NotStarted => ("◌", "."),
        Glyph::Idle => ("○", "o"),
        Glyph::NeedsYou => ("⚑", "!"),
        Glyph::Collapsed => ("▸", ">"),
        Glyph::Selection => ("▌", ">"),
        Glyph::Separator => ("›", ">"),
        Glyph::Warning => ("⚠", "!"),
    };
    if ascii { plain } else { unicode }
}

/// The spinner frame: [`SPINNER`], or `- \ | /` in ASCII.
pub fn spinner(frame: usize, ascii: bool) -> &'static str {
    const ASCII: [&str; 4] = ["-", "\\", "|", "/"];
    if ascii {
        ASCII[frame % ASCII.len()]
    } else {
        SPINNER[frame % SPINNER.len()]
    }
}

/// New screens draw runtimes as text tags, not logos.
pub fn runtime_tag(r: proto::Runtime) -> &'static str {
    match r {
        proto::Runtime::Claude => "cl",
        proto::Runtime::Codex => "cx",
        proto::Runtime::Shell => "sh",
    }
}
