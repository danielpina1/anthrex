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
  is `Profiles` for ≥ 0.160.0, `Legacy` below, and `Legacy` when the version is unknown (the
  probe failed or timed out; amended after final review, see Implementation notes). Every
  path that resolves the dialect first waits for the daemon's launch gate (the probe).
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

- An unknown Codex version is `Legacy` (amended after final review): a profile on a Codex too old
  to read it carries no `-s`, so the user's own `sandbox_mode` would apply. A session's grant
  records the dialect it was computed for (`HeadlessSpec.codex_grant_dialect`), and a confined
  session always renders in that dialect, never in a newer one that would drop its protection.
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
  `read_only`; the version table picks `Legacy` for 0.159.x and for an unknown version, and
  `Profiles` for 0.160.0.
- Manual acceptance on this Mac, same isolation, one small Codex worker run that commits, with
  the user's approval for that real-agent run.
- Manual acceptance on nexus1 (isolated `ANTHREX_SOCKET`/`ANTHREX_DATA_DIR` under `/tmp/ax-*`): one
  small run with a Codex implementer commits and reaches `task_done`; afterwards the checkout's
  `config` matches the engine's and `refs/` is untouched. Linux CI has no sandbox, so this check is
  the only end-to-end proof and is recorded in the PR.

## 6. Out of scope

- Moving macOS workers (either runtime) to `WholeDir`.

## Implementation notes

Deviations from the design above, recorded by the implementation.

- A Codex worker's read-only list also includes the worktree `.git` and the protected
  agent-config paths (`.claude`, `.codex`, `.mcp.json`, `AGENTS.md`, `CLAUDE.md`, minus
  owned literals) that exist at launch. Legacy `workspace-write` protected `.git` and
  `.codex` implicitly; a profile does not (ruling R3). Section 5's "5 protected paths"
  expectation therefore holds only for paths that exist; the e2e fixture has none.
- Under Legacy, the interactive Codex orchestrator now also receives the three
  `CODEX_SANDBOX_PINS` after `-s read-only` (harmless for read-only).
- Unknown or empty `codex_sandbox` values map to read-only.
- The worker grant waits for the Codex version probe (the launch gate) before choosing
  the dialect. The session's working directory is canonicalized once, off the lock, for
  the read-only entries (macOS `/tmp` is `/private/tmp`), so they spell the checkout as
  the grant's canonical entries do. The protected tails are never canonicalized (a worker
  could swap one for a link). The plan's own `"write"` entry for the working directory
  keeps `spec.cwd` as given: `spec.cwd` is also the session's process directory, and
  nothing else needed it changed.
- New tests live in `argv_sandbox_tests.rs` and `worker_grant.rs` / `worker_grant_tests.rs`
  because `argv_tests.rs` and `ops.rs` were at the 600-line limit.
- Unknown version → `Legacy` (amended after final review, ruling R6). The design said an
  unknown version was `Profiles`; on Codex < 0.160 a profile argv has no `-s`, so a probe that
  failed or timed out would have left the user's own `sandbox_mode` in charge. `for_version(None)`
  is now `Legacy`, and every production path that resolves a Codex dialect waits for the launch
  gate first: the worker grant, `create_headless`, the window launch and restart, the decider
  call, and a headless send or resume (a window restored at start can be resumed while the probe
  runs). None holds a lock across the wait.
- A grant records its dialect (ruling R7): `HeadlessSpec.codex_grant_dialect`, set by the worker
  grant; `None` (a spec persisted by an earlier daemon, or never granted) means `Legacy`. A
  confined plan renders in that dialect, not the CLI's current one, so a restored or resumed
  spec whose grant had no read-only entries keeps Legacy's implicit `.git`/`.codex` protection,
  and the grant and its argv can never disagree. Read-only and full-access plans hold no grant
  and render in the current (gate-awaited) dialect. Every confined plan uses the grant's
  dialect, not only those with writable roots: a confined spec that never went through the
  grant has always been rendered Legacy.
- Read-only entries are fixed at launch: a protected path created later in the session (an
  `AGENTS.md` the worker writes, say) stays writable for that session. `.codex` is still checked
  by `codex_config_guard` before each process; `AGENTS.md` and `CLAUDE.md` were never protected
  for Codex before either.
- When a dialect cannot express a plan, §4.2 said the caller falls back to the `Files` grant. The
  grant side does exactly that (Legacy gets `Files` and no read-only entries); the argv-level
  backstop in `codex_args`, which should never fire, narrows further and renders a read-only
  session rather than a widened one.
- `SandboxPlan::new` also drops a `write` entry equal to the working directory or to an earlier
  `write` entry: each profile path is a key of one inline TOML table, where a repeated key is
  invalid.
- Manual acceptance: recorded by the controller (nexus1, macOS)
