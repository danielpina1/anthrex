//! Colours and glyphs. Spec §5.1: inherit the terminal background, colours are roles
//! (`role`), every glyph has an ASCII twin (`glyph`), and every look of a window, task,
//! run, sub-agent, stage or alert is a glyph and a role (milestone 9.0.7 decision 3).

use ratatui::style::{Color, Modifier, Style};

/// The built-in accent, used until `config.toml`'s `accent` (decision 4) says
/// otherwise. Renderers never use this constant directly (decision 38): they draw
/// the `Accent` role through `role(Role::Accent, app.palette())`, which is
/// `app.settings.accent` under truecolor and ANSI light blue otherwise (milestone
/// 9.0.6 decision 1). It survives as `UiSettings::default`'s source of truth and
/// as the palette of tests that draw without an `App`.
pub const DEFAULT_ACCENT: Color = Color::Rgb(0x89, 0xb4, 0xfa);
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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

impl Palette {
    /// The default accent, no truecolor, unicode: the palette of a draw without an
    /// `App` (the panel's and the old dialogs' own tests).
    pub const PLAIN: Palette = Palette {
        accent: DEFAULT_ACCENT,
        truecolor: false,
        ascii: false,
    };
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
        Role::Paused => (Color::Cyan, Color::Rgb(0xcb, 0xa6, 0xf7)),
    };
    let style = Style::default().fg(if p.truecolor { rgb } else { ansi });
    if r == Role::Attention {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// A role's foreground in [`Palette::PLAIN`]: what tests that draw with the default
/// settings expect a look to be drawn in.
#[cfg(test)]
pub(crate) fn fg(r: Role) -> Color {
    role(r, Palette::PLAIN)
        .fg
        .expect("every role has a foreground")
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

/// Decision 3: the nine marks of §5.1 principle 4, and milestone 9.0.7 decision 6's twelve.
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
    // Milestone 9.0.7 decision 6: twins need not be unique.
    Expanded,
    Live,
    Checking,
    Review,
    Queued,
    Merging,
    Blocked,
    Ended,
    Paused,
    Run,
    Hub,
    Focus,
}

/// A glyph, or its ASCII twin when `ascii` (`UiSettings.badges.ascii`).
pub const fn glyph(g: Glyph, ascii: bool) -> &'static str {
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
        Glyph::Expanded => ("▾", "v"),
        Glyph::Live => ("●", "*"),
        Glyph::Checking => ("◇", "~"),
        Glyph::Review => ("◐", "%"),
        Glyph::Queued => ("▫", ":"),
        Glyph::Merging => ("»", "="),
        Glyph::Blocked => ("⊘", "#"),
        Glyph::Ended => ("–", "_"),
        Glyph::Paused => ("‖", "\""),
        Glyph::Run => ("◉", "@"),
        Glyph::Hub => ("◆", "H"),
        Glyph::Focus => ("▎", "|"),
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

/// Milestone 9.0.7 decision 2: the one ASCII border every frame draws in ASCII mode,
/// the pane frame, the dialog frame and the screen frame alike.
pub const ASCII_BORDER: ratatui::symbols::border::Set<'static> = ratatui::symbols::border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// A pane's border: rounded, or [`ASCII_BORDER`] when `ascii` (decision 2).
pub fn border_set(ascii: bool) -> ratatui::symbols::border::Set<'static> {
    if ascii {
        ASCII_BORDER
    } else {
        ratatui::symbols::border::ROUNDED
    }
}

/// `…`, or `...` in ASCII: decision 5's fold of the client's ellipsis, as a
/// `&'static str` for the kit's cuts (final fix wave M5: one helper for the screens'
/// six copies).
pub fn ellipsis(p: Palette) -> &'static str {
    if p.ascii { "..." } else { "…" }
}

/// `·`, or `-` in ASCII (decision 5).
pub fn dot(p: Palette) -> &'static str {
    if p.ascii { "-" } else { "·" }
}

/// A choice's value, `‹ value ›`, or `< value >` in ASCII, sanitised (milestone 9.0.6
/// decision 5). Pure text, here so the model (`run_edit.rs`) and the kit both read it
/// without the model depending on the UI kit (final fix wave, task 13a's m7).
pub fn choice(value: &str, p: Palette) -> String {
    let value = crate::safe_text::one_line(value);
    if p.ascii {
        format!("< {value} >")
    } else {
        format!("‹ {value} ›")
    }
}

/// ` · `, or ` - ` in ASCII: the hint lines' separator.
pub fn dot_sep(p: Palette) -> &'static str {
    if p.ascii { " - " } else { " · " }
}

/// New screens draw runtimes as text tags, not logos.
pub fn runtime_tag(r: proto::Runtime) -> &'static str {
    match r {
        proto::Runtime::Claude => "cl",
        proto::Runtime::Codex => "cx",
        proto::Runtime::Shell => "sh",
    }
}

mod looks;
pub use looks::*;

#[cfg(test)]
#[path = "theme_looks_tests.rs"]
mod looks_tests;
