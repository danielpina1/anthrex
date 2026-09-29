//! `submit_scout_report`'s arguments, checked again on the daemon side (milestone 8b
//! decision 13), and where a report is stored. Pure.
//!
//! The limits are the tool schema's (`mcp::tools_scout`); every object is closed, so a
//! confinement key the profile type does not have (`cache_dirs`, the `confined_*`
//! tables) is refused as not in the schema.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use proto::{OutputFilter, RepoProfile, Route, ScoutFile, ScoutKind, ScoutReport, ScoutState};

use super::machine::ScoutMachine;
use super::spec::ScoutSpec;
use serde_json::{Map, Value};

/// The directory, under a repository's or a run's data directory, that holds reports.
pub const SCOUTS_DIR: &str = "scouts";
/// The `scout_refs` alias for the stored profile's onboarding report.
pub const ONBOARDING_ALIAS: &str = "onboarding";

/// A report's arguments, validated. Serde since milestone 9, which keeps a research
/// task's report in `Task.orch.research` (decision 35).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScoutReportArgs {
    pub summary: String,
    pub files: Vec<ScoutFile>,
    pub modules: Vec<String>,
    pub interfaces: Vec<String>,
    pub risks: Vec<String>,
    pub profile: Option<RepoProfile>,
}

const ROOT_KEYS: &[&str] = &[
    "summary",
    "files",
    "modules",
    "interfaces",
    "risks",
    "profile",
];
const PROFILE_KEYS: &[&str] = &[
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
];
const ENV_MAX: usize = 20;
/// Ruling M6: the longest `profile.env` key (the brief bounds only the count, 20, and
/// the values, 1000 characters).
const ENV_KEY_CHARS: usize = 64;
const ENV_KEY: &str = "^[A-Za-z_][A-Za-z0-9_]*$";

/// `args` as a report of a `kind` scout, or `invalid arguments: <field>: <problem>`.
pub fn validate(args: &Value, kind: ScoutKind) -> Result<ScoutReportArgs, String> {
    check(args, kind).map_err(|problem| format!("invalid arguments: {problem}"))
}

fn check(args: &Value, kind: ScoutKind) -> Result<ScoutReportArgs, String> {
    let root = closed(args, "", ROOT_KEYS, &["summary", "files"])?;
    let summary = string(&root["summary"], "summary", 1, 8000)?;
    let mut files = Vec::new();
    for (i, file) in array(&root["files"], "files", 60)?.iter().enumerate() {
        let path = format!("files[{i}]");
        let file = closed(file, &path, &["path", "why"], &["path", "why"])?;
        files.push(ScoutFile {
            path: string(&file["path"], &at(&path, "path"), 1, 500)?,
            why: string(&file["why"], &at(&path, "why"), 1, 300)?,
        });
    }
    let modules = strings(root.get("modules"), "modules", 40, 200)?;
    let interfaces = strings(root.get("interfaces"), "interfaces", 40, 500)?;
    let risks = strings(root.get("risks"), "risks", 20, 500)?;
    let profile = match (root.get("profile"), kind) {
        (None, ScoutKind::Onboarding) => {
            return Err("profile: required for the onboarding scout".into());
        }
        (Some(_), ScoutKind::Area) => {
            return Err("profile: only the onboarding scout reports a profile".into());
        }
        (None, ScoutKind::Area) => None,
        (Some(value), ScoutKind::Onboarding) => Some(profile(value)?),
    };
    Ok(ScoutReportArgs {
        summary,
        files,
        modules,
        interfaces,
        risks,
        profile,
    })
}

fn profile(value: &Value) -> Result<RepoProfile, String> {
    let p = closed(value, "profile", PROFILE_KEYS, &[])?;
    let list =
        |key: &str, max: usize, chars: usize| strings(p.get(key), &at("profile", key), max, chars);
    let text = |key: &str, chars: usize| {
        p.get(key)
            .map(|v| string(v, &at("profile", key), 1, chars))
            .transpose()
    };
    let check_timeout_secs = match p.get("check_timeout_secs") {
        None => None,
        Some(value) => {
            let path = "profile.check_timeout_secs";
            let n = value
                .as_u64()
                .ok_or_else(|| format!("{path}: expected an integer"))?;
            if !(10..=14400).contains(&n) {
                return Err(format!("{path}: must be between 10 and 14400"));
            }
            Some(n)
        }
    };
    let output_filter = match p.get("output_filter").map(Value::as_str) {
        None => OutputFilter::default(),
        Some(Some("failures-only")) => OutputFilter::FailuresOnly,
        Some(Some("tail")) => OutputFilter::Tail,
        Some(Some("none")) => OutputFilter::None,
        Some(_) => {
            return Err("profile.output_filter: expected failures-only, tail or none".into());
        }
    };
    Ok(RepoProfile {
        languages: list("languages", 10, 40)?,
        modules: list("modules", 40, 300)?,
        hub: list("hub", 40, 300)?,
        source: list("source", 40, 300)?,
        generated: list("generated", 40, 300)?,
        protected: list("protected", 40, 300)?,
        setup: text("setup", 2000)?,
        check: text("check", 2000)?,
        check_timeout_secs,
        single_test: text("single_test", 1000)?,
        test_passed: text("test_passed", 300)?,
        sample_test: text("sample_test", 300)?,
        output_filter,
        filter_prefixes: list("filter_prefixes", 10, 100)?,
        conventions: list("conventions", 20, 300)?,
        manifests: list("manifests", 50, 300)?,
        // Milestone 9.1's tier keys: not in the onboarding report until task M9.1.5.
        build_check: None,
        module_test: None,
        module_tests: None,
        module_graph: None,
        module_names: None,
        full_triggers: Vec::new(),
        slow_tests: None,
        timing_tests: None,
        skip_markers: Vec::new(),
        test_paths: Vec::new(),
        full_shards: None,
        toolchain_id: None,
        env: env(p.get("env"))?,
    })
}

fn env(value: Option<&Value>) -> Result<BTreeMap<String, String>, String> {
    let path = "profile.env";
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let Value::Object(map) = value else {
        return Err(format!("{path}: expected an object"));
    };
    if map.len() > ENV_MAX {
        return Err(format!("{path}: must have at most {ENV_MAX} properties"));
    }
    let mut env = BTreeMap::new();
    for (key, value) in map {
        // Ruling M6: checked before the pattern, so an oversized key is never echoed.
        if key.chars().count() > ENV_KEY_CHARS {
            return Err(format!(
                "{path}: a key is longer than {ENV_KEY_CHARS} characters"
            ));
        }
        if !env_key(key) {
            return Err(format!("{path}: key {key} must match {ENV_KEY}"));
        }
        env.insert(key.clone(), string(value, &at(path, key), 0, 1000)?);
    }
    Ok(env)
}

/// `^[A-Za-z_][A-Za-z0-9_]*$`.
fn env_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `path.key`, or `key` at the root.
fn at(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// `path: problem`, or `problem` at the root.
fn problem(path: &str, problem: &str) -> String {
    if path.is_empty() {
        problem.to_string()
    } else {
        format!("{path}: {problem}")
    }
}

/// An object with only `keys`, every one of `required` present.
fn closed<'a>(
    value: &'a Value,
    path: &str,
    keys: &[&str],
    required: &[&str],
) -> Result<&'a Map<String, Value>, String> {
    let Value::Object(map) = value else {
        return Err(problem(path, "expected an object"));
    };
    if let Some(key) = required.iter().find(|k| !map.contains_key(**k)) {
        return Err(format!("{}: missing", at(path, key)));
    }
    if let Some(key) = map.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("{}: not in the schema", at(path, key)));
    }
    Ok(map)
}

fn array<'a>(value: &'a Value, path: &str, max: usize) -> Result<&'a [Value], String> {
    let Value::Array(items) = value else {
        return Err(problem(path, "expected an array"));
    };
    if items.len() > max {
        return Err(problem(path, &format!("must have at most {max} items")));
    }
    Ok(items)
}

/// A string of `min..=max` characters (JSON Schema counts characters, not bytes).
fn string(value: &Value, path: &str, min: usize, max: usize) -> Result<String, String> {
    let Value::String(s) = value else {
        return Err(problem(path, "expected a string"));
    };
    let chars = s.chars().count();
    if chars < min {
        return Err(problem(path, "must not be empty"));
    }
    if chars > max {
        return Err(problem(path, &format!("must be at most {max} characters")));
    }
    Ok(s.clone())
}

/// An optional array of at most `max` strings of 1 to `chars` characters.
fn strings(
    value: Option<&Value>,
    path: &str,
    max: usize,
    chars: usize,
) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    array(value, path, max)?
        .iter()
        .enumerate()
        .map(|(i, item)| string(item, &format!("{path}[{i}]"), 1, chars))
        .collect()
}

/// Where scout `id`'s report is stored: `<run_dir>/scouts/<id>.json` for a run scout,
/// `<repo_dir>/scouts/<id>.json` for a repository one.
pub fn report_path(repo_dir: &Path, run_dir: Option<&Path>, id: &str) -> PathBuf {
    run_dir
        .unwrap_or(repo_dir)
        .join(SCOUTS_DIR)
        .join(format!("{id}.json"))
}

/// The report a task's `scout_refs` entry names: the alias `onboarding` is the stored
/// profile's onboarding report, when there is one; anything else is a run scout's.
pub fn resolve_ref(
    reference: &str,
    run_dir: &Path,
    repo_dir: &Path,
    onboarding_report: Option<&str>,
) -> PathBuf {
    match onboarding_report {
        Some(id) if reference == ONBOARDING_ALIAS => report_path(repo_dir, None, id),
        _ => report_path(repo_dir, Some(run_dir), reference),
    }
}

/// The stored report of `spec`'s scout from its accepted arguments.
pub fn build_report(
    spec: &ScoutSpec,
    route: Route,
    window_id: u32,
    machine: &ScoutMachine,
    args: ScoutReportArgs,
    finished_at: u64,
) -> ScoutReport {
    ScoutReport {
        id: spec.id.clone(),
        kind: spec.kind,
        run_id: spec.run_id.clone(),
        question: spec.question.clone(),
        summary: args.summary,
        files: args.files,
        modules: args.modules,
        interfaces: args.interfaces,
        risks: args.risks,
        profile: args.profile,
        route,
        window_id: Some(window_id),
        started_at: machine.started_at,
        finished_at,
        tool_calls: machine.tool_calls,
        usage: machine.usage,
    }
}

/// A state as the refusals spell it: `scout <id> is failed`.
pub fn state_label(state: ScoutState) -> &'static str {
    match state {
        ScoutState::Starting => "starting",
        ScoutState::Working => "working",
        ScoutState::Reported => "reported",
        ScoutState::Failed => "failed",
    }
}
