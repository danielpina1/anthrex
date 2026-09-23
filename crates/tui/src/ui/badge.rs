//! Runtime badges (spec decision 10, amended by decision A5): a glyph plus a colour for
//! each of the three runtimes, with an ASCII fallback for a terminal that either says so
//! (`conversation.badges.force_ascii`) or whose locale is explicitly not UTF-8. Colour is
//! never the only signal — the glyph itself always distinguishes the runtimes, so a
//! monochrome terminal still tells Claude, Codex and Shell apart.
//!
//! This module performs no I/O: `prefers_ascii` takes the locale as a plain `Option<&str>`
//! rather than reading the environment itself, per `AGENTS.md` hard rule 5 and task
//! M6.5.11's own acceptance criterion. `crates/cli/src/main.rs` is the only place the
//! environment is read.

use ratatui::style::Color;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

/// Every badge occupies exactly this many columns, unicode or ASCII, so nothing
/// downstream shifts when the two forms differ in width.
pub const BADGE_WIDTH: u16 = 3;

/// One runtime's badge: the text to draw (already chosen as glyph or ASCII by
/// [`BadgeSet::from_config`]) and its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Badge {
    pub text: String,
    pub color: Color,
}

/// The three runtime badges, resolved from config for one of the two rendering modes
/// (unicode or ASCII). Built once by `UiSettings::from_config` / `with_locale`.
#[derive(Debug, Clone, PartialEq)]
pub struct BadgeSet {
    pub claude: Badge,
    pub codex: Badge,
    pub shell: Badge,
    /// Whether these are the ASCII forms. The conversation view draws its own glyphs
    /// (`▸ ▾ ⟐ ⚠ ⋯ › ✓ ✕ ⊘`) in ASCII too when this is set (task M6.5.13), so one
    /// decision (A5) switches the whole view, never half of it.
    pub ascii: bool,
}

fn to_badge(badge: &config::Badge, ascii: bool) -> Badge {
    Badge {
        text: if ascii {
            badge.ascii.clone()
        } else {
            badge.glyph.clone()
        },
        color: Color::Rgb(badge.color.0, badge.color.1, badge.color.2),
    }
}

impl BadgeSet {
    pub fn from_config(badges: &config::Badges, ascii: bool) -> Self {
        BadgeSet {
            claude: to_badge(&badges.claude, ascii),
            codex: to_badge(&badges.codex, ascii),
            shell: to_badge(&badges.shell, ascii),
            ascii,
        }
    }

    /// The badge for `runtime`. Total: every `proto::Runtime` variant has a badge
    /// (mirrors `config::Badges::for_runtime`).
    pub fn for_runtime(&self, runtime: proto::Runtime) -> &Badge {
        match runtime {
            proto::Runtime::Claude => &self.claude,
            proto::Runtime::Codex => &self.codex,
            proto::Runtime::Shell => &self.shell,
        }
    }
}

/// Decision A5: ASCII is used when `force_ascii` says so, or when the first set and
/// non-empty value of `LC_ALL`, `LC_CTYPE`, `LANG` does not contain `utf-8` or `utf8`
/// case-insensitively. An absent locale (`None`) means unicode, because the rest of the
/// TUI already draws `◌ ⠋ ✓ ✕` unconditionally (`crates/tui/src/theme.rs`) — switching
/// only the badges to ASCII on no signal at all would produce a half-ASCII screen, which
/// is worse than either whole. Pure: the caller reads the environment.
pub fn prefers_ascii(force_ascii: bool, locale: Option<&str>) -> bool {
    if force_ascii {
        return true;
    }
    let Some(locale) = locale else {
        return false;
    };
    if locale.is_empty() {
        return true;
    }
    let lower = locale.to_lowercase();
    !(lower.contains("utf-8") || lower.contains("utf8"))
}

/// The badge padded to [`BADGE_WIDTH`] columns, so a unicode and an ASCII badge occupy
/// the same slot and nothing downstream shifts.
pub fn span(badge: &Badge) -> Span<'static> {
    let width = UnicodeWidthStr::width(badge.text.as_str()) as u16;
    let pad = BADGE_WIDTH.saturating_sub(width) as usize;
    let mut content = badge.text.clone();
    content.push_str(&" ".repeat(pad));
    Span::styled(content, ratatui::style::Style::default().fg(badge.color))
}

#[cfg(test)]
#[path = "badge_tests.rs"]
mod tests;
