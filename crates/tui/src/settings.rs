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
    pub badges: crate::ui::badge::BadgeSet,
    // Kept so `with_locale` can rebuild `badges` once the CLI has read the environment
    // (decision A5): `from_config` alone only has `force_ascii`, never the locale.
    // `pub(crate)` rather than private so the struct-update syntax (`..UiSettings::default()`)
    // used throughout `crates/tui/src/app_tests/` and `crates/tui/src/ui/tests.rs` still
    // compiles from outside this module.
    pub(crate) badges_config: config::Badges,
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
            // Whole-branch-review Minor: `config::Panes::max` deliberately has no
            // field here to receive it — M7 (split panes, deferred) is what will
            // actually read it. A `panes_max` field threaded this far and read
            // nowhere is dead code by this milestone's own hard rule 8, not
            // forward-compatibility; add it back alongside its first real reader.
            tree_keep_finished_secs: c.ui.tree_keep_finished_secs,
            badges: crate::ui::badge::BadgeSet::from_config(
                &c.conversation.badges,
                c.conversation.badges.force_ascii,
            ),
            badges_config: c.conversation.badges.clone(),
        }
    }

    /// Applies decision A5's locale rule on top of `from_config`'s `force_ascii`-only
    /// choice: `conversation.badges.force_ascii` still wins outright, but a set,
    /// non-UTF-8 locale now also switches to the ASCII badges. Tests do not have to call
    /// this — only `crates/cli/src/main.rs`'s `attach`, which is where the environment is
    /// read (`AGENTS.md` hard rule 5).
    pub fn with_locale(mut self, locale: Option<&str>) -> Self {
        let ascii = crate::ui::badge::prefers_ascii(self.badges_config.force_ascii, locale);
        self.badges = crate::ui::badge::BadgeSet::from_config(&self.badges_config, ascii);
        self
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
    }

    /// Whole-branch-review m17: this used to be `assert_eq!(f(x), f(x))` —
    /// `UiSettings::default()` *is* `Self::from_config(&config::Config::default())`
    /// (this file's own `impl Default`), so the old assertion passed for every
    /// possible body `from_config` could have, including a wrong one. What decision
    /// 4's table actually promises is that the client's built-in defaults are these
    /// specific values; pin those instead.
    #[test]
    fn defaults_match_decision_4s_table() {
        let settings = UiSettings::from_config(&config::Config::default());
        assert_eq!(settings.prefix, (KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert_eq!(settings.prefix_label, "C-b");
        assert_eq!(settings.accent, Color::Rgb(0x89, 0xb4, 0xfa));
        assert!(settings.bell_attention);
        assert!(!settings.bell_done);
        assert_eq!(settings.default_runtime, proto::Runtime::Shell);
        assert_eq!(settings.scrollback_lines, 5000);
        assert_eq!(settings.sidebar_width, crate::ui::DEFAULT_SIDEBAR_WIDTH);
        assert_eq!(settings.tree_keep_finished_secs, 300);
        assert_eq!(settings.badges.claude.text, "\u{25c6}");
        assert_eq!(settings.badges.codex.text, "\u{25c7}");
        assert_eq!(settings.badges.shell.text, "$");
        assert_eq!(settings.badges.claude.color, Color::Rgb(0xd7, 0x9b, 0x61));
        assert_eq!(settings.badges.codex.color, Color::Rgb(0x7f, 0xc8, 0xb4));
        assert_eq!(settings.badges.shell.color, Color::Rgb(0x9a, 0xa0, 0xb5));

        // The property the old test's name actually promised, kept as a second,
        // narrower assertion rather than dropped: `UiSettings::default()` must still
        // agree with `from_config(&Config::default())`, now proven against a value
        // this test has already pinned field-by-field, not against itself.
        assert_eq!(settings, UiSettings::default());
    }

    #[test]
    fn an_unset_sidebar_width_falls_back_to_the_built_in_default() {
        let settings = UiSettings::from_config(&config::Config::default());
        assert_eq!(settings.sidebar_width, crate::ui::DEFAULT_SIDEBAR_WIDTH);
    }

    #[test]
    fn with_locale_overrides_the_glyphs() {
        let ascii = UiSettings::from_config(&config::Config::default()).with_locale(Some("C"));
        assert_eq!(ascii.badges.claude.text, "[C]");
        assert_eq!(ascii.badges.codex.text, "[X]");
        assert_eq!(ascii.badges.shell.text, "[$]");

        let unicode =
            UiSettings::from_config(&config::Config::default()).with_locale(Some("en_US.UTF-8"));
        assert_eq!(unicode.badges.claude.text, "\u{25c6}");
        assert_eq!(unicode.badges.codex.text, "\u{25c7}");
        assert_eq!(unicode.badges.shell.text, "$");

        let absent = UiSettings::from_config(&config::Config::default()).with_locale(None);
        assert_eq!(absent.badges.claude.text, "\u{25c6}");
        assert_eq!(absent.badges.codex.text, "\u{25c7}");
        assert_eq!(absent.badges.shell.text, "$");
    }
}
