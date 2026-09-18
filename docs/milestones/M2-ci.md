# M2: Continuous integration

## Header

- **Status**: ready
- **Depends on**: milestone 1 (done)
- **Spec sections**: core spec (`docs/superpowers/specs/2026-09-17-anthrex-design.md`) §8, last bullet: "CI (GitHub Actions): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` on macOS and Ubuntu." The product design spec (`docs/superpowers/specs/2026-09-18-anthrex-product-design.md`) says nothing about CI beyond `docs/ROADMAP.md`'s "Why this order" reasoning ("CI first. Every later milestone is implemented by an agent. Automated checks on every pull request catch regressions before a human reviews them."). Everything else in this brief is the controller's own decision, recorded below.
- **Branch**: `m2-ci`
- **Protocol version**: unchanged. `proto::PROTO_VERSION` stays `1`. This milestone touches no Rust type, message, or CLI command. (Product spec 10.1: a milestone that changes a message shape sets `PROTO_VERSION` to one more than the value on `main` when it starts; one that changes none keeps it.)

## Goal

Every push to `main` and every pull request runs the same checks a human runs locally today: format, lint, build, test, and the PTY smoke script. They run on both macOS and Ubuntu, automatically. A broken build or a regressed test shows up as a red check on the pull request before anyone reviews the diff, and `README.md` shows the build's live status at a glance.

## Scope

In:

- A GitHub Actions workflow, `.github/workflows/ci.yml`, that runs the five commands `AGENTS.md` already requires, on both `macos-latest` and `ubuntu-latest`, on every push to `main` and every pull request.
- Reformatting the existing codebase so `cargo fmt --all --check` passes, since it does not today (verified locally: see Design decision 9). This is a dedicated, format-only commit with no behavioural change.
- Confirming that the two tests which rely on a real PTY write stalling under `stty raw -echo` (one Rust test, one smoke-script stage) actually exercise that stall on `ubuntu-latest`, and making them fail safe, skip with a clear message rather than pass vacuously, if they cannot.
- A CI status badge at the top of `README.md`.

Out:

- Any change to daemon, TUI, CLI, or wire-protocol behaviour. No new Rust type, message, or CLI flag.
- Branch protection settings on GitHub. That is a manual, one-time setting in the repository's web UI; see "Manual check".
- A `rust-toolchain.toml` file (Design decision 6).
- Anything not in the five verification commands: no code coverage, no release builds, no artifact publishing, no second workflow file.
- Any item in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`. Its "Assignment to milestones" table assigns this milestone nothing from the ledger; see "Follow-ups handled".

## Design decisions

1. The workflow lives at `.github/workflows/ci.yml`, one workflow named `CI`, one job `ci`, matrixed over `os: [ubuntu-latest, macos-latest]`. It triggers on `push` to `main` and on `pull_request` with no branch filter, so it runs for a pull request targeting any branch.
2. `strategy.fail-fast: false`. Each OS's result is independent information (a macOS-only or Ubuntu-only regression is still useful to see), so one leg failing must not cancel the other.
3. `timeout-minutes: 20` at the job level. `env.RUST_BACKTRACE: "1"` at the workflow level, so a panic inside a daemon or PTY thread during `cargo test` or the smoke script prints a backtrace in the log instead of a bare message.
4. Toolchain: `dtolnay/rust-toolchain@stable` with `components: rustfmt, clippy`. This installs the newest stable release, not a pinned version. Verified locally: the current stable toolchain is `1.92.0` (`rustc 1.92.0 (ded5c06cf 2025-12-08)`), which already satisfies the workspace's declared `rust-version = "1.92"` (`Cargo.toml`, `[workspace.package]`). "Stable" will keep satisfying that floor as Rust releases new stable versions on its normal cadence.
5. Cache: `Swatinem/rust-cache@v2`, one call per job, with its defaults (keys on `Cargo.lock` and the job's OS and Rust version; caches the cargo registry, git checkouts, and `target/`). No extra configuration is needed for a workspace this size.
6. **No `rust-toolchain.toml` is added.** `cargo` already enforces the `Cargo.toml` `rust-version` field itself: verified locally that `cargo build` against a manifest with a `rust-version` newer than the installed toolchain fails with `error: rustc 1.92.0 is not supported by the following package: ... requires rustc 1.150`, with no extra tooling. Adding a `rust-toolchain.toml` that also pins a version would duplicate that floor in a second file that can drift from `Cargo.toml`'s `rust-version`, for no added safety, since `dtolnay/rust-toolchain@stable` and `cargo`'s own check together already guarantee an adequate compiler.
7. Steps, in order, one shell command per step so a failure names exactly which check broke: checkout, install toolchain, restore cache, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace --all-targets`, `cargo test --workspace`, `python3 scripts/pty-smoke.py`. This is the exact order and exact command AGENTS.md lists, plus the smoke script last, since the smoke script's own `ensure_binary()` reuses the `target/debug/anthrex` the build step already produced, and since a broken build or a failing unit test should be reported before the slower, five-command smoke run even starts.
8. Action versions pinned as of 2026-09-18: `actions/checkout@v4`, `Swatinem/rust-cache@v2`, `dtolnay/rust-toolchain@stable` (this action is versioned by branch name, not semver; `@stable` always tracks the newest stable Rust release, which is the intended behaviour here, not a moving-target risk, see Design decision 4). No other action is added.
9. **The reformat is real and is its own commit.** Verified locally, `cargo fmt --all --check` (rustc/cargo 1.92.0, no `rustfmt.toml` or `.rustfmt.toml` in the repository, so default rustfmt style applies) fails across 26 files in all four crates, every file that has a line over rustfmt's default 100-column width gets rewrapped. `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace --all-targets`, and `cargo test --workspace` all pass today; only formatting is broken. Task M2.1 runs `cargo fmt --all` once, as a dedicated commit with no other change, before the workflow is added, so the first real CI run in M2.2's pull request starts green on the fmt step instead of failing on day one.
10. **The stall-dependent tests must prove the stall, not assume it.** `crates/daemon/tests/manager.rs::a_program_that_ignores_stdin_never_blocks_write_input_or_list` and stage 8 of `scripts/pty-smoke.py` both depend on a real PTY-master write blocking under `stty raw -echo; sleep 5`. `crates/daemon/src/window.rs`'s own doc comment (line 25) says this stalls "after about a kilobyte on macOS" and says nothing about Linux; the kernel's pty input-buffer limit is not the same on every platform, and Ubuntu's `ubuntu-latest` GitHub runner has never been checked against this codebase. If the stall does not actually happen on `ubuntu-latest`, both the Rust test and the smoke-script stage would still "pass" without exercising the regression they exist to catch (a program that ignores stdin must never block `write_input` or `list`; the freeze it guards against is milestone 1's worst bug, per `AGENTS.md`). Task M2.4 and M2.5 add an explicit, fast probe that detects whether the stall actually happened and, only when it did not, print a clear message and skip the timing assertions instead of asserting something that was never tested. Everything else about `TMPDIR`, `/bin/zsh`, `SHELL`, and `pgrep` was checked and found not to need a fix; see "Risks and gotchas" item 4 for the evidence.
11. The badge is the first line of `README.md`, before the `# anthrex` heading has any body text after it, so it renders directly under the title on GitHub:
    ```
    [![CI](https://github.com/danielpina1/anthrex/actions/workflows/ci.yml/badge.svg)](https://github.com/danielpina1/anthrex/actions/workflows/ci.yml)
    ```

## Interfaces

None. No Rust type, protocol message, config key, or CLI command is added or changed. The only artifact this milestone defines is the workflow file itself, reproduced here in full since it is this milestone's whole deliverable:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  RUST_BACKTRACE: "1"

jobs:
  ci:
    name: ci (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    timeout-minutes: 20
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - uses: Swatinem/rust-cache@v2

      - name: cargo fmt --all --check
        run: cargo fmt --all --check

      - name: cargo clippy --workspace --all-targets -- -D warnings
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: cargo build --workspace --all-targets
        run: cargo build --workspace --all-targets

      - name: cargo test --workspace
        run: cargo test --workspace

      - name: python3 scripts/pty-smoke.py
        run: python3 scripts/pty-smoke.py
```

The two GitHub check names this produces are `ci (ubuntu-latest)` and `ci (macos-latest)`. Branch protection (Manual check, item 4) names these two checks exactly.

## Tasks

### M2.1: Format the codebase

**Files** (modify, formatting only, every file `cargo fmt --all --check` currently flags):
`crates/cli/src/client.rs`, `crates/cli/src/main.rs`, `crates/cli/src/spawn.rs`,
`crates/daemon/src/launch.rs`, `crates/daemon/src/lib.rs`, `crates/daemon/src/lifecycle.rs`, `crates/daemon/src/manager.rs`, `crates/daemon/src/server.rs`, `crates/daemon/src/window.rs`,
`crates/daemon/tests/manager.rs`, `crates/daemon/tests/server.rs`, `crates/daemon/tests/window.rs`,
`crates/proto/src/codec.rs`, `crates/proto/src/messages.rs`, `crates/proto/src/paths.rs`, `crates/proto/src/types.rs`,
`crates/tui/src/app.rs`, `crates/tui/src/connection.rs`, `crates/tui/src/keymap.rs`, `crates/tui/src/lib.rs`, `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/modal.rs`, `crates/tui/src/ui/sidebar.rs`, `crates/tui/src/ui/statusbar.rs`, `crates/tui/src/ui/terminal.rs`,
`crates/tui/tests/connection.rs`.

**Tests first**: no new test. `cargo fmt` never changes program semantics, only whitespace and line breaks, so the gate is that every existing test still passes with the same counts as before the reformat. Before making any change, record the baseline: `cargo test --workspace` reports the proto, daemon (unit + `tests/manager.rs` + `tests/server.rs` + `tests/window.rs`), tui (unit + `tests/connection.rs`), and cli test counts. After the reformat, the same command must report the same counts, all passing.

**Change**: Run `cargo fmt --all` at the repository root. Make no hand edit. Do not touch any file `cargo fmt --all --check` did not already flag. Commit exactly this diff, and nothing else, as its own commit (for example `style: run cargo fmt --all across the workspace`).

**Acceptance**:
- `cargo fmt --all --check` exits 0.
- `cargo build --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` all still pass, with the same test counts as the pre-reformat baseline.
- `git diff` for this commit touches only the files listed above, and every changed line is a whitespace or line-wrap change (spot check at least one file from each crate; no identifier, string literal, or expression is altered).

### M2.2: Add the CI workflow

**Files** (create): `.github/workflows/ci.yml`.

**Tests first**: this is a configuration file, not Rust code, so there is no `cargo test` for it. Before pushing, check the file is well-formed YAML with correct indentation by comparing it line for line against the block in "Interfaces" above (or, if a YAML parser is available locally, load it and confirm it parses to a mapping with top-level keys `name`, `on`, `env`, `jobs`). The real verification is live: once this branch is pushed and the pull request for this milestone is opened, GitHub Actions runs this exact workflow against the branch, because `pull_request` triggers on the branch that adds the trigger file itself.

**Change**: Create `.github/workflows/ci.yml` with exactly the content in "Interfaces". Do not add steps, jobs, or triggers beyond what is specified there.

**Acceptance**: The file matches "Interfaces" byte for byte (comments aside). After pushing, the GitHub Actions tab shows one workflow run named `CI` with two jobs, `ci (ubuntu-latest)` and `ci (macos-latest)`. Both are green once M2.4 and M2.5 are also done (M2.2 on its own may show M2.4/M2.5's not-yet-landed risk on the Ubuntu leg only if the stall does not reproduce there before those tasks land, see Design decision 10, so do M2.2 through M2.5 in order before treating any red Ubuntu run as a real regression).

### M2.3: Add the CI status badge to README

**Files** (modify): `README.md`.

**Tests first**: none; this is a documentation change. Verify by rendering: GitHub's own preview of `README.md` on the pull request branch shows a badge image directly under the `# anthrex` title.

**Change**: Insert exactly this line as the new first line of `README.md`, followed by the existing blank line and the existing `# anthrex` heading (i.e. the badge sits above the heading):
```
[![CI](https://github.com/danielpina1/anthrex/actions/workflows/ci.yml/badge.svg)](https://github.com/danielpina1/anthrex/actions/workflows/ci.yml)
```
Do not change any other line of `README.md`.

**Acceptance**: `README.md`'s first line is the badge markdown above; the rest of the file is byte-for-byte unchanged. Once `.github/workflows/ci.yml` exists on `main` and has run at least once, the badge renders "passing" or "failing" rather than "no status".

### M2.4: Make the Rust stall test prove the stall on Linux

**Files** (modify): `crates/daemon/tests/manager.rs` (function `a_program_that_ignores_stdin_never_blocks_write_input_or_list`); `crates/daemon/src/window.rs` (doc comment at line 25, the one above `INPUT_QUEUE_CAPACITY`).

**Tests first**: this hardens an existing test rather than adding a new one, since the property under test (`write_input` and `list` must return promptly regardless of the child) does not change; only how the test proves the precondition (the child truly is not draining the PTY) changes. Extend `a_program_that_ignores_stdin_never_blocks_write_input_or_list` with a preliminary probe, run immediately after the existing `tokio::time::sleep(Duration::from_millis(600))`, before the concurrent lister thread is spawned:
- In a loop bounded by both a 2-second wall-clock deadline and 400 iterations, call `m.write_input(id, &chunk)` with a fresh 4096-byte chunk each time, with no sleep between calls.
- If any call returns the "input queue full" error (the `anyhow` error whose message contains `"not reading input"`, per `Window::write_input`'s `TrySendError::Full` branch), the probe has proved the writer thread's blocking PTY write is genuinely stalled behind `stty raw -echo; sleep 5`, record `stalled = true` and stop looping.
- If the loop exhausts its 400 iterations or its 2-second budget without ever seeing that error, the write is draining normally on this platform and the rest of the test cannot exercise the regression it exists to catch. Print `eprintln!("skip: pty write to a non-reading child did not stall within 2s on this platform; a_program_that_ignores_stdin_never_blocks_write_input_or_list did not exercise C1 here")`, call `m.remove(id).unwrap()`, and `return` from the test without running the existing lister-thread assertions.
- When `stalled` is true, run the existing assertions unchanged (the concurrent `list()` hammer thread, the 16-chunk `write_input`/`list` timing loop, the `< 100ms` bounds, `m.remove(id).unwrap()`).

Update the doc comment on `INPUT_QUEUE_CAPACITY` in `crates/daemon/src/window.rs` after you have seen the Ubuntu leg of this milestone's own pull request run at least once: if the Ubuntu job's log shows the probe found `stalled = true`, change "stalls writes to the PTY master after about a kilobyte on macOS" to say it stalls on Linux too, and drop "on macOS"; if the Ubuntu job's log shows the skip message, instead say plainly that this is verified on macOS and unconfirmed on Linux, and that `crates/daemon/tests/manager.rs`'s probe detects the case at test time rather than assuming a fixed byte count. Record which of the two happened, with the exact CI run's log excerpt, under this brief's "Implementation notes".

**Change**: as described above.

**Acceptance**: `cargo test --workspace` still passes locally (the probe will report `stalled = true` on a macOS development machine, matching the pre-existing behaviour, so the rest of the test still runs as before). Once pushed, the Ubuntu leg of the workflow's log for this test shows either the existing assertions running (stall confirmed) or the `skip:` message (stall not reproduced), never a silent, uninformative pass with no printed reason either way.

### M2.5: Make the smoke-script stall stage prove the stall on Linux

**Files** (modify): `scripts/pty-smoke.py` (stage 8, the block starting at the `== stage 8:` print and ending before `== stage 9:`).

**Tests first**: same rationale as M2.4; this hardens the existing stage rather than adding a new one. Around the existing `proc3.send_large(paste)` call:
- Record `paste_started = time.monotonic()` immediately before the call and `paste_elapsed = time.monotonic() - paste_started` immediately after it returns (the call itself is unchanged: same 32 KiB bracketed-paste payload, same default 10-second timeout).
- Compute `stalled = paste_elapsed > 0.5`. A write that never stalls completes in well under 0.5 s (it is 32 KiB into a kernel buffer with nothing else happening); a write that does stall behind the child's `sleep 5` takes several seconds, bounded only by `send_large`'s own timeout.
- If `stalled` is `False`, print `f"note: the 32 KiB paste wrote in {paste_elapsed:.2f}s without stalling on this platform; stage 8 could not exercise the blocked-PTY-write regression here"` and skip only the two existing timing assertions (`if ls_elapsed > 1.5: fail(...)` and `if detach_elapsed > 2.0: fail(...)`) by guarding each with `if stalled:`. Still run the `anthrex ls` call, the `shell-4` content check, and the detach itself exactly as today, only the "did it take too long" bounds are conditional on `stalled`.
- If `stalled` is `True`, behaviour is unchanged from today: both timing bounds are checked and `fail()` on violation, and the final `print(...)` at the end of the stage reports the two elapsed times as it does now.

**Change**: as described above.

**Acceptance**: `python3 scripts/pty-smoke.py` still passes locally end to end (macOS: `stalled` is `True`, unchanged behaviour, same printed summary line). Once run in the Ubuntu job, the log for stage 8 shows either the unchanged summary line (stall confirmed) or the `note:` message (stall not reproduced), never a pass with no explanation of which case occurred.

## Verification

Run, in order, exactly the commands `AGENTS.md` requires:

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

All five must exit 0 locally before pushing. Milestone-specific checks:

- Push the branch and open the pull request. Confirm the Actions tab shows `ci (ubuntu-latest)` and `ci (macos-latest)`, both green, within the 20-minute timeout.
- Open each job's log and confirm all five steps ran (not skipped, not cached-and-reused) and that the `cargo fmt --all --check`, `cargo clippy ... -D warnings`, `cargo build`, `cargo test`, and smoke-script steps each show their normal output, not an early exit.
- Confirm the Ubuntu job's log for M2.4's test and M2.5's stage prints one of the two expected outcomes (assertions ran, or a clear skip/note message) as described in those tasks' Acceptance.
- Confirm `git log` for this branch shows the format-only commit (M2.1) as a separate commit from the commit that adds `.github/workflows/ci.yml` (M2.2).

## Manual check

1. Open the pull request for `m2-ci` on GitHub. Confirm both `ci (ubuntu-latest)` and `ci (macos-latest)` checks are green.
2. Open the diff for the formatting commit (M2.1) and skim every file: confirm every changed line is a whitespace or line-wrap change, never a logic change.
3. View `README.md` in the pull request's "Files changed" tab (GitHub renders Markdown previews) and confirm the CI badge appears directly under the `# anthrex` title and links to the workflow.
4. **Branch protection is a manual GitHub setting, not part of this brief's tasks.** In the repository's Settings → Branches, add a protection rule for `main` that requires the `ci (ubuntu-latest)` and `ci (macos-latest)` status checks (exact names from "Interfaces") to pass before merging, and requires the branch to be up to date with `main` before merging. Do this once, by hand, after this milestone's pull request is open so the check names already exist for GitHub to offer.
5. Confirm no daemon was left running from this work: `pgrep -fl "anthrex daemon"` shows nothing of yours. This milestone runs no manual `anthrex` session; the check exists only because `AGENTS.md` requires it before finishing any milestone.

## Risks and gotchas

1. **The stall may not reproduce on `ubuntu-latest`.** Linux's pty input-buffer limit is not necessarily the same as macOS's. Recognise this by M2.4's `skip:` message or M2.5's `note:` message appearing in the Ubuntu job's log. If it appears, that is expected and handled, do not treat it as a failure, and do not try to force a stall by increasing the byte count past what the tasks specify; record the outcome in "Implementation notes" as M2.4 already asks.
2. **A future rustfmt release can change its default style.** `cargo fmt --all --check` failing on an unrelated pull request, across many files, with no logic changes involved, is this: rerun `cargo fmt --all` and commit, the same as task M2.1. There is no `rustfmt.toml` pinning a style version, by the same reasoning as Design decision 6 (one fewer file to keep in sync); if this becomes a recurring problem for later milestones, that is a decision for a later milestone to revisit, not this one.
3. **`Swatinem/rust-cache` keys on `Cargo.lock`.** If a later milestone adds a dependency and forgets to commit the updated `Cargo.lock`, the build still works (cargo regenerates it), just without a cache hit that milestone; this is a slower CI run, not a failure.
4. **Audited and found clean, no fix needed**: `TMPDIR` is only read on `cfg!(target_os = "macos")` in `crates/proto/src/paths.rs::socket_dir`; the Linux branch uses `dirs::runtime_dir()` with a `/tmp/anthrex-<uid>` fallback and never touches `TMPDIR`. `/bin/zsh` appears only as a hardcoded fixture string in `crates/daemon/src/launch.rs`'s unit tests (`ctx()`), which never spawns a real shell, it only asserts that `LaunchContext.shell` is threaded through unchanged, so it is platform-independent. The `SHELL` environment variable is read once, in `crates/daemon/src/lifecycle.rs::run`, with a `/bin/sh` fallback that exists on both macOS and Ubuntu runners; no test depends on a specific `$SHELL` value. `pgrep` appears nowhere in any script or test, its only appearance in the repository is the manual-run instruction in `AGENTS.md`, which is followed by a human or an implementer's own shell, not by any code this workflow runs.
5. **GitHub runs `pull_request` against a merge of the branch and the base.** A workflow that would pass against this branch alone can still fail here if `main` has diverged in a conflicting way. This is standard GitHub behaviour, not specific to this milestone; if it happens, rebase or merge `main` into the branch and push again.

## Follow-ups handled

Per `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`'s "Assignment to milestones" table: "M2 CI: Nothing from the ledger. CI enforces `-D warnings`, which keeps the clippy sweep from regressing." This brief's `cargo clippy --workspace --all-targets -- -D warnings` step (Design decision 7, "Interfaces") is exactly that enforcement; no other ledger item is assigned to this milestone.

## Implementation notes

*(Filled in by the implementer: every deviation from this brief, and the outcome of the Design decision 10 / M2.4 / M2.5 Linux-stall check, goes here.)*
