# Codex Sandbox Through Permission Profiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every Codex session is sandboxed through Codex permission profiles (Linux and macOS), so a Codex worker can `git commit` on Linux, behind a seam that absorbs future Codex sandbox-config changes.

**Architecture:** A pure `SandboxPlan` says what a Codex session may touch; one renderer per Codex sandbox "dialect" (`Legacy`, `Profiles`) turns it into argv; the dialect comes from the Codex version the daemon already probes at startup, through a one-row-per-generation table. The worker grant for Codex follows the host shape (`WholeDir` on Linux) whenever the dialect can express read-only sub-paths.

**Tech Stack:** Rust 2024 workspace (`crates/daemon`, `crates/fake-agent`, `crates/cli` tests), Codex CLI ≥ 0.160 permission profiles.

**Spec:** `docs/superpowers/specs/2026-10-07-codex-linux-worker-git-design.md`

## Global Constraints

- Profiles dialect from Codex `0.160.0`; `Legacy` below; `Profiles` when the version is unknown.
- The profile extends `":read-only"`, never `":workspace"`.
- Never mix `-s` / `sandbox_mode` / `CODEX_SANDBOX_PINS` / `sandbox_workspace_write.*` with `default_permissions` in one argv.
- A renderer that cannot express a plan returns `Err(Unsupported)`; it never drops entries. Nothing is ever silently widened.
- Network stays off for every confined Codex session; `/tmp` and `$TMPDIR` stay unwritable.
- Every TOML string (values and path keys) comes from `launch::codex::toml_string`.
- AGENTS.md hard rules apply: no blocking work under the manager lock; `daemon::lock(&m)`; tests first; files under ~600 lines; no change to process-kill or signal code (`ProbeChild`'s drop stays untouched).
- Tests pin every `*_BIN` to `fake-agent` or a nonexistent path and run under `GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1`. Never execute real `claude` or `codex` binaries in tests.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A path that is both granted and read-only (equal paths): Codex lets `write` beat `read` at equal specificity, so the plan must drop the write entry; pinned in Task 1 (`equal_write_and_read_only_paths_keep_read_only`).
2. Paths with spaces, quotes or backslashes as TOML keys: must be quoted, not broken; pinned in Task 1 (`profile_keys_are_quoted_toml_strings`).
3. A resumed session must carry the same profile as its first turn (a resume that loses the profile would run with Codex's default): pinned in Task 3 (`resume_carries_the_same_profile`).
4. A daemon whose version probe fails or times out must not fall back to `Legacy` (whose worker grant cannot commit on Linux): pinned in Task 1 (`unknown_version_is_profiles`) and Task 2 (`recorded_version_drives_the_detected_dialect`). Superseded after the final review (ruling R6): an unknown version is `Legacy` (`unknown_version_is_legacy`); see the spec's Implementation notes.
5. `Legacy` on Linux must keep the `Files` grant (it cannot express `read_only`): pinned in Task 5 (`legacy_dialect_keeps_the_files_grant`).

---

### Task 1: The sandbox plan, the dialects and their renderers

**Files:**
- Create: `crates/daemon/src/headless/codex_sandbox/mod.rs` (plan, mode, dialect, version table, recorded version)
- Create: `crates/daemon/src/headless/codex_sandbox/legacy.rs` (today's flags)
- Create: `crates/daemon/src/headless/codex_sandbox/profiles.rs` (permission profiles)
- Create: `crates/daemon/src/headless/codex_sandbox/tests.rs`
- Modify: `crates/daemon/src/headless/mod.rs` (add `pub mod codex_sandbox;`)

**Interfaces:**
- Produces:
  ```rust
  pub enum Mode { ReadOnly, Confined, FullAccess }
  impl Mode { pub fn from_codex_sandbox(value: &str) -> Mode } // "workspace-write" → Confined, "danger-full-access" → FullAccess, anything else → ReadOnly
  pub struct SandboxPlan { pub mode: Mode, pub cwd: PathBuf, pub write: Vec<PathBuf>, pub read_only: Vec<PathBuf> }
  impl SandboxPlan { pub fn new(mode: Mode, cwd: PathBuf, write: Vec<PathBuf>, read_only: Vec<PathBuf>) -> SandboxPlan }
  pub enum CodexSandboxDialect { Legacy, Profiles }
  pub const DIALECTS: &[((u64, u64, u64), CodexSandboxDialect)];
  impl CodexSandboxDialect {
      pub fn for_version(version: Option<(u64, u64, u64)>) -> CodexSandboxDialect;
      pub fn expresses_read_only(self) -> bool;
      pub fn render(self, plan: &SandboxPlan, resuming: bool, resume_takes_sandbox: bool) -> Result<Vec<String>, Unsupported>;
  }
  pub struct Unsupported(pub &'static str);
  pub fn record_version(version: (u64, u64, u64)); // first call wins (OnceLock)
  pub fn recorded_version() -> Option<(u64, u64, u64)>;
  pub const PROFILE_NAME: &str = "anthrex";
  ```

- [ ] **Step 1: Write the failing tests** in `codex_sandbox/tests.rs`:

```rust
use super::*;
use std::path::PathBuf;

fn p(s: &str) -> PathBuf { PathBuf::from(s) }

fn confined() -> SandboxPlan {
    SandboxPlan::new(
        Mode::Confined,
        p("/w/task"),
        vec![p("/d/git"), p("/d/tmp")],
        vec![p("/d/git/config"), p("/d/git/refs")],
    )
}

#[test]
fn version_table_picks_the_dialect() {
    assert_eq!(CodexSandboxDialect::for_version(Some((0, 159, 9))), CodexSandboxDialect::Legacy);
    assert_eq!(CodexSandboxDialect::for_version(Some((0, 160, 0))), CodexSandboxDialect::Profiles);
    assert_eq!(CodexSandboxDialect::for_version(Some((1, 0, 0))), CodexSandboxDialect::Profiles);
}

#[test]
fn unknown_version_is_profiles() {
    assert_eq!(CodexSandboxDialect::for_version(None), CodexSandboxDialect::Profiles);
}

#[test]
fn mode_maps_the_configured_values() {
    assert_eq!(Mode::from_codex_sandbox("workspace-write"), Mode::Confined);
    assert_eq!(Mode::from_codex_sandbox("danger-full-access"), Mode::FullAccess);
    assert_eq!(Mode::from_codex_sandbox("read-only"), Mode::ReadOnly);
    assert_eq!(Mode::from_codex_sandbox("bogus"), Mode::ReadOnly);
}

#[test]
fn profiles_render_a_confined_plan() {
    let args = CodexSandboxDialect::Profiles.render(&confined(), false, false).unwrap();
    assert_eq!(
        args,
        [
            "-c", "default_permissions=\"anthrex\"",
            "-c", "permissions.anthrex.extends=\":read-only\"",
            "-c", "permissions.anthrex.network.enabled=false",
            "-c", "permissions.anthrex.filesystem={\"/w/task\"=\"write\",\"/d/git\"=\"write\",\"/d/tmp\"=\"write\",\"/d/git/config\"=\"read\",\"/d/git/refs\"=\"read\"}",
        ]
    );
}

#[test]
fn profiles_render_the_resume_identically() {
    let first = CodexSandboxDialect::Profiles.render(&confined(), false, false).unwrap();
    let resume = CodexSandboxDialect::Profiles.render(&confined(), true, false).unwrap();
    assert_eq!(first, resume);
}

#[test]
fn profiles_render_read_only_and_full_access() {
    let ro = SandboxPlan::new(Mode::ReadOnly, p("/w"), vec![], vec![]);
    assert_eq!(
        CodexSandboxDialect::Profiles.render(&ro, false, false).unwrap(),
        ["-c", "default_permissions=\":read-only\""]
    );
    let full = SandboxPlan::new(Mode::FullAccess, p("/w"), vec![], vec![]);
    assert_eq!(
        CodexSandboxDialect::Profiles.render(&full, false, false).unwrap(),
        ["-c", "default_permissions=\":danger-full-access\""]
    );
}

#[test]
fn profiles_never_mix_legacy_flags() {
    for mode in [Mode::ReadOnly, Mode::Confined, Mode::FullAccess] {
        let plan = SandboxPlan::new(mode, p("/w"), vec![p("/r")], vec![]);
        for resuming in [false, true] {
            let args = CodexSandboxDialect::Profiles.render(&plan, resuming, false).unwrap();
            assert!(!args.iter().any(|a| a == "-s"
                || a.starts_with("sandbox_mode=")
                || a.starts_with("sandbox_workspace_write.")), "{args:?}");
        }
    }
}

#[test]
fn equal_write_and_read_only_paths_keep_read_only() {
    let plan = SandboxPlan::new(Mode::Confined, p("/w"), vec![p("/d/x"), p("/d/y")], vec![p("/d/x")]);
    assert_eq!(plan.write, vec![p("/d/y")]);
    assert_eq!(plan.read_only, vec![p("/d/x")]);
}

#[test]
fn profile_keys_are_quoted_toml_strings() {
    let plan = SandboxPlan::new(Mode::Confined, p("/w/a \"b\"\\c"), vec![], vec![]);
    let args = CodexSandboxDialect::Profiles.render(&plan, false, false).unwrap();
    assert!(args[7].contains(r#"{"/w/a \"b\"\\c"="write"}"#), "{}", args[7]);
}

#[test]
fn legacy_renders_todays_flags() {
    let plan = SandboxPlan::new(Mode::Confined, p("/w"), vec![p("/d/git"), p("/d/tmp")], vec![]);
    assert_eq!(
        CodexSandboxDialect::Legacy.render(&plan, false, false).unwrap(),
        [
            "-s", "workspace-write",
            "-c", "sandbox_workspace_write.network_access=false",
            "-c", "sandbox_workspace_write.exclude_tmpdir_env_var=true",
            "-c", "sandbox_workspace_write.exclude_slash_tmp=true",
            "-c", "sandbox_workspace_write.writable_roots=[\"/d/git\",\"/d/tmp\"]",
        ]
    );
    let resumed = CodexSandboxDialect::Legacy.render(&plan, true, false).unwrap();
    assert_eq!(resumed[..2], ["-c", "sandbox_mode=\"workspace-write\""]);
    let resumed_s = CodexSandboxDialect::Legacy.render(&plan, true, true).unwrap();
    assert_eq!(resumed_s[..2], ["-s", "workspace-write"]);
}

#[test]
fn legacy_refuses_read_only_paths() {
    assert!(CodexSandboxDialect::Legacy.render(&confined(), false, false).is_err());
    assert!(!CodexSandboxDialect::Legacy.expresses_read_only());
    assert!(CodexSandboxDialect::Profiles.expresses_read_only());
}
```


- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p anthrex-daemon --lib headless::codex_sandbox`
Expected: compile failure (module does not exist).

- [ ] **Step 3: Implement.** `mod.rs`:

```rust
//! What a Codex session may touch ([`SandboxPlan`]) and how each generation of Codex's
//! sandbox configuration is told ([`CodexSandboxDialect`]). A new Codex generation is a
//! new variant, its own renderer file and one [`DIALECTS`] row; nothing else moves.

mod legacy;
mod profiles;

use std::path::PathBuf;
use std::sync::OnceLock;

pub const PROFILE_NAME: &str = "anthrex";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode { ReadOnly, Confined, FullAccess }

impl Mode {
    /// `limits.worker_codex_sandbox` and the role constants' values; anything unknown
    /// is the narrowest mode.
    pub fn from_codex_sandbox(value: &str) -> Mode {
        match value {
            "workspace-write" => Mode::Confined,
            "danger-full-access" => Mode::FullAccess,
            _ => Mode::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPlan {
    pub mode: Mode,
    pub cwd: PathBuf,
    /// Writable beside `cwd`.
    pub write: Vec<PathBuf>,
    /// Read-only although inside a writable path.
    pub read_only: Vec<PathBuf>,
}

impl SandboxPlan {
    /// Drops a `write` entry equal to a `read_only` one: at equal specificity Codex lets
    /// write win, and a protected path must never become writable.
    pub fn new(mode: Mode, cwd: PathBuf, write: Vec<PathBuf>, read_only: Vec<PathBuf>) -> SandboxPlan {
        let write = write.into_iter().filter(|w| !read_only.contains(w)).collect();
        SandboxPlan { mode, cwd, write, read_only }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CodexSandboxDialect { Legacy, Profiles }

/// Ascending; the last row whose version is ≤ the CLI's wins.
pub const DIALECTS: &[((u64, u64, u64), CodexSandboxDialect)] = &[
    ((0, 0, 0), CodexSandboxDialect::Legacy),
    ((0, 160, 0), CodexSandboxDialect::Profiles),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub &'static str);

impl CodexSandboxDialect {
    /// `None` (the probe failed or has not answered) is the newest dialect.
    pub fn for_version(version: Option<(u64, u64, u64)>) -> CodexSandboxDialect {
        let Some(version) = version else {
            return DIALECTS[DIALECTS.len() - 1].1;
        };
        DIALECTS.iter().rev().find(|(min, _)| *min <= version).map(|(_, d)| *d)
            .unwrap_or(DIALECTS[0].1)
    }

    pub fn expresses_read_only(self) -> bool {
        matches!(self, CodexSandboxDialect::Profiles)
    }

    pub fn render(self, plan: &SandboxPlan, resuming: bool, resume_takes_sandbox: bool)
        -> Result<Vec<String>, Unsupported> {
        match self {
            CodexSandboxDialect::Legacy => legacy::render(plan, resuming, resume_takes_sandbox),
            CodexSandboxDialect::Profiles => Ok(profiles::render(plan)),
        }
    }
}

static VERSION: OnceLock<(u64, u64, u64)> = OnceLock::new();

/// The startup probe's answer (`lifecycle::codex_version`); the first call wins.
pub fn record_version(version: (u64, u64, u64)) { let _ = VERSION.set(version); }

pub fn recorded_version() -> Option<(u64, u64, u64)> { VERSION.get().copied() }

#[cfg(test)]
mod tests;
```

`legacy.rs` (moves today's logic from `codex_args`; keep `CODEX_SANDBOX_PINS` in `argv.rs` and import it):

```rust
//! Codex before permission profiles: `-s` (or `-c sandbox_mode=` on a resume whose CLI
//! rejects `-s`), the network and tmp pins, and the extra writable roots. It cannot deny
//! a path inside a writable root.

use super::{Mode, SandboxPlan, Unsupported};
use crate::headless::argv::{CODEX_SANDBOX_PINS, toml_array};
use crate::launch::codex::toml_string;

pub(super) fn render(plan: &SandboxPlan, resuming: bool, resume_takes_sandbox: bool)
    -> Result<Vec<String>, Unsupported> {
    if !plan.read_only.is_empty() {
        return Err(Unsupported("legacy Codex sandbox flags cannot make a path read-only"));
    }
    let mode = match plan.mode {
        Mode::ReadOnly => "read-only",
        Mode::Confined => "workspace-write",
        Mode::FullAccess => "danger-full-access",
    };
    let mut args = Vec::new();
    if resuming && !resume_takes_sandbox {
        args.extend(["-c".into(), format!("sandbox_mode={}", toml_string(mode))]);
    } else {
        args.extend(["-s".into(), mode.to_string()]);
    }
    for pin in CODEX_SANDBOX_PINS {
        args.extend(["-c".into(), pin.to_string()]);
    }
    if !plan.write.is_empty() {
        let roots: Vec<String> = plan.write.iter().map(|p| p.display().to_string()).collect();
        args.extend(["-c".into(), format!("sandbox_workspace_write.writable_roots={}", toml_array(&roots))]);
    }
    Ok(args)
}
```

`profiles.rs`:

```rust
//! Codex permission profiles (≥ 0.160): one profile extending `:read-only`, so nothing
//! is writable but what the plan names, and a `read` entry inside a writable path keeps
//! that sub-path read-only. Identical on a first turn and a resume.

use super::{Mode, PROFILE_NAME, SandboxPlan};
use crate::launch::codex::toml_string;

pub(super) fn render(plan: &SandboxPlan) -> Vec<String> {
    let c = |v: String| ["-c".to_string(), v];
    match plan.mode {
        Mode::ReadOnly => c(format!("default_permissions={}", toml_string(":read-only"))).to_vec(),
        Mode::FullAccess => c(format!("default_permissions={}", toml_string(":danger-full-access"))).to_vec(),
        Mode::Confined => {
            let entries: Vec<String> = std::iter::once(&plan.cwd)
                .chain(&plan.write)
                .map(|p| format!("{}={}", toml_string(&p.display().to_string()), toml_string("write")))
                .chain(plan.read_only.iter().map(|p| {
                    format!("{}={}", toml_string(&p.display().to_string()), toml_string("read"))
                }))
                .collect();
            [
                c(format!("default_permissions={}", toml_string(PROFILE_NAME))),
                c(format!("permissions.{PROFILE_NAME}.extends={}", toml_string(":read-only"))),
                c(format!("permissions.{PROFILE_NAME}.network.enabled=false")),
                c(format!("permissions.{PROFILE_NAME}.filesystem={{{}}}", entries.join(","))),
            ]
            .concat()
        }
    }
}
```

Make `toml_array` in `headless/argv.rs` `pub(crate)` if it is not already.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p anthrex-daemon --lib headless::codex_sandbox`
Expected: all 11 pass.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon/src/headless/codex_sandbox crates/daemon/src/headless/mod.rs crates/daemon/src/headless/argv.rs
git commit -m "feat(daemon): Codex sandbox plan with legacy and profile dialects"
```

---

### Task 2: Record the probed version and expose the dialect through `CliCaps`

**Files:**
- Modify: `crates/daemon/src/headless/argv.rs` (`CliCaps`, `CLI_CAPS`)
- Modify: `crates/daemon/src/lifecycle/codex_version.rs` (`report` only)
- Modify: `crates/daemon/src/launch/codex.rs` (`MIN_CODEX_VERSION` comment only; see below)
- Modify: `crates/fake-agent/src/main.rs:55-58`
- Test: `crates/daemon/src/headless/codex_sandbox/tests.rs`, `crates/daemon/src/headless/argv_caps_tests.rs`

**Interfaces:**
- Consumes: Task 1's `CodexSandboxDialect`, `record_version`, `recorded_version`.
- Produces:
  ```rust
  pub enum DialectChoice { Detected, Fixed(CodexSandboxDialect) } // in codex_sandbox/mod.rs
  // CliCaps gains: pub codex_sandbox: DialectChoice   (CLI_CAPS: DialectChoice::Detected)
  impl CliCaps { pub fn codex_dialect(&self) -> CodexSandboxDialect }
  ```

- [ ] **Step 1: Write the failing tests.** In `codex_sandbox/tests.rs`:

```rust
#[test]
fn fixed_choice_ignores_the_recorded_version() {
    assert_eq!(DialectChoice::Fixed(CodexSandboxDialect::Legacy).resolve(), CodexSandboxDialect::Legacy);
}

#[test]
fn recorded_version_drives_the_detected_dialect() {
    // Unit tests never call record_version, so the process has no version: newest dialect.
    assert_eq!(recorded_version(), None);
    assert_eq!(DialectChoice::Detected.resolve(), CodexSandboxDialect::Profiles);
    assert_eq!(DialectChoice::Detected.resolve_with(Some((0, 155, 0))), CodexSandboxDialect::Legacy);
}
```

In `argv_caps_tests.rs`:

```rust
#[test]
fn production_caps_detect_the_codex_dialect() {
    assert_eq!(CLI_CAPS.codex_sandbox, DialectChoice::Detected);
    let legacy = CliCaps { codex_sandbox: DialectChoice::Fixed(CodexSandboxDialect::Legacy), ..CLI_CAPS };
    assert_eq!(legacy.codex_dialect(), CodexSandboxDialect::Legacy);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p anthrex-daemon --lib codex_sandbox argv_caps`
Expected: compile failure (`DialectChoice` missing).

- [ ] **Step 3: Implement.** In `codex_sandbox/mod.rs`:

```rust
/// How a `CliCaps` picks its dialect: production reads the startup probe; tests fix one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialectChoice { Detected, Fixed(CodexSandboxDialect) }

impl DialectChoice {
    pub fn resolve(self) -> CodexSandboxDialect { self.resolve_with(recorded_version()) }
    pub fn resolve_with(self, version: Option<(u64, u64, u64)>) -> CodexSandboxDialect {
        match self {
            DialectChoice::Detected => CodexSandboxDialect::for_version(version),
            DialectChoice::Fixed(dialect) => dialect,
        }
    }
}
```

In `argv.rs`, add to `CliCaps`:

```rust
    /// Which Codex sandbox dialect argv uses (`headless::codex_sandbox`).
    pub codex_sandbox: crate::headless::codex_sandbox::DialectChoice,
```

`CLI_CAPS` gets `codex_sandbox: DialectChoice::Detected,` and `impl CliCaps` gets:

```rust
    pub fn codex_dialect(&self) -> CodexSandboxDialect { self.codex_sandbox.resolve() }
```

Fix every `CliCaps { … }` literal that does not use `..CLI_CAPS` (compiler-guided; the list in the grep is `launch/role_tests.rs`, `scout/tests.rs`, `scout/design_spec_tests.rs`, `decider/tests_argv.rs`, `headless/argv_tests.rs`, `headless/argv_caps_tests.rs`) by adding `codex_sandbox: DialectChoice::Fixed(CodexSandboxDialect::Legacy)` — those tests assert today's flags.

In `lifecycle/codex_version.rs` `report`, record a successful read before the existing match (do not touch `ProbeChild` or `probe`):

```rust
fn report(result: anyhow::Result<(u64, u64, u64)>) {
    if let Ok(version) = &result {
        crate::headless::codex_sandbox::record_version(*version);
    }
    match result {
        Ok(version) if version < MIN_CODEX_VERSION => { /* unchanged */ }
        Ok(version) if crate::headless::codex_sandbox::CodexSandboxDialect::for_version(Some(version))
            == crate::headless::codex_sandbox::CodexSandboxDialect::Legacy => {
            tracing::warn!(?version, "Codex older than 0.160.0: Codex workers cannot commit on Linux; upgrade Codex");
        }
        Ok(_) => {}
        other => tracing::debug!(result = ?other, "could not read Codex version"),
    }
    tracing::debug!("{PROBE_FINISHED}");
}
```

In `fake-agent/src/main.rs`, report a profiles-era version, overridable:

```rust
    if args.iter().any(|arg| arg == "--version") {
        let version = env::var("FAKE_CODEX_VERSION").unwrap_or_else(|_| "0.160.1".into());
        println!("codex-cli {version}");
        return Ok(0);
    }
```

`crates/cli/tests/codex_version.rs` uses its own stub script, so it is unaffected; confirm by running it.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p anthrex-daemon --lib && cargo test -p anthrex --test codex_version`
Expected: PASS (existing argv tests pass because their caps are fixed to `Legacy`; any that use `CLI_CAPS` directly and assert legacy flags now fail — switch them to `CliCaps { codex_sandbox: DialectChoice::Fixed(CodexSandboxDialect::Legacy), ..CLI_CAPS }`, keeping them as legacy regression tests).

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(daemon): pick the Codex sandbox dialect from the probed version"
```

---

### Task 3: Headless Codex sessions render through the plan

**Files:**
- Modify: `crates/daemon/src/headless/mod.rs` (`HeadlessSpec.codex_read_only`)
- Modify: `crates/daemon/src/headless/argv.rs` (`codex_args`)
- Modify: every `HeadlessSpec { … }` literal: `scout/spec.rs:251`, `manager/role_window.rs:113`, `run/role_launch.rs:234` and `:316`, and test literals (compiler-guided)
- Test: `crates/daemon/src/headless/argv_tests.rs` (or a new `argv_sandbox_tests.rs` if `argv_tests.rs` would pass 600 lines)

**Interfaces:**
- Consumes: Task 1 `SandboxPlan::new`, `Mode::from_codex_sandbox`, `render`; Task 2 `CliCaps::codex_dialect`.
- Produces: `HeadlessSpec.codex_read_only: Vec<PathBuf>` (`#[serde(default)]`), filled by Task 5. `pub fn codex_plan(spec: &HeadlessSpec) -> SandboxPlan` in `argv.rs`.

- [ ] **Step 1: Write the failing tests** (Profiles caps fixed explicitly):

```rust
fn profiles() -> CliCaps {
    CliCaps { codex_sandbox: DialectChoice::Fixed(CodexSandboxDialect::Profiles), ..CLI_CAPS }
}

fn worker_spec() -> HeadlessSpec {
    let mut spec = codex_spec(); // the file's existing Codex spec helper
    spec.codex_sandbox = "workspace-write".into();
    spec.codex_writable_roots = vec!["/d/git".into(), "/d/tmp".into()];
    spec.codex_read_only = vec!["/d/git/config".into()];
    spec
}

#[test]
fn a_worker_gets_its_profile_with_read_only_entries() {
    let args = codex_args(&worker_spec(), &SessionArg::New { .. /* as existing tests */ }, "m", exe(), 1, sock(), &profiles());
    let fs = args.iter().find(|a| a.starts_with("permissions.anthrex.filesystem=")).unwrap();
    assert!(fs.contains("\"/d/git\"=\"write\""), "{fs}");
    assert!(fs.contains("\"/d/git/config\"=\"read\""), "{fs}");
    assert!(args.iter().any(|a| a == "default_permissions=\"anthrex\""));
    assert!(!args.iter().any(|a| a == "-s" || a.starts_with("sandbox_")), "{args:?}");
}

#[test]
fn resume_carries_the_same_profile() {
    let spec = worker_spec();
    let first = codex_args(&spec, &new_session(), "m", exe(), 1, sock(), &profiles());
    let resume = codex_args(&spec, &SessionArg::Resume { session_id: "s".into() }, "m", exe(), 1, sock(), &profiles());
    let pick = |a: &[String]| a.iter().filter(|x| x.starts_with("default_permissions=") || x.starts_with("permissions.")).cloned().collect::<Vec<_>>();
    assert_eq!(pick(&first), pick(&resume));
    assert!(!pick(&first).is_empty());
}

#[test]
fn a_read_only_role_gets_the_builtin_read_only_profile() {
    let mut spec = codex_spec();
    spec.codex_sandbox = "read-only".into();
    let args = codex_args(&spec, &new_session(), "m", exe(), 1, sock(), &profiles());
    assert!(args.iter().any(|a| a == "default_permissions=\":read-only\""), "{args:?}");
}
```

(Use the helper names `argv_tests.rs` already has for a session, exe and socket; the placeholders `new_session()`, `exe()`, `sock()` stand for them.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p anthrex-daemon --lib headless::argv`
Expected: compile failure (`codex_read_only` missing).

- [ ] **Step 3: Implement.** Add to `HeadlessSpec` after `codex_writable_roots`:

```rust
    /// Read-only although inside a writable root (a worker's denied git entries); only
    /// a dialect that can express it receives it (`headless::codex_sandbox`).
    #[serde(default)]
    pub codex_read_only: Vec<PathBuf>,
```

In `argv.rs`:

```rust
pub fn codex_plan(spec: &HeadlessSpec) -> SandboxPlan {
    SandboxPlan::new(
        Mode::from_codex_sandbox(&spec.codex_sandbox),
        spec.cwd.clone(),
        spec.codex_writable_roots.clone(),
        spec.codex_read_only.clone(),
    )
}
```

Replace the block in `codex_args` from `if resuming && !caps.codex_resume_takes_sandbox {` through the `writable_roots` block with:

```rust
    match caps
        .codex_dialect()
        .render(&codex_plan(spec), resuming, caps.codex_resume_takes_sandbox)
    {
        Ok(sandbox) => args.extend(sandbox),
        // Task 5 never hands a dialect a plan it cannot express; if one arrives, the
        // narrowest built-in mode runs rather than a widened one.
        Err(unsupported) => {
            tracing::error!(reason = unsupported.0, "Codex sandbox plan not expressible; read-only");
            let narrow = SandboxPlan::new(Mode::ReadOnly, spec.cwd.clone(), vec![], vec![]);
            args.extend(caps.codex_dialect().render(&narrow, resuming, caps.codex_resume_takes_sandbox)
                .expect("every dialect expresses read-only"));
        }
    }
```

Update the doc comment above `codex_args` to say the sandbox comes from `codex_sandbox`. Add `codex_read_only: Vec::new(),` to every `HeadlessSpec` literal.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p anthrex-daemon --lib headless:: run::role_launch scout::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(daemon): headless Codex sessions take their sandbox from the plan"
```

---

### Task 4: Deciders and the interactive orchestrator render through the plan

**Files:**
- Modify: `crates/daemon/src/decider/argv.rs:150-170` (`codex_decider_args`)
- Modify: `crates/daemon/src/launch/role.rs:143-145` (`codex_role_args`)
- Test: `crates/daemon/src/decider/tests_argv.rs`, `crates/daemon/src/launch/role_tests.rs`

**Interfaces:**
- Consumes: Task 1 `SandboxPlan`, `Mode::ReadOnly`, `render`; Task 2 `CliCaps::codex_dialect`.

- [ ] **Step 1: Write the failing tests.** In `decider/tests_argv.rs`:

```rust
#[test]
fn a_codex_decider_on_profiles_is_read_only_without_legacy_flags() {
    let caps = CliCaps { codex_sandbox: DialectChoice::Fixed(CodexSandboxDialect::Profiles), ..CLI_CAPS };
    let args = codex_decider_args(&ctx(Runtime::Codex, "m", caps), &DECIDER_CAPS, Path::new("/s.json"), "p");
    assert!(args.iter().any(|a| a == "default_permissions=\":read-only\""), "{args:?}");
    assert!(!args.iter().any(|a| a == "-s" || a.starts_with("sandbox_")), "{args:?}");
}
```

In `launch/role_tests.rs` (same shape, using that file's existing `codex_role_args` fixture):

```rust
#[test]
fn a_codex_orchestrator_on_profiles_is_read_only_and_asks() {
    let caps = CliCaps { codex_sandbox: DialectChoice::Fixed(CodexSandboxDialect::Profiles), ..CLI_CAPS };
    let args = codex_role_args(&role(), &ctx(), &caps);
    assert!(args.iter().any(|a| a == "default_permissions=\":read-only\""), "{args:?}");
    assert!(args.windows(2).any(|w| w == ["-a", "on-request"]), "{args:?}");
    assert!(!args.iter().any(|a| a == "-s"), "{args:?}");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p anthrex-daemon --lib decider:: launch::role`
Expected: FAIL (`-s` present, no `default_permissions`).

- [ ] **Step 3: Implement.** In `codex_decider_args`, replace `args.extend(["-s"…]); for pin in CODEX_SANDBOX_PINS {…}` with:

```rust
    let plan = SandboxPlan::new(Mode::ReadOnly, PathBuf::new(), vec![], vec![]);
    args.extend(ctx.caps.codex_dialect().render(&plan, false, true)
        .expect("every dialect expresses read-only"));
```

In `codex_role_args`, replace `args.extend(["-s", "read-only", "-a", "on-request"].map(String::from));` with:

```rust
    let plan = SandboxPlan::new(Mode::ReadOnly, PathBuf::new(), vec![], vec![]);
    args.extend(caps.codex_dialect().render(&plan, false, true)
        .expect("every dialect expresses read-only"));
    args.extend(["-a", "on-request"].map(String::from));
```

Note for the Implementation notes: under `Legacy` the interactive orchestrator now also gets `CODEX_SANDBOX_PINS` (harmless keys for `read-only`); update that file's legacy expectation accordingly.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p anthrex-daemon --lib decider:: launch::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(daemon): Codex deciders and orchestrators take their sandbox from the plan"
```

---

### Task 5: A Codex worker's grant follows the host shape when its dialect allows

**Files:**
- Modify: `crates/daemon/src/run/driver/ops.rs:162-205`
- Test: `crates/daemon/src/run/driver/ops_tests.rs` (or the file that tests this launch-completion function today; find it with `grep -rn "worker_git_grant\|GrantShape" crates/daemon/src/run/driver`)
- Test: `crates/cli/tests/run_e2e_worker_pins.rs`

**Interfaces:**
- Consumes: Task 2 `CliCaps::codex_dialect`, `CodexSandboxDialect::expresses_read_only`; Task 3 `HeadlessSpec.codex_read_only`. `ctx` already carries `cli_caps` (`run/driver/context.rs:32`).
- Produces: a pure helper `pub(super) fn grant_shape(claude: bool, dialect: CodexSandboxDialect) -> git::GrantShape`.

- [ ] **Step 1: Write the failing unit tests:**

```rust
#[test]
fn a_codex_worker_on_profiles_gets_the_host_shape() {
    assert_eq!(grant_shape(false, CodexSandboxDialect::Profiles), git::GrantShape::host());
    assert_eq!(grant_shape(true, CodexSandboxDialect::Legacy), git::GrantShape::host());
}

#[test]
fn legacy_dialect_keeps_the_files_grant() {
    assert_eq!(grant_shape(false, CodexSandboxDialect::Legacy), git::GrantShape::Files);
}
```

(`GrantShape` must derive `PartialEq, Debug`; add them if missing.)

Update the e2e test's Codex half (`run_e2e_worker_pins.rs`, after `// Codex:`), replacing the legacy-pin assertions:

```rust
    // Codex (fake-agent reports 0.160.1, so the profiles dialect): network off, no
    // legacy flags, the task's own temporary directory writable, and on Linux the same
    // git entries read-only as Claude's.
    let argv: Vec<String> = serde_json::from_str(&h.io_lines("worker-t2-1", "args")[0]).unwrap();
    assert!(argv.iter().any(|a| a == "default_permissions=\"anthrex\""), "{argv:?}");
    assert!(argv.iter().any(|a| a == "permissions.anthrex.network.enabled=false"), "{argv:?}");
    assert!(!argv.iter().any(|a| a == "-s" || a.starts_with("sandbox_")), "{argv:?}");
    let fs = argv.iter().find(|a| a.starts_with("permissions.anthrex.filesystem=")).unwrap();
    assert!(fs.contains(&format!("{:?}=\"write\"", tmps[1])), "{fs}");
    for tail in git_denied {
        assert!(fs.contains(&format!("{tail}\"=\"read\"")), "{tail}: {fs}");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p anthrex-daemon --lib run::driver && cargo test -p anthrex --test run_e2e_worker_pins`
Expected: unit tests fail to compile (`grant_shape` missing); e2e fails (on Linux no `read` entries; on macOS it passes the profile checks after Task 3 already, which is fine).

- [ ] **Step 3: Implement.** In `ops.rs`:

```rust
/// A worker's grant shape: the host's (on Linux the git directory whole, its protected
/// entries read-only) for Claude, and for Codex when its sandbox dialect can make a
/// path read-only inside a writable one; the exact files otherwise.
pub(super) fn grant_shape(claude: bool, dialect: CodexSandboxDialect) -> git::GrantShape {
    if claude || dialect.expresses_read_only() {
        git::GrantShape::host()
    } else {
        git::GrantShape::Files
    }
}
```

Replace the comment and `let shape = if claude {…}` with `let shape = grant_shape(claude, ctx.cli_caps.codex_dialect());` (use whatever path to the caps `ctx` exposes), and replace the final Codex block with:

```rust
    if !spec.codex_writable_roots.is_empty() {
        spec.codex_writable_roots = grant.writable;
        spec.codex_read_only = grant.deny;
    }
```

(Order matters: `grant.deny` is moved after Claude's `extend(grant.deny.iter().cloned())`; a spec is one runtime, so both branches never fire together.)

- [ ] **Step 4: Run the tests**

Run: `cargo test -p anthrex-daemon --lib run:: && cargo test -p anthrex --test run_e2e_worker_pins --test run_e2e_pair --test run_e2e_patterns`
Expected: PASS. Also run the Linux target: `ssh nexus1 'bash -lc "cd ~/repos/anthrex && git fetch -q && git checkout -q fix/codex-linux-git && GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 cargo test -p anthrex --test run_e2e_worker_pins"'` (push the branch first; never touch nexus1's live daemon).

- [ ] **Step 5: Commit**

```bash
git commit -am "fix(daemon): Codex workers get the whole git directory with protected entries read-only"
```

---

### Task 6: Docs, changelog and manual acceptance

**Files:**
- Modify: `CHANGELOG.md` (Unreleased: "Codex workers can commit on Linux; every Codex session is sandboxed through permission profiles on Codex ≥ 0.160")
- Modify: `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` (mark the 2026-10-06 Codex-on-Linux note resolved, pointing at this spec)
- Modify: `docs/superpowers/specs/2026-10-07-codex-linux-worker-git-design.md` (add "Implementation notes": deviations, the orchestrator legacy-pins note from Task 4, manual results)
- Modify: `docs/install.md` (Codex ≥ 0.160 recommended; older Codex runs but its workers cannot commit on Linux)

- [ ] **Step 1: Full local verification**

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo +1.99 clippy --workspace --all-targets -- -D warnings
cargo +1.99 clippy --workspace --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Expected: all pass.

- [ ] **Step 2: Manual acceptance on nexus1** (approved real-agent run; isolated): build the branch there, then with `ANTHREX_SOCKET=/tmp/ax-cx/s.sock ANTHREX_DATA_DIR=/tmp/ax-cx/data` start a one-task plan run in a scratch repo under `/tmp/ax-cx/repo` whose implementer route is `runtime="codex"`. Expected: the worker commits and reaches `task_done`; afterwards the task checkout's `git/config` equals the engine's and `git/refs` is unchanged. End with `anthrex daemon stop` (same variables) and `pgrep -fl "anthrex daemon"` showing nothing of ours. Record the result in the spec's Implementation notes.

- [ ] **Step 3: Manual acceptance on macOS** — only after the user approves this real-agent run in chat. Same procedure on this Mac under `/tmp/ax-cx`. If Codex refuses a `write` entry for a path that does not exist yet (`index.lock` under the `Files` shape), stop and report to the user: the remedy (Codex on macOS also takes `WholeDir`) changes the spec.

- [ ] **Step 4: Commit and open the PR**

```bash
git commit -am "docs: Codex permission-profile sandboxing notes and changelog"
git push -u origin fix/codex-linux-git
gh pr create --title "fix: Codex workers commit on Linux via permission profiles" --body-file <scratchpad>/pr-codex.md
```

The PR body lists every task with its status, the verification output, both manual results, and ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Do not merge.
