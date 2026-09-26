//! Proposal rules (decisions 5, 9 and 10): validation, the onboarding scout's findings
//! and `anthrex profile edit`. Pure (decision 1): no file, process, thread,
//! async-runtime or clock access.
//!
//! Every key here is a `RepoProfile` field, and `RepoProfile` has no confinement key, so
//! neither a scout's findings nor an edit can name one: confinement stays the user's own
//! config (decision 5).

use std::path::Path;

use proto::{CommandCheck, DroppedCommand, ProfileVerification, RepoProfile};

use super::confined_hint;
use crate::run::globs::validate_glob;
use crate::run::plan::BUILTIN_PROTECTED;
use crate::run::report::format_utc;

/// The keys whose change re-runs verification (decision 10).
pub const REVERIFY_KEYS: &[&str] = &[
    "setup",
    "check",
    "check_timeout_secs",
    "single_test",
    "test_passed",
    "sample_test",
    "env",
];

/// What `anthrex profile edit` accepts: every `RepoProfile` field, and `env.<NAME>` for
/// one environment entry (decision 10). The order is the field order.
pub const EDIT_KEYS: &[&str] = &[
    "languages",
    "modules",
    "hub",
    "source",
    "generated",
    "protected",
    "setup",
    "check",
    "check_timeout_secs",
    "single_test",
    "test_passed",
    "sample_test",
    "output_filter",
    "filter_prefixes",
    "conventions",
    "manifests",
    "env",
    "env.<NAME>",
];

/// M8a's range for `check_timeout_secs`.
const CHECK_TIMEOUT_RANGE: std::ops::RangeInclusive<u64> = 10..=14_400;

/// Every glob list, and the path lists decision 7 fingerprints (held to the same rule,
/// so a fingerprint never reads outside the repository).
fn path_lists(profile: &RepoProfile) -> [(&'static str, &Vec<String>); 7] {
    [
        ("modules", &profile.modules),
        ("hub", &profile.hub),
        ("source", &profile.source),
        ("generated", &profile.generated),
        ("protected", &profile.protected),
        ("conventions", &profile.conventions),
        ("manifests", &profile.manifests),
    ]
}

fn is_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Why `env` may not hold `key`, M8a's texts, or `None`.
fn env_problem(key: &str) -> Option<String> {
    if !is_env_key(key) {
        return Some(format!("key {key} must match [A-Za-z_][A-Za-z0-9_]*"));
    }
    config::reserved_env::reserved_env(key)
        .map(|reason| format!("key {key} may not be set by a profile: {reason}"))
}

/// `test_passed`'s problem, as M8a decision 7 words it, or `None`.
fn test_passed_problem(passed: &str) -> Option<String> {
    if !passed.contains("{test}") {
        return Some("must contain {test}".to_string());
    }
    // `{test}` is replaced by an escaped test name before use, so compile it with a
    // stand-in name (as M8a's plan check does).
    let sample = passed.replace("{test}", &regex::escape("t"));
    regex::Regex::new(&sample)
        .err()
        .map(|e| format!("is not a valid regular expression: {e}"))
}

/// Every problem in `profile`, each `<key>: <problem>`: globs and paths, `{test}`, the
/// regular expression, `check_timeout_secs`'s range, and `env` keys (M8a's reserved
/// list, `config::reserved_env`).
pub fn validate(profile: &RepoProfile) -> Vec<String> {
    let mut problems = Vec::new();
    for (key, list) in path_lists(profile) {
        for entry in list {
            if let Err(e) = validate_glob(entry) {
                problems.push(format!("{key}: {entry} {e}"));
            }
        }
    }
    if let Some(secs) = profile.check_timeout_secs
        && !CHECK_TIMEOUT_RANGE.contains(&secs)
    {
        problems.push(format!(
            "check_timeout_secs: must be between {} and {}",
            CHECK_TIMEOUT_RANGE.start(),
            CHECK_TIMEOUT_RANGE.end()
        ));
    }
    if let Some(single) = &profile.single_test
        && !single.contains("{test}")
    {
        problems.push("single_test: must contain {test}".to_string());
    }
    if let Some(problem) = profile.test_passed.as_deref().and_then(test_passed_problem) {
        problems.push(format!("test_passed: {problem}"));
    }
    for key in profile.env.keys() {
        if let Some(problem) = env_problem(key) {
            problems.push(format!("env: {problem}"));
        }
    }
    problems
}

/// The scout's profile with what cannot be proposed removed (decision 5): list entries
/// that fail the glob rule, built-in and repeated `protected` entries, `env` keys that
/// are malformed or reserved, and an out-of-range `check_timeout_secs`. Commands are
/// left for verification, which drops them with a reason (decision 9).
pub fn from_findings(proposed: &RepoProfile) -> RepoProfile {
    let mut out = proposed.clone();
    let valid = |list: &Vec<String>| -> Vec<String> {
        list.iter()
            .filter(|entry| validate_glob(entry).is_ok())
            .cloned()
            .collect()
    };
    out.modules = valid(&proposed.modules);
    out.hub = valid(&proposed.hub);
    out.source = valid(&proposed.source);
    out.generated = valid(&proposed.generated);
    out.conventions = valid(&proposed.conventions);
    out.manifests = valid(&proposed.manifests);
    out.protected = Vec::new();
    for entry in valid(&proposed.protected) {
        if !BUILTIN_PROTECTED.contains(&entry.as_str()) && !out.protected.contains(&entry) {
            out.protected.push(entry);
        }
    }
    out.env.retain(|key, _| env_problem(key).is_none());
    out.check_timeout_secs = proposed
        .check_timeout_secs
        .filter(|secs| CHECK_TIMEOUT_RANGE.contains(secs));
    out
}

/// A command-line value: a TOML value when `v = <value>` parses as exactly that one
/// key, else the text as a string (so `edit check 'cargo test'` works).
fn parse_value(value: &str) -> toml::Value {
    match toml::from_str::<toml::Table>(&format!("v = {value}")) {
        Ok(mut table) if table.len() == 1 => table
            .remove("v")
            .unwrap_or_else(|| toml::Value::String(value.to_string())),
        _ => toml::Value::String(value.to_string()),
    }
}

/// Decision 10's `anthrex profile edit <key> <value>` (`None`: `--unset`) on the
/// stored profile. The bool says whether verification must re-run. Refused: a key that
/// is not a `RepoProfile` field (`unknown key <key>; one of <keys>`), a value of the
/// wrong type, a built-in `protected` entry, and any value `validate` rejects.
pub fn apply_edit(
    stored: &RepoProfile,
    key: &str,
    value: Option<&str>,
) -> Result<(RepoProfile, bool), String> {
    let (field, env_name) = match key.strip_prefix("env.") {
        Some(name) if !name.is_empty() => ("env", Some(name)),
        _ => (key, None),
    };
    if !EDIT_KEYS.contains(&field) || field == "env.<NAME>" {
        return Err(format!(
            "unknown key {key}; one of {}",
            EDIT_KEYS.join(", ")
        ));
    }
    let mut table = match toml::Value::try_from(stored) {
        Ok(toml::Value::Table(table)) => table,
        Ok(_) => return Err("the stored profile is not a table".to_string()),
        Err(e) => return Err(format!("the stored profile cannot be edited: {e}")),
    };
    match (env_name, value) {
        (Some(name), value) => {
            if let Some(problem) = env_problem(name) {
                return Err(format!("env: {problem}"));
            }
            let env = table
                .entry("env")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            let Some(env) = env.as_table_mut() else {
                return Err("env: must be a table".to_string());
            };
            match value {
                Some(text) => {
                    env.insert(name.to_string(), toml::Value::String(text.to_string()));
                }
                None => {
                    env.remove(name);
                }
            }
        }
        (None, Some(text)) => {
            table.insert(field.to_string(), parse_value(text));
        }
        (None, None) => {
            table.remove(field);
        }
    }
    let edited: RepoProfile = toml::Value::Table(table)
        .try_into()
        .map_err(|e: toml::de::Error| format!("{key}: {}", e.message()))?;
    if let Some(builtin) = edited
        .protected
        .iter()
        .find(|entry| BUILTIN_PROTECTED.contains(&entry.as_str()))
    {
        return Err(format!(
            "protected: {builtin} is built in and always applies; list only extra entries"
        ));
    }
    let prefix = format!("{field}: ");
    let problems: Vec<String> = validate(&edited)
        .into_iter()
        .filter(|p| p.starts_with(&prefix))
        .collect();
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let reverify = REVERIFY_KEYS.contains(&field) && edited != *stored;
    Ok((edited, reverify))
}

/// Decision 9: why a `single_test` that cannot be verified is not proposed.
pub const SINGLE_TEST_NEEDS: &str = "single_test needs sample_test and test_passed to be verified";

/// What every dropped `single_test` reason ends with: the three keys go together.
const DROPPED_WITH_IT: &str = "; test_passed and sample_test are dropped with it";

/// Why a command the verification has no record of (or a record of another command)
/// is not proposed.
const NOT_VERIFIED: &str = "it was not verified";

/// Decision 5's `protected: built-in <5 globs> + <extras or "no extras">`.
pub fn protected_line(profile: &RepoProfile) -> String {
    let extras = if profile.protected.is_empty() {
        "no extras".to_string()
    } else {
        profile.protected.join(", ")
    };
    format!(
        "protected: built-in {} + {extras}",
        BUILTIN_PROTECTED.join(", ")
    )
}

/// How a command failed: its timeout, its exit code, or neither.
fn failure(check: &CommandCheck) -> String {
    match (check.timed_out, check.code) {
        (true, _) => format!("timed out after {}s", check.secs),
        (false, Some(code)) => format!("exit {code} after {}s", check.secs),
        (false, None) => format!("no exit code after {}s", check.secs),
    }
}

/// Why the proposed `single_test` template cannot even be run: M8a decision 7's rules
/// on the two templates, then decision 9's need for both companions.
fn single_test_problem(proposed: &RepoProfile, single: &str) -> Option<String> {
    if !single.contains("{test}") {
        return Some("single_test: must contain {test}".to_string());
    }
    if let Some(problem) = proposed
        .test_passed
        .as_deref()
        .and_then(test_passed_problem)
    {
        return Some(format!("test_passed: {problem}"));
    }
    (proposed.sample_test.is_none() || proposed.test_passed.is_none())
        .then(|| SINGLE_TEST_NEEDS.to_string())
}

/// Decision 9: the proposal keeps only the commands that passed. A `setup` or `check`
/// that failed, timed out or has no record of its own command in `v` is removed and
/// listed (with decision 9's hint, naming `root`, when `v` ran confined); a
/// `single_test` goes with `test_passed` and `sample_test` unless it exited 0 **and**
/// matched its line. Without a `single_test`, its two companions mean nothing and are
/// removed silently. The glob lists are left as they are.
pub fn apply_verification(
    proposed: &RepoProfile,
    v: &ProfileVerification,
    root: &Path,
) -> (RepoProfile, Vec<DroppedCommand>) {
    let mut profile = proposed.clone();
    let mut dropped = Vec::new();
    let hint = v.confined.then(|| confined_hint(root));
    for (key, slot, record) in [
        ("setup", &mut profile.setup, &v.setup),
        ("check", &mut profile.check, &v.check),
    ] {
        let Some(command) = slot.clone() else {
            continue;
        };
        let (reason, tail) = match record {
            Some(c) if c.command == command && c.ok => continue,
            Some(c) if c.command == command => {
                let reason = match &hint {
                    Some(hint) => format!("{}\n{hint}", failure(c)),
                    None => failure(c),
                };
                (reason, c.tail.clone())
            }
            _ => (NOT_VERIFIED.to_string(), String::new()),
        };
        *slot = None;
        dropped.push(DroppedCommand {
            key: key.to_string(),
            command,
            reason,
            tail,
        });
    }
    let Some(single) = proposed.single_test.clone() else {
        profile.test_passed = None;
        profile.sample_test = None;
        return (profile, dropped);
    };
    let failed = match single_test_problem(proposed, &single) {
        Some(problem) => Some((problem, String::new())),
        None => match &v.single_test {
            Some(c) if c.command == single && c.ok => None,
            Some(c) if c.command == single => {
                let reason = if !c.timed_out && c.code == Some(0) {
                    format!(
                        "exit 0 after {}s, but no output line matched test_passed for \
                         sample_test {}",
                        c.secs,
                        proposed.sample_test.as_deref().unwrap_or_default()
                    )
                } else {
                    failure(c)
                };
                Some((reason, c.tail.clone()))
            }
            _ => Some((NOT_VERIFIED.to_string(), String::new())),
        },
    };
    if let Some((reason, tail)) = failed {
        profile.single_test = None;
        profile.test_passed = None;
        profile.sample_test = None;
        dropped.push(DroppedCommand {
            key: "single_test".to_string(),
            command: single,
            reason: format!("{reason}{DROPPED_WITH_IT}"),
            tail,
        });
    }
    (profile, dropped)
}

/// The profile as TOML in its field order, empty lists left out, `env` last as its own
/// table.
fn profile_toml(profile: &RepoProfile) -> String {
    let Ok(toml::Value::Table(table)) = toml::Value::try_from(profile) else {
        return String::new();
    };
    let mut out = String::new();
    for key in EDIT_KEYS.iter().filter(|key| !key.starts_with("env")) {
        match table.get(*key) {
            None => {}
            Some(toml::Value::Array(items)) if items.is_empty() => {}
            Some(value) => out.push_str(&format!("{key} = {value}\n")),
        }
    }
    if !profile.env.is_empty() {
        let mut env = toml::Table::new();
        if let Some(value) = table.get("env") {
            env.insert("env".to_string(), value.clone());
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&toml::to_string(&env).unwrap_or_default());
    }
    out
}

/// `anthrex profile show`'s text (Interfaces, CLI): the profile as TOML, then a comment
/// block with decision 5's protected line, the verification (when there is one) and
/// every dropped command with its reason's further lines and its output tail.
pub fn show_text(
    profile: &RepoProfile,
    verification: Option<&ProfileVerification>,
    dropped: &[DroppedCommand],
) -> String {
    let mut out = profile_toml(profile);
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("# {}\n", protected_line(profile)));
    if let Some(v) = verification {
        let at = format_utc(v.at);
        let confined = if v.confined { "confined" } else { "unconfined" };
        out.push_str(&format!(
            "# verification {} ({confined})\n",
            at.get(..16).unwrap_or(&at)
        ));
        for (key, check) in [
            ("setup", &v.setup),
            ("check", &v.check),
            ("single_test", &v.single_test),
        ] {
            let Some(c) = check else {
                continue;
            };
            let status = if c.ok { "ok" } else { "fail" };
            let mut line = format!("#   {key:<13}{status:<4}{:>4}s   {}", c.secs, c.command);
            if key == "single_test"
                && let Some(sample) = &profile.sample_test
            {
                line.push_str(&format!("  (sample: {sample})"));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    if !dropped.is_empty() {
        out.push_str("# dropped\n");
    }
    for d in dropped {
        let mut reason = d.reason.lines();
        out.push_str(&format!(
            "#   {}: {}: {}\n",
            d.key,
            reason.next().unwrap_or_default(),
            d.command
        ));
        for line in reason.chain(d.tail.lines()) {
            out.push_str(format!("#     {line}").trim_end());
            out.push('\n');
        }
    }
    out
}
