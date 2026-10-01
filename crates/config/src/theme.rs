//! The `[theme]` table (milestone 9.0.6 decision 2): one key, `truecolor`, that says
//! whether the client may draw in 24-bit colour. `auto` (the default) leaves the
//! decision to `COLORTERM`, which only the CLI reads (`tui::theme::truecolor`).
//! Parsing follows milestone 6 decision 5: a bad value is a [`Problem`] and the field
//! keeps its default; an unknown key is `unknown key, ignored`.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Truecolor {
    #[default]
    Auto,
    On,
    Off,
}

impl Truecolor {
    fn name(self) -> &'static str {
        match self {
            Truecolor::Auto => "auto",
            Truecolor::On => "on",
            Truecolor::Off => "off",
        }
    }
}

/// The `[theme]` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Theme {
    pub truecolor: Truecolor,
}

pub(crate) const KNOWN_THEME_KEYS: &[&str] = &["truecolor"];

pub(crate) fn read_theme(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("theme") else {
        return;
    };
    let Some(theme) = value.as_table() else {
        problems.push(not_a_table_problem("theme"));
        return;
    };
    let Some(value) = theme.get("truecolor") else {
        return;
    };
    config.theme.truecolor = match value.as_str() {
        Some("auto") => Truecolor::Auto,
        Some("on") => Truecolor::On,
        Some("off") => Truecolor::Off,
        _ => {
            problems.push(Problem {
                key: "theme.truecolor".to_string(),
                message: r#"must be "auto", "on" or "off""#.to_string(),
                default: config.theme.truecolor.name().to_string(),
            });
            return;
        }
    };
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
