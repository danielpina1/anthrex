//! Plan parsing and run building: decision 7 (the plan file and the per-key profile),
//! the run limits, decision 15's slug and decision 56's protected-path merge. Pure — no
//! `std::fs`, `std::process`, `std::thread`, `tokio` or `std::time::SystemTime` (design
//! decision 2). Task resolution and the cross-task rules are in `validate.rs`.

use std::collections::BTreeSet;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;

use proto::{EditFile, Plan, PlanEdit, ProfileSpec, RunState};

use super::globs::validate_glob;
use super::model::{Profile, Run, RunLimits, task_branch, task_path};
use super::validate::{
    EditScope, combined_cycles, implicit_deps, resolve_task_lenient, validate_tasks,
};

/// Decision 56's built-in protected paths. Configuration and the plan add to them;
/// neither can remove one.
pub const BUILTIN_PROTECTED: &[&str] = &[
    ".claude/**",
    ".mcp.json",
    ".codex/**",
    "**/CLAUDE.md",
    "**/AGENTS.md",
];

/// `check_timeout_secs` when neither the plan nor the config sets it: the spec's 30
/// minutes.
pub const DEFAULT_CHECK_TIMEOUT_SECS: u64 = 1800;
const CHECK_TIMEOUT_RANGE: std::ops::RangeInclusive<u64> = 10..=14_400;
const WRITERS_RANGE: std::ops::RangeInclusive<u8> = 1..=8;
const READERS_RANGE: std::ops::RangeInclusive<u8> = 1..=8;
const BOUNCES_RANGE: std::ops::RangeInclusive<u8> = 1..=5;
/// Decision 15: the goal part of a run id is cut to this many characters.
const SLUG_MAX: usize = 32;

/// One validation problem. `rule` is a short stable id: a spec rule number where the
/// message cites one (`7.2.4`, `8.1`, `9`, `12.1`), else the family (`fields`, `id`,
/// `globs`, `kind`, `range`, `profile`, `route`, `8`). A plan edit's per-state refusal
/// (decision 13) has rule `13` and an empty `field`; its `message` is a whole sentence
/// naming the task and its state, and is displayed alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanError {
    pub task: Option<String>,
    pub field: String,
    pub rule: String,
    pub message: String,
}

impl PlanError {
    pub fn new(task: Option<&str>, field: &str, rule: &str, message: impl Into<String>) -> Self {
        PlanError {
            task: task.map(str::to_string),
            field: field.to_string(),
            rule: rule.to_string(),
            message: message.into(),
        }
    }
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.field.is_empty() {
            // A whole sentence that already names its task (decision 13's refusals).
            return f.write_str(&self.message);
        }
        match &self.task {
            Some(id) => write!(f, "task {id}: {}: {}", self.field, self.message),
            None => write!(f, "{}: {}", self.field, self.message),
        }
    }
}

/// What start preflight (M8a.8, decision 17) found, the input to [`build_run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preflight {
    pub root: PathBuf,
    pub project: PathBuf,
    pub git_common_dir: PathBuf,
    pub base_branch: String,
    pub base_sha: String,
    pub protected_files: Vec<String>,
}

pub struct BuildContext<'a> {
    pub id: String,
    pub wt_dir: PathBuf,
    /// The run's own data directory, `<data_dir>/runs/<id>`.
    pub data_dir: PathBuf,
    pub config: &'a config::Orchestrator,
    pub now: u64,
    pub yes: bool,
}

/// Parses a plan file. The error is the `toml` crate's, which names the line and the
/// offending key (every table is `deny_unknown_fields`).
pub fn parse_plan(text: &str) -> Result<Plan, String> {
    toml::from_str(text).map_err(|e| e.to_string())
}

/// Parses an edit batch (`[[edit]]` tables, `proto::EditFile`).
pub fn parse_edits(text: &str) -> Result<Vec<PlanEdit>, String> {
    toml::from_str::<EditFile>(text)
        .map(|file| file.edits)
        .map_err(|e| e.to_string())
}

fn non_blank(value: Option<&String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty()).cloned()
}

/// Decision 7: each key from the plan when present, else from `[orchestrator.profile]`,
/// else empty (`check_timeout_secs` 1800). A present-but-blank string key resolves to
/// `None`, so a plan can switch off a configured `check`. `protected` merges instead:
/// the built-ins, then the config's additions, then the plan's, without duplicates.
pub fn resolve_profile(plan: &ProfileSpec, config: &ProfileSpec) -> Profile {
    fn pick<T: Clone>(plan: &Option<T>, config: &Option<T>) -> Option<T> {
        plan.clone().or_else(|| config.clone())
    }
    let mut protected: Vec<String> = BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect();
    for extra in config
        .protected
        .iter()
        .chain(plan.protected.iter())
        .flatten()
    {
        if !protected.contains(extra) {
            protected.push(extra.clone());
        }
    }
    Profile {
        modules: pick(&plan.modules, &config.modules).unwrap_or_default(),
        hub: pick(&plan.hub, &config.hub).unwrap_or_default(),
        source: pick(&plan.source, &config.source).unwrap_or_default(),
        check: non_blank(pick(&plan.check, &config.check).as_ref()),
        check_timeout_secs: pick(&plan.check_timeout_secs, &config.check_timeout_secs)
            .unwrap_or(DEFAULT_CHECK_TIMEOUT_SECS),
        single_test: non_blank(pick(&plan.single_test, &config.single_test).as_ref()),
        test_passed: non_blank(pick(&plan.test_passed, &config.test_passed).as_ref()),
        setup: non_blank(pick(&plan.setup, &config.setup).as_ref()),
        generated: pick(&plan.generated, &config.generated).unwrap_or_default(),
        protected,
        env: pick(&plan.env, &config.env).unwrap_or_default(),
    }
}

/// The limits a run is frozen with: the config, with the plan's three limits winning
/// when set. Ranges are checked separately, by [`build_run`].
pub fn run_limits(
    config: &config::Orchestrator,
    max_writers: Option<u8>,
    max_readers: Option<u8>,
    max_bounces: Option<u8>,
) -> RunLimits {
    RunLimits {
        max_writers: max_writers.unwrap_or(config.max_writers),
        max_readers: max_readers.unwrap_or(config.max_readers),
        max_bounces: max_bounces.unwrap_or(config.max_bounces),
        max_tasks: config.max_tasks,
        max_windows: config.max_windows,
        default_runtime: config.default_runtime,
        review_small: config.review_small,
        budget_s: config.budget_s,
        budget_m: config.budget_m,
        budget_l: config.budget_l,
        stall_after_secs: config.stall_after_secs,
        rate_limit_retry_secs: config.rate_limit_retry_secs,
        denials_before_block: config.denials_before_block,
        git_timeout_secs: config.git_timeout_secs,
        worker_permission_mode: config.worker_permission_mode.clone(),
        worker_allowed_tools: config.worker_allowed_tools.clone(),
        worker_codex_sandbox: config.worker_codex_sandbox.clone(),
        worker_sandbox: config.worker_sandbox,
        claude_auth: config.claude.auth.into(),
        api_key_helper: config.claude.api_key_helper.clone(),
    }
}

fn check_range<T: PartialOrd + fmt::Display>(
    field: &str,
    value: Option<T>,
    range: &std::ops::RangeInclusive<T>,
    errors: &mut Vec<PlanError>,
) {
    if let Some(v) = value
        && !range.contains(&v)
    {
        errors.push(PlanError::new(
            None,
            field,
            "range",
            format!("must be between {} and {}", range.start(), range.end()),
        ));
    }
}

fn is_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Plan-level checks: the goal, the plan's limits, and the resolved profile.
fn check_plan(plan: &Plan, profile: &Profile, errors: &mut Vec<PlanError>) {
    if plan.goal.trim().is_empty() {
        errors.push(PlanError::new(None, "goal", "fields", "must not be blank"));
    }
    check_range("max_writers", plan.max_writers, &WRITERS_RANGE, errors);
    check_range("max_readers", plan.max_readers, &READERS_RANGE, errors);
    check_range("max_bounces", plan.max_bounces, &BOUNCES_RANGE, errors);
    check_range(
        "profile.check_timeout_secs",
        Some(profile.check_timeout_secs),
        &CHECK_TIMEOUT_RANGE,
        errors,
    );
    if let Some(single) = &profile.single_test
        && !single.contains("{test}")
    {
        errors.push(PlanError::new(
            None,
            "profile.single_test",
            "profile",
            "must contain {test}",
        ));
    }
    if let Some(passed) = &profile.test_passed {
        // `{test}` is replaced by an escaped test name before use (decision 34's
        // `Proof`), so compile it with a stand-in name, not as a literal repetition.
        let sample = passed.replace("{test}", &regex::escape("t"));
        if let Err(e) = regex::Regex::new(&sample) {
            errors.push(PlanError::new(
                None,
                "profile.test_passed",
                "profile",
                format!("is not a valid regular expression: {e}"),
            ));
        }
    }
    for key in profile.env.keys() {
        if !is_env_key(key) {
            errors.push(PlanError::new(
                None,
                "profile.env",
                "profile",
                format!("key {key} must match [A-Za-z_][A-Za-z0-9_]*"),
            ));
        }
    }
    for (field, globs) in [
        ("profile.generated", &profile.generated),
        ("profile.protected", &profile.protected),
    ] {
        for glob in globs {
            if let Err(e) = validate_glob(glob) {
                errors.push(PlanError::new(None, field, "globs", format!("{glob} {e}")));
            }
        }
    }
}

/// Turns a parsed plan into a run in `awaiting_approval`, or every problem found.
/// `ctx.yes` records `approved by --yes` in `approved_by`; moving the run to `running`
/// is the engine's (M8a.11), since that needs the run branch first.
pub fn build_run(plan: Plan, pre: Preflight, ctx: BuildContext<'_>) -> Result<Run, Vec<PlanError>> {
    let config = ctx.config;
    let profile = resolve_profile(&plan.profile, &config.profile);
    let limits = run_limits(config, plan.max_writers, plan.max_readers, plan.max_bounces);
    let mut errors = Vec::new();
    check_plan(&plan, &profile, &mut errors);

    let mut tasks = Vec::with_capacity(plan.tasks.len());
    for spec in plan.tasks {
        let (mut task, task_errors) = resolve_task_lenient(
            spec,
            &profile,
            &limits,
            &config.models,
            limits.default_runtime,
        );
        errors.extend(task_errors);
        task.branch = task_branch(&ctx.id, task.id());
        task.worktree = task_path(&ctx.wt_dir, &ctx.id, task.id());
        task.notes.extend(super::validate::protected_notes(
            &task.spec.owns,
            &pre.protected_files,
        ));
        tasks.push(task);
    }

    let touched: BTreeSet<String> = tasks.iter().map(|t| t.spec.id.clone()).collect();
    errors.extend(validate_tasks(
        &tasks,
        &touched,
        &EditScope::Run,
        limits.max_tasks,
        &profile,
    ));
    if !errors.is_empty() {
        return Err(errors);
    }
    let implicit = implicit_deps(&tasks);
    for (task, deps) in tasks.iter_mut().zip(implicit) {
        task.implicit_deps = deps;
    }
    // Backstop: `implicit_deps` never closes a cycle, and this proves it per plan.
    let cycles = combined_cycles(&tasks);
    if !cycles.is_empty() {
        return Err(cycles);
    }

    let unverified = profile.check.is_none();
    Ok(Run {
        id: ctx.id,
        goal: plan.goal,
        root: pre.root,
        project: pre.project,
        git_common_dir: pre.git_common_dir,
        wt_dir: ctx.wt_dir,
        data_dir: ctx.data_dir,
        base_branch: pre.base_branch,
        run_head: pre.base_sha.clone(),
        base_sha: pre.base_sha,
        last_green_candidate: None,
        base_moved: None,
        state: RunState::AwaitingApproval,
        paused_from: None,
        halted_reason: None,
        approved_by: ctx.yes.then(|| "--yes".to_string()),
        profile,
        limits,
        roster: config.models.clone(),
        tasks,
        merge_queue: Vec::new(),
        outbox: Vec::new(),
        next_message: 1,
        pending_ops: Default::default(),
        next_op: 1,
        windows_created: 0,
        // Decision 47: revisions start at 1. Decision 34: no check, unverified.
        revision: 1,
        unverified,
        final_check_failed: false,
        trusted_project: Vec::new(),
        protected_files: pre.protected_files,
        rate_limits: Default::default(),
        outcome: None,
        log: Vec::new(),
        created_at: ctx.now,
        finish_edit: false,
        finish_reply: None,
        cancelled: false,
        verify_failures: 0,
        halt_retryable: false,
        restored: None,
        session_nonce: 0,
        codex_project_config: None,
    })
}

/// Decision 15's run id: the goal lower-cased, every run of characters other than ASCII
/// letters and digits turned into one `-`, trimmed of `-`, cut to 32 characters (and
/// trimmed again, so it never ends in `-`), `run` if nothing is left, then `-` and the
/// suffix as 4 lowercase hex digits.
pub fn slug(goal: &str, suffix: u16) -> String {
    let mut out = String::new();
    for c in goal.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    // Only ASCII survives, so a character cut is a byte cut.
    out.truncate(SLUG_MAX);
    let head = out.trim_end_matches('-');
    let head = if head.is_empty() { "run" } else { head };
    format!("{head}-{suffix:04x}")
}

/// Decision 15's redraw test, the refs half (M8a.8's carry): whether a branch in `refs`
/// (full names, as `git for-each-ref refs/heads/anthrex/` lists them) takes `id`. The
/// driver also counts `<data_dir>/runs/<id>`.
pub fn run_id_taken(id: &str, refs: &[String]) -> bool {
    let branch = format!("refs/heads/anthrex/{id}");
    let under = format!("{branch}/");
    refs.iter().any(|r| *r == branch || r.starts_with(&under))
}

/// A random 16-bit suffix, drawn from the standard library's per-process random hash
/// keys (no clock, no I/O).
pub fn random_suffix() -> u16 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u8(0);
    hasher.finish() as u16
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
