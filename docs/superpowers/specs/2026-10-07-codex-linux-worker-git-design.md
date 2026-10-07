# Codex sandbox through permission profiles: design

Date: 2026-10-07. Status: approved in conversation, awaiting spec review.

## 1. Goal

A Codex worker in an orchestrated run on Linux can `git commit` in its task checkout, exactly as a
Claude worker can since #45. Any model on either runtime can then do development work in a run.
Every Codex session, on Linux and macOS, is sandboxed through Codex permission profiles, behind a
seam that absorbs future changes to Codex's sandbox configuration.

Success: on nexus1 (Ubuntu, Codex 0.160.1, bubblewrap) a run whose implementer is Codex reaches
`task_done` with its own commit, and the checkout's protected git entries are unchanged.

## 2. The problem

A Codex worker always gets `GrantShape::Files` (`crates/daemon/src/run/driver/ops.rs:175-202`),
because Codex's legacy `sandbox_workspace_write.writable_roots` cannot deny a path inside a writable
root. `Files` names each git file and its `.lock`. Codex's Linux sandbox is bubblewrap, which binds
only paths that exist, so `index.lock` and `HEAD.lock` (created by git with `O_EXCL`) are never
writable and the commit fails. Granting the whole git directory through `writable_roots` would let
the worker rewrite the checkout's `config`, which every unsandboxed engine git call reads.

## 3. Evidence: Codex permission profiles deny inside a writable root

Codex ≥ 0.160 has permission profiles (`default_permissions` plus `[permissions.<name>]`); a more
specific `read` or `deny` entry overrides a broader `write`. Probe on nexus1, 2026-10-07, one
`codex exec` (3.5k tokens) with:

```
-c 'default_permissions="axprobe"'
-c 'permissions.axprobe.extends=":read-only"'
-c 'permissions.axprobe.filesystem={"/tmp/ax-probe/w"="write","/tmp/ax-probe/w/config"="read","/tmp/ax-probe/w/refs"="read"}'
```

| Operation inside the sandbox | Result |
|---|---|
| create `index.lock` (`O_EXCL`), rename over `index` | allowed |
| create `HEAD.lock`, rename over `HEAD` | allowed |
| new `COMMIT_EDITMSG`, new `objects/ab/cd` | allowed |
| append to / replace by rename / unlink `config` | blocked |
| create in `refs/`, rename `refs/` | blocked |
| write outside the granted directory | blocked |

The host saw `config` unchanged afterwards. The unsandboxed control run allowed every operation.

## 4. Design

### 4.1 Grant shape

`ops.rs` stops forcing Codex onto `Files`: a Codex worker gets `GrantShape::host()` like a Claude
worker (`WholeDir` on Linux, `Files` elsewhere). `WorkerGrant { writable, deny }` is unchanged, and
so are the placeholders that make every denied entry exist before launch and `put_denied`.
When the detected dialect (4.2) is `Legacy`, a Codex worker keeps `Files`, as today.
The deny list is no longer dropped for Codex: `HeadlessSpec` gains `codex_read_only: Vec<PathBuf>`,
filled from `grant.deny` beside `codex_writable_roots`.

### 4.2 One sandbox plan, rendered per Codex generation

Codex's sandbox configuration has already changed shape once (`-s` plus `sandbox_workspace_write`,
then permission profiles) and will change again. The design separates *what* a session may touch
from *how* a given Codex version is told:

- `headless::codex_sandbox::SandboxPlan` (new, pure): `{ mode: Mode, network: bool, write: Vec<PathBuf>,
  read_only: Vec<PathBuf> }` with `Mode::{ReadOnly, Confined, FullAccess}`. Built once per spec
  from `codex_sandbox`, the cwd, `codex_writable_roots` and `codex_read_only`. It knows nothing of
  flags.
- `CodexSandboxDialect` (enum, one variant per Codex generation): `Legacy` (today's `-s`,
  `sandbox_mode`, `CODEX_SANDBOX_PINS`, `writable_roots`; cannot express `read_only`) and
  `Profiles` (≥ 0.160). Each variant has one `render(&SandboxPlan, resuming) -> Result<Vec<String>,
  Unsupported>` in its own file. A future Codex change is a new variant plus its file and tests;
  nothing else moves.
- `render` refuses a plan it cannot express faithfully (`Legacy` with a non-empty `read_only`)
  instead of dropping entries; the caller then falls back to the `Files` grant, which `Legacy`
  can express. Nothing is ever silently widened.
- The dialect is chosen from the Codex version the daemon already probes at startup
  (`lifecycle/codex_version.rs`), now recorded instead of only logged: `CliCaps.codex_sandbox_dialect`
  is `Profiles` for ≥ 0.160.0, `Legacy` below, and `Profiles` when the version is unknown (the
  newest dialect; old Codex is the exception to support, not the default).
  A table `DIALECTS: &[(min_version, CodexSandboxDialect)]` holds the mapping, so adding a
  generation is one row.

The `Profiles` rendering, on a first turn and on a resume alike:

```
-c default_permissions="anthrex"
-c permissions.anthrex.extends=":read-only"
-c permissions.anthrex.network.enabled=false
-c permissions.anthrex.filesystem={"<cwd>"="write","<root>"="write",…,"<denied>"="read",…}
```

- `<cwd>` is the session's working directory; `<root>` each `write` path; `<denied>` each
  `read_only` path. Every path is a TOML string from `launch::codex::toml_string`.
- The profile extends `:read-only`, not `:workspace`: `:workspace` re-mounts resolved `gitdir:`
  targets read-only, which would undo the grant.
- `-s`, `sandbox_mode`, `CODEX_SANDBOX_PINS` and `writable_roots` are never mixed in: Codex rejects
  combining the two configurations. `/tmp` and `$TMPDIR` stay unwritable because `:read-only`
  grants neither; network stays off by the explicit key.
- `Mode::ReadOnly` renders `default_permissions=":read-only"`; `Mode::FullAccess` renders
  `":danger-full-access"`. `limits.worker_codex_sandbox` maps `read-only` / `workspace-write` /
  `danger-full-access` onto the three modes, as today.
- Every Codex role goes through the plan: workers, racers and test writers (`Confined`) and
  reviewers, scouts, planners, deciders and the orchestrator (`ReadOnly`, or whatever they use
  today). So on a current Codex no session uses the legacy flags.

### 4.3 Failing closed

- `MIN_CODEX_VERSION` stays the floor for launching at all; below 0.160 the `Legacy` dialect keeps
  today's behaviour (`Files` grant, commits fail on Linux as now), and the startup warning names
  0.160 as the version that enables Codex worker commits on Linux.
- A Codex that rejects a profile fails the launch, visibly; Codex refuses split policies it cannot
  enforce rather than running unconfined (its documented behaviour).
- The engine's own guard is unchanged and remains the second line: `pinned::check` verifies the
  task `config` byte for byte, with a single link, before every engine git call.

### 4.4 macOS

macOS Codex sessions use the same `Profiles` dialect. The grant shape stays `GrantShape::host()`
(`Files` on macOS, as for Claude), so the profile lists the exact files and `.lock` names the
legacy flags list today. Seatbelt, unlike bubblewrap, can grant a path that does not exist yet;
the manual macOS acceptance in §5 confirms it under profiles before merge.

## 5. Testing

- `argv.rs` unit tests: a worker spec on Linux shape yields the profile flags (first turn and
  resume), no `-s`, no `sandbox_mode`, no legacy pins; each `worker_codex_sandbox` value maps as in
  4.2; a reviewer spec is byte-identical to today.
- `ops.rs`: a Codex worker on Linux gets `WholeDir`, and its `codex_read_only` equals the grant's deny.
- `crates/cli/tests/run_e2e_worker_pins.rs`: Codex rows on Linux expect the 5 protected paths plus
  the 10 git-denied entries, as Claude's do.
- `codex_sandbox` unit tests: each dialect renders each mode; `Legacy` refuses a plan with
  `read_only`; the version table picks `Legacy` for 0.159.x, `Profiles` for 0.160.0 and for an
  unknown version.
- Manual acceptance on this Mac, same isolation, one small Codex worker run that commits, with
  the user's approval for that real-agent run.
- Manual acceptance on nexus1 (isolated `ANTHREX_SOCKET`/`ANTHREX_DATA_DIR` under `/tmp/ax-*`): one
  small run with a Codex implementer commits and reaches `task_done`; afterwards the checkout's
  `config` matches the engine's and `refs/` is untouched. Linux CI has no sandbox, so this check is
  the only end-to-end proof and is recorded in the PR.

## 6. Out of scope

- Moving macOS workers (either runtime) to `WholeDir`.
