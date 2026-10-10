//! Milestone 9.0.6 decision 35: the Profile screen's view of a `RepoProfile`, pure.
//! Milestone 9.10 decision 24's sections, one row per key in their order with its value,
//! its verification record and, against the stored profile, its mark and old value;
//! the check cell; each key's editor kind; and the TOML literal an edit sends, which
//! `apply_edit`'s `parse_value` reads back exactly (`daemon/src/profile/proposal.rs:193`).
//! No I/O: values are read through `toml::Value`, the same table the daemon edits.

use crate::theme::{Glyph, glyph};
use proto::{CommandCheck, ProfileVerification, RepoProfile};
use std::collections::BTreeSet;

/// Decision 24: the sections of the one-page Profile screen, in order: the title,
/// whether it sits inside Advanced, and its keys (`env` is the environment section's
/// add row; each `env.<NAME>` row joins it).
pub const SECTIONS: [(&str, bool, &[&str]); 11] = [
    (
        "How anthrex checks your work",
        false,
        &["setup", "check", "single_test"],
    ),
    (
        "Your repo",
        false,
        &["source", "test_paths", "generated", "protected"],
    ),
    ("Delivery", false, &["delivery.mode"]),
    (
        "testing tiers",
        true,
        &[
            "build_check",
            "module_test",
            "module_tests",
            "module_graph",
            "module_names",
            "full_triggers",
            "slow_tests",
            "timing_tests",
            "skip_markers",
            "full_shards",
            "toolchain_id",
        ],
    ),
    ("output filter", true, &["output_filter", "filter_prefixes"]),
    ("environment", true, &["env"]),
    ("timeouts", true, &["check_timeout_secs"]),
    ("test result pattern", true, &["test_passed", "sample_test"]),
    ("shared code area", true, &["hub"]),
    (
        "repo details",
        true,
        &["languages", "modules", "manifests", "conventions"],
    ),
    ("delivery remote", true, &["delivery.remote"]),
];

/// Decision 24: [`SECTIONS`].
pub fn sections() -> &'static [(&'static str, bool, &'static [&'static str])] {
    &SECTIONS
}

/// Whether `key`'s row sits inside Advanced.
pub fn in_advanced(key: &str) -> bool {
    section_of(key).1
}

/// The section of `key` and whether it sits inside Advanced (`env.<NAME>` is the
/// environment section's).
fn section_of(key: &str) -> (&'static str, bool) {
    let key = if key.starts_with("env.") {
        ENV_ADD
    } else {
        key
    };
    SECTIONS
        .iter()
        .find(|(_, _, keys)| keys.contains(&key))
        .map(|(title, advanced, _)| (*title, *advanced))
        .unwrap_or(("", false))
}

/// The key of the environment section's last row, which adds a variable.
pub const ENV_ADD: &str = "env";

/// A proposal row against the stored profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Added,
    Removed,
    Changed,
}

impl Mark {
    /// `+`, `−` (`-` in ASCII), `~`.
    pub fn glyph(self, ascii: bool) -> &'static str {
        match self {
            Mark::Added => "+",
            Mark::Removed if ascii => "-",
            Mark::Removed => "−",
            Mark::Changed => "~",
        }
    }
}

/// The editor a key gets (decision 35).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A command or a string: one text line.
    Text,
    /// One item per line.
    List,
    /// Digits.
    Number,
    /// One of these values.
    Choice(&'static [&'static str]),
    /// `env.<NAME>`: a name and a value.
    Env,
}

const LISTS: [&str; 12] = [
    "languages",
    "modules",
    "hub",
    "source",
    "generated",
    "protected",
    "filter_prefixes",
    "conventions",
    "manifests",
    "full_triggers",
    "skip_markers",
    "test_paths",
];

/// The editor kind of `key` (`env` and `env.<NAME>` are `Env`).
pub fn kind_of(key: &str) -> Kind {
    match key {
        k if k == ENV_ADD || k.starts_with("env.") => Kind::Env,
        "check_timeout_secs" | "full_shards" => Kind::Number,
        "module_names" => Kind::Choice(&["cargo", "dir"]),
        "delivery.mode" => Kind::Choice(&["local", "pr"]),
        "output_filter" => Kind::Choice(&["failures-only", "tail", "none"]),
        k if LISTS.contains(&k) => Kind::List,
        _ => Kind::Text,
    }
}

/// One row of the Profile tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Decision 24: the section's title.
    pub section: &'static str,
    /// Decision 24: the plain label.
    pub label: String,
    /// Whether the row sits inside Advanced.
    pub advanced: bool,
    /// The stored value this row is shown against (with `against`), as displayed.
    pub old: Option<String>,
    /// The `RepoProfile` key, `env.<NAME>`, or [`ENV_ADD`].
    pub key: String,
    /// The value as shown (a list's items joined with `, `); `None` when unset.
    pub value: Option<String>,
    /// Only on the proposal view.
    pub mark: Option<Mark>,
    /// The verification record of this row's command, when it ran this command.
    pub check: Option<CommandCheck>,
}

/// The verified command keys, in the screen's order: [`check_of`] answers for each.
pub const VERIFIED_KEYS: [&str; 8] = [
    "setup",
    "check",
    "single_test",
    "build_check",
    "module_test",
    "module_tests",
    "module_graph",
    "toolchain_id",
];

/// The verification record of a verified command key (`ProfileVerification`'s eight).
pub fn check_of<'a>(v: &'a ProfileVerification, key: &str) -> Option<&'a CommandCheck> {
    match key {
        "setup" => v.setup.as_ref(),
        "check" => v.check.as_ref(),
        "single_test" => v.single_test.as_ref(),
        "build_check" => v.build_check.as_ref(),
        "module_graph" => v.module_graph.as_ref(),
        "module_test" => v.module_test.as_ref(),
        "module_tests" => v.module_tests.as_ref(),
        "toolchain_id" => v.toolchain_id.as_ref(),
        _ => None,
    }
}

fn table_of(profile: &RepoProfile) -> toml::Table {
    match toml::Value::try_from(profile) {
        Ok(toml::Value::Table(table)) => table,
        _ => toml::Table::new(),
    }
}

/// A key's value in `table`, `None` for an absent key or an empty list (as
/// `profile_toml` leaves them out). `env.<NAME>` reads the environment table, and
/// `delivery.<sub>` the `[delivery]` table.
fn value_in(table: &toml::Table, key: &str) -> Option<toml::Value> {
    let nested = |table_name: &str, sub: &str| table.get(table_name)?.as_table()?.get(sub);
    let value = if let Some(name) = key.strip_prefix("env.") {
        nested("env", name)?
    } else if let Some(sub) = key.strip_prefix("delivery.") {
        nested("delivery", sub)?
    } else {
        table.get(key)?
    };
    match value {
        toml::Value::Array(items) if items.is_empty() => None,
        other => Some(other.clone()),
    }
}

/// How a value reads on a row: a string as written, a list's items joined with `, `.
fn display(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Array(items) => items.iter().map(display).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

fn env_names(table: &toml::Table) -> BTreeSet<String> {
    (table.get("env").and_then(toml::Value::as_table))
        .map(|env| env.keys().cloned().collect())
        .unwrap_or_default()
}

/// Every row of `profile`, section by section in decision 24's order (each
/// `env.<NAME>` before the environment section's add row). With `against` (the stored
/// profile), each row is marked against it and carries its old value, and an
/// environment entry only it has is listed too (removed). A verified command carries
/// its record while the record ran the row's command.
pub fn rows(
    profile: &RepoProfile,
    verification: Option<&ProfileVerification>,
    against: Option<&RepoProfile>,
) -> Vec<Row> {
    let table = table_of(profile);
    let other = against.map(table_of);
    let mut out = Vec::new();
    for (_, _, keys) in sections() {
        for key in keys.iter() {
            if *key == ENV_ADD {
                let mut names = env_names(&table);
                if let Some(other) = &other {
                    names.extend(env_names(other));
                }
                for name in names {
                    out.push(row(
                        &table,
                        other.as_ref(),
                        verification,
                        format!("env.{name}"),
                    ));
                }
            }
            out.push(row(&table, other.as_ref(), verification, key.to_string()));
        }
    }
    out
}

/// One row of `key` (the add row for [`ENV_ADD`], which has no value).
fn row(
    table: &toml::Table,
    other: Option<&toml::Table>,
    verification: Option<&ProfileVerification>,
    key: String,
) -> Row {
    let (section, advanced) = section_of(&key);
    let label = crate::profile_words::label(&key);
    if key == ENV_ADD {
        return Row {
            section,
            label,
            advanced,
            old: None,
            key,
            value: None,
            mark: None,
            check: None,
        };
    }
    let value = value_in(table, &key);
    let before = other.and_then(|other| value_in(other, &key));
    let mark = other.and_then(|_| match (&value, &before) {
        (Some(_), None) => Some(Mark::Added),
        (None, Some(_)) => Some(Mark::Removed),
        (Some(a), Some(b)) if a != b => Some(Mark::Changed),
        _ => None,
    });
    let shown = value.as_ref().map(display);
    let check = verification
        .and_then(|v| check_of(v, &key))
        .filter(|c| Some(&c.command) == shown.as_ref())
        .cloned();
    Row {
        section,
        label,
        advanced,
        old: before.as_ref().map(display),
        key,
        value: shown,
        mark,
        check,
    }
}

/// Interfaces "Profile check cell": `✓ 0 · 4s`, `✗ 101 · 12s`, `✗ timed out · 600s`;
/// in ASCII `+`/`x` and `-`.
pub fn check_cell(c: &CommandCheck, ascii: bool) -> String {
    let mark = glyph(if c.ok { Glyph::Passed } else { Glyph::Failed }, ascii);
    let dot = if ascii { "-" } else { "·" };
    let how = match (c.timed_out, c.code) {
        (true, _) => "timed out".to_string(),
        (false, Some(code)) => code.to_string(),
        (false, None) => "no exit code".to_string(),
    };
    format!("{mark} {how} {dot} {}s", c.secs)
}

/// Decision 28: the card's check cell, `✓ 12s` / `✗ 3m10s` (ASCII `+` / `x`).
pub fn card_cell(c: &CommandCheck, ascii: bool) -> String {
    let mark = glyph(if c.ok { Glyph::Passed } else { Glyph::Failed }, ascii);
    format!("{mark} {}", crate::profile_words::took(c.secs))
}

/// The changes card's rows: only those marked against the stored profile.
pub fn changed(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter().filter(|r| r.mark.is_some()).collect()
}

/// The text an editor of `key` starts from: a list one item per line.
pub fn edit_text(profile: &RepoProfile, key: &str) -> String {
    match value_in(&table_of(profile), key) {
        Some(toml::Value::Array(items)) => items.iter().map(display).collect::<Vec<_>>().join("\n"),
        Some(value) => display(&value),
        None => String::new(),
    }
}

/// Final review C-I1: a row edit's `value` as a row shows it, read as the daemon's
/// `parse_value` reads it (`v = <value>` when that parses as one TOML value, else the
/// text as typed); an environment value as typed, since it is stored unparsed; an unset
/// `—`.
pub fn tried_value(key: &str, value: Option<&str>) -> String {
    let Some(value) = value else {
        return "—".to_string();
    };
    if key.starts_with("env.") {
        return value.to_string();
    }
    match toml::from_str::<toml::Table>(&format!("v = {value}")) {
        Ok(table) if table.len() == 1 => table.get("v").map_or(value.to_string(), display),
        _ => value.to_string(),
    }
}

/// The `value` an edit of `key` sends for what was typed: a TOML literal written with
/// `toml::Value`'s `Display` (a string quoted and escaped, a list an array of its
/// non-blank trimmed lines, a number an integer), which `parse_value` reads back as
/// exactly that value. An environment value is sent as typed, since `apply_edit`
/// stores it unparsed; a `delivery.*` value bare and trimmed (`pr`, not `"pr"`), since
/// `proposal_delivery::edit` reads the text as `profile edit delivery.mode=pr` sends it.
pub fn value_literal(key: &str, typed: &str) -> Result<String, String> {
    if key.starts_with("delivery.") {
        let text = typed.trim();
        if text.is_empty() {
            return Err("type a value first, or u to unset".to_string());
        }
        return Ok(text.to_string());
    }
    let value = match kind_of(key) {
        Kind::Env => return Ok(typed.to_string()),
        Kind::List => toml::Value::Array(
            typed
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| toml::Value::String(line.to_string()))
                .collect(),
        ),
        Kind::Number => match typed.trim().parse::<i64>() {
            Ok(n) if n >= 0 => toml::Value::Integer(n),
            _ => return Err(format!("{key}: digits only")),
        },
        Kind::Text | Kind::Choice(_) => {
            let text = typed.trim();
            if text.is_empty() {
                return Err("type a value first, or u to unset".to_string());
            }
            toml::Value::String(text.to_string())
        }
    };
    Ok(value.to_string())
}

#[cfg(test)]
#[path = "profile_view_tests.rs"]
mod tests;
