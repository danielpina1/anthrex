//! The client's view of `config::Config` (task M6.9, decision 38): every `config`
//! value the TUI needs, already resolved to the concrete type the renderers and `App`
//! use — `Ui::sidebar_width`'s `None` turned into the built-in width, the prefix
//! character turned into the `(KeyCode, KeyModifiers)` pair `Keymap::new` takes, and so
//! on. Built once by the CLI's `attach` and passed into `App::new`; nothing under
//! `crates/tui/src/app/` or `crates/tui/src/ui/` reads `config` directly (`AGENTS.md`
//! hard rule 5 and this milestone's layering rule: the client stays pure, and loading
//! the config is I/O).

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::Color;

#[derive(Debug, Clone, PartialEq)]
pub struct UiSettings {
    pub prefix: (KeyCode, KeyModifiers),
    pub prefix_label: String,
    pub accent: Color,
    pub bell_attention: bool,
    pub bell_done: bool,
    pub default_runtime: proto::Runtime,
    pub scrollback_lines: usize,
    pub sidebar_width: u16,
    pub tree_keep_finished_secs: u64,
    pub panes_max: u8,
}

impl Default for UiSettings {
    /// Matches `config::Config::default()` field for field (task M6.9's test
    /// `from_config_maps_every_field`), so a client run with no config file at all
    /// behaves exactly like one built from `UiSettings::from_config`.
    fn default() -> Self {
        Self::from_config(&config::Config::default())
    }
}

impl UiSettings {
    pub fn from_config(c: &config::Config) -> Self {
        Self {
            prefix: (KeyCode::Char(c.prefix.0), KeyModifiers::CONTROL),
            prefix_label: c.prefix.label(),
            accent: Color::Rgb(c.accent.0, c.accent.1, c.accent.2),
            bell_attention: c.bell.attention,
            bell_done: c.bell.done,
            default_runtime: c.default_runtime,
            scrollback_lines: c.scrollback_lines,
            // A `None` sidebar width (nothing set in the config) falls back to the
            // client's own built-in width, never zero.
            sidebar_width: c
                .ui
                .sidebar_width
                .unwrap_or(crate::ui::DEFAULT_SIDEBAR_WIDTH),
            tree_keep_finished_secs: c.ui.tree_keep_finished_secs,
            panes_max: c.panes.max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn from_config_maps_every_field() {
        let toml = r##"
            prefix = "C-a"
            accent = "#f5c2e7"
            default_runtime = "codex"
            scrollback_lines = 1234

            [bell]
            attention = false
            done = true

            [ui]
            sidebar_width = 40
            tree_keep_finished_secs = 60

            [panes]
            max = 3
        "##;
        let (config, problems) = config::parse(toml);
        assert!(problems.is_empty(), "{problems:?}");

        let settings = UiSettings::from_config(&config);
        assert_eq!(settings.prefix, (KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(settings.prefix_label, "C-a");
        assert_eq!(settings.accent, Color::Rgb(0xf5, 0xc2, 0xe7));
        assert!(!settings.bell_attention);
        assert!(settings.bell_done);
        assert_eq!(settings.default_runtime, proto::Runtime::Codex);
        assert_eq!(settings.scrollback_lines, 1234);
        assert_eq!(settings.sidebar_width, 40);
        assert_eq!(settings.tree_keep_finished_secs, 60);
        assert_eq!(settings.panes_max, 3);
    }

    #[test]
    fn defaults_match_the_config_crates_own_defaults() {
        assert_eq!(
            UiSettings::from_config(&config::Config::default()),
            UiSettings::default()
        );
    }

    #[test]
    fn an_unset_sidebar_width_falls_back_to_the_built_in_default() {
        let settings = UiSettings::from_config(&config::Config::default());
        assert_eq!(settings.sidebar_width, crate::ui::DEFAULT_SIDEBAR_WIDTH);
    }
}
