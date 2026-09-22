#!/usr/bin/env python3
"""Scripted PTY smoke test for anthrex.

`cargo test` covers the pieces; this covers the thing itself. It drives the real
target/debug/anthrex binary through a pseudo-terminal the way a person would -
attaching, creating windows, typing, pasting, detaching - and asserts on the
rendered screen text. Run it from the repo root:

    python3 scripts/pty-smoke.py

It builds the debug binary first if it is missing, and it runs entirely against
an isolated socket and data directory under /tmp, which it removes on the way out.

ratatui's terminal backend only rewrites the cells that changed between two
frames, so on any redraw after the first, plain substring search over the raw
byte stream sees words separated by cursor-jump escapes (e.g. "send" ... jump
... "a" ... jump ... "literal" ...) and never finds a multi-word phrase. So
this script keeps a tiny VT100-ish screen buffer (cursor addressing, SGR/OSC
skipped, erase-in-display/line) built only from the standard library, replays
every byte seen so far into it, and asserts against the reconstructed grid
instead of the raw stream.
"""
import fcntl
import json
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import tempfile
import termios
import time
import tty

from pty_tree_smoke import (
    run_graph_glyphs_stage,
    run_inspector_stage,
    run_project_tree_stage,
    run_tree_connectors_stage,
)


REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(REPO, "target/debug/anthrex")
FAKE_AGENT_BIN = os.path.join(REPO, "target/debug/fake-agent")
DATA_DIR = tempfile.mkdtemp(prefix="anthrex-smoke-", dir="/tmp")
SOCKET = os.path.join(DATA_DIR, "daemon.sock")
FAKE_AGENT_SCRIPT = os.path.join(DATA_DIR, "fake-agent.jsonl")
ROWS, COLS = 40, 120
# W1 and W2 (the worktree stages) create linked checkouts against this repository.
# Fixed rather than a `tempfile.mkdtemp`, per the acceptance check that
# `/tmp/anthrex-smoke-repo-*` is gone once the script finishes.
SMOKE_REPO = f"/tmp/anthrex-smoke-repo-{os.getpid()}"

ENV = dict(os.environ)
ENV["ANTHREX_SOCKET"] = SOCKET
ENV["ANTHREX_DATA_DIR"] = DATA_DIR
ENV["ANTHREX_CLAUDE_BIN"] = FAKE_AGENT_BIN
ENV["ANTHREX_CODEX_BIN"] = FAKE_AGENT_BIN
ENV["FAKE_AGENT_SCRIPT"] = FAKE_AGENT_SCRIPT
ENV["TERM"] = "xterm-256color"
# This script compares rendered screens against literal text. The bottom-bar git
# segment (design decision 21) would make those comparisons depend on the state of
# whatever working tree the daemon happens to run in, so the daemon this script starts
# never probes or watches git at all.
ENV["ANTHREX_GIT"] = "off"
# `crates/proto/src/paths.rs::config_path()` falls back to the real OS config
# directory (e.g. `~/Library/Application Support/anthrex/config.toml`) whenever
# `ANTHREX_CONFIG` is unset. Pointing it here instead means this script's daemon and
# every client it drives never consult whatever a developer running this locally has
# actually configured (a different prefix key would break every `\x02`-prefixed send
# below in a way that has nothing to do with the product). This path is fixed, not a
# `tempfile.mkdtemp`, and every stage but one (the resume stage below) never creates
# it — the suite's own correctness depends on it staying absent for those stages, and
# `ensure_config_path_absent` below is what makes that an enforced invariant instead
# of a hope. See the M6.12/fix-wave-11 reviews for why a fixed, silently-poisonable
# path was a Major finding: a stray file here used to fail stage 2 with "timed out
# waiting for 'new-agent form'", an error that named the form, never this path.
ENV["ANTHREX_CONFIG"] = "/tmp/anthrex-smoke-data/config.toml"


def ensure_config_path_absent():
    """Refuses to run against a pre-existing file at `ANTHREX_CONFIG`'s fixed path.

    fix-wave-11 review, Major: the suite's own correctness depends on this path being
    absent (every `\\x02`-prefixed send below assumes the default `C-b` prefix, which
    a real file here could override), but nothing ever checked that before stage 1
    ran — a stray file (a previous run killed hard enough to skip its own cleanup, a
    manual `ANTHREX_CONFIG=... anthrex ...` debugging session, a typo'd `mkdir -p`)
    silently poisoned every future run with a misdirecting failure at stage 2 that
    named the form, never this path.

    Chosen fix: fail loudly and name the path, rather than `rm -f` it ourselves.
    Removing a file this script did not create is a side effect on something that
    might not be this script's to delete — a developer's own accidental override they
    still care about, say — and the cost of being wrong there (silently destroying
    it) is worse than the cost of being right here (one failed run with a clear
    message). The one stage that legitimately needs a real file at this path (the
    resume stage below) creates it itself, after this check has already passed, and
    owns removing it again itself, in its own `try`/`finally` — deliberately not the
    module-level `finally` below, which cannot tell "this run created the file" from
    "a file was already here and `ensure_config_path_absent` just refused to touch
    it", and must not delete the latter.
    """
    path = ENV["ANTHREX_CONFIG"]
    if os.path.exists(path):
        fail(
            f"a file already exists at {path!r}, the fixed path this suite always "
            "points ANTHREX_CONFIG at so a developer's own real config can never "
            "apply. Every stage in this suite assumes that path starts absent (a "
            "real config there can override the C-b prefix every \\x02 send below "
            "depends on) — left in place, the suite fails downstream with an error "
            "that gives no hint this path is the cause. Remove it by hand "
            f"(`rm -rf {os.path.dirname(path)}`) and re-run."
        )

# `C-b Q`'s own wait for the daemon to confirm a stop is `STOPPING_TIMEOUT`, 5s
# (crates/tui/src/app/link.rs). Past that the client gives up *without* quitting
# (it only toasts), so a genuine quit must land well inside it — a client stuck on
# the read-arm bug this stage guards against would just hang past any bound. Per
# docs/timing-budgets.md's standing rule 1, the bound below is derived from that
# constant plus generous slack for a freshly-spawned daemon with zero windows to
# tear down, not tuned close to the real (sub-second) cost.
TUI_QUIT_TIMEOUT = 15.0


def fail(msg):
    print(f"FAIL: {msg}")
    raise SystemExit(1)


CSI_RE = re.compile(r"\x1b\[([^a-zA-Z]*)([a-zA-Z])")
OSC_RE = re.compile(r"\x1b\].*?(\x07|\x1b\\)", re.S)


class Screen:
    """Minimal VT100-ish screen buffer: cursor addressing + erase, ignores SGR/OSC."""

    def __init__(self, rows, cols):
        self.rows, self.cols = rows, cols
        self.grid = [[" "] * cols for _ in range(rows)]
        self.row = 0
        self.col = 0

    def feed(self, data: str):
        i, n = 0, len(data)
        while i < n:
            ch = data[i]
            if ch == "\x1b":
                m = CSI_RE.match(data, i)
                if m:
                    self._csi(m.group(1), m.group(2))
                    i = m.end()
                    continue
                if data[i : i + 2] == "\x1b]":
                    m2 = OSC_RE.match(data, i)
                    if m2:
                        i = m2.end()
                        continue
                # Unrecognized escape: skip just the ESC byte, keep going.
                i += 1
                continue
            elif ch == "\r":
                self.col = 0
                i += 1
            elif ch == "\n":
                self.row = min(self.row + 1, self.rows - 1)
                i += 1
            elif ch in ("\x07", "\x08"):
                i += 1
            else:
                if 0 <= self.row < self.rows and 0 <= self.col < self.cols:
                    self.grid[self.row][self.col] = ch
                self.col += 1
                if self.col >= self.cols:
                    self.col = 0
                    self.row = min(self.row + 1, self.rows - 1)
                i += 1

    def _csi(self, params, final):
        parts = params.split(";") if params else []

        def num(idx, default=1):
            if idx < len(parts) and parts[idx].isdigit():
                return int(parts[idx])
            return default

        if final in ("H", "f"):
            self.row = max(0, min(num(0, 1) - 1, self.rows - 1))
            self.col = max(0, min(num(1, 1) - 1, self.cols - 1))
        elif final == "A":
            self.row = max(0, self.row - num(0, 1))
        elif final == "B":
            self.row = min(self.rows - 1, self.row + num(0, 1))
        elif final == "C":
            self.col = min(self.cols - 1, self.col + num(0, 1))
        elif final == "D":
            self.col = max(0, self.col - num(0, 1))
        elif final == "J":
            mode = num(0, 0)
            if mode in (2, 3):
                self.grid = [[" "] * self.cols for _ in range(self.rows)]
            elif mode == 0:
                for c in range(self.col, self.cols):
                    self.grid[self.row][c] = " "
                for r in range(self.row + 1, self.rows):
                    self.grid[r] = [" "] * self.cols
            elif mode == 1:
                for c in range(0, self.col + 1):
                    self.grid[self.row][c] = " "
                for r in range(0, self.row):
                    self.grid[r] = [" "] * self.cols
        elif final == "K":
            mode = num(0, 0)
            if mode == 0:
                for c in range(self.col, self.cols):
                    self.grid[self.row][c] = " "
            elif mode == 1:
                for c in range(0, self.col + 1):
                    self.grid[self.row][c] = " "
            elif mode == 2:
                self.grid[self.row] = [" "] * self.cols
        # SGR ('m'), cursor show/hide, mouse/paste mode toggles, etc: no text effect.

    def text(self):
        return "\n".join("".join(r).rstrip() for r in self.grid)


class PtyProc:
    """A process running under a PTY, with a reconstructed screen and wait-for-text helpers."""

    def __init__(self, argv):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            # Child
            try:
                os.chdir(REPO)
                os.execvpe(argv[0], argv, ENV)
            except Exception as e:  # pragma: no cover
                os.write(2, f"exec failed: {e}\n".encode())
                os._exit(127)
        # Parent: set the window size to 120x40 right after spawning.
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.raw = b""
        self.mark = 0

    def mark_here(self):
        """Remembers the current end of the raw buffer, to later inspect only new bytes."""
        self.mark = len(self.raw)

    def bytes_since_mark(self) -> bytes:
        return self.raw[self.mark :]

    def read_available(self, timeout=0.2):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if self.fd in r:
            try:
                chunk = os.read(self.fd, 65536)
            except OSError:
                return False
            if not chunk:
                return False
            self.raw += chunk
            return True
        return False

    def screen_text(self):
        scr = Screen(ROWS, COLS)
        scr.feed(self.raw.decode("utf-8", errors="replace"))
        return scr.text()

    def screen_region_text(self, col_start, col_end=None):
        """Like `screen_text`, but only the columns `[col_start, col_end)` of
        every row (default `col_end`: the right edge). For asserting on one
        pane of a split layout without a match in another pane satisfying it
        by coincidence — the sidebar and the graph overview both draw box
        corners and row/edge glyphs, on the same screen, from different code.
        """
        scr = Screen(ROWS, COLS)
        scr.feed(self.raw.decode("utf-8", errors="replace"))
        end = COLS if col_end is None else col_end
        return "\n".join("".join(row[col_start:end]).rstrip() for row in scr.grid)

    def wait_for(self, text, timeout=10.0, label=None):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if text in self.screen_text():
                return
            self.read_available(timeout=0.2)
        fail(f"timed out waiting for {label or text!r}\n--- rendered screen ---\n{self.screen_text()}")

    def wait_for_focused_window(self, name, timeout=10.0):
        """Wait until `name` is the main pane, not merely a sidebar row."""
        self.wait_for(
            f"{name} · shell",
            timeout=timeout,
            label=f"{name} focused main-pane title",
        )

    def send(self, data: bytes):
        os.write(self.fd, data)

    def send_large(self, data: bytes, timeout=10.0):
        """Writes a lot of bytes, draining output as it goes so neither side wedges.

        Both directions have to keep moving: the client blocks writing its own frames
        once we stop reading, and a client that is blocked writing is not reading, so a
        plain blocking `os.write` here would deadlock the pair. The fd is put in
        non-blocking mode and reads are drained on every pass, which also means a client
        that has genuinely stopped reading shows up as this `timeout` rather than a hang.
        """
        deadline = time.monotonic() + timeout
        view = memoryview(data)
        os.set_blocking(self.fd, False)
        try:
            while view:
                if time.monotonic() > deadline:
                    fail(f"writing to the client's pty stalled with {len(view)} bytes left")
                self.read_available(timeout=0.0)
                try:
                    view = view[os.write(self.fd, view[:4096]) :]
                except BlockingIOError:
                    time.sleep(0.01)
        finally:
            os.set_blocking(self.fd, True)

    def wait_exit(self, timeout=5.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid == self.pid:
                return status
            # Keep draining output so the child never blocks on a full pty buffer.
            self.read_available(timeout=0.2)
        fail(f"process did not exit within {timeout}s")

    def close(self):
        try:
            os.close(self.fd)
        except OSError:
            pass


def run_cmd(args, expect_ok=True, timeout=15, env=None):
    result = subprocess.run(
        [BIN] + args, cwd=REPO, env=env if env is not None else ENV, capture_output=True, text=True, timeout=timeout
    )
    if expect_ok and result.returncode != 0:
        fail(f"`anthrex {' '.join(args)}` exited {result.returncode}\nstdout: {result.stdout}\nstderr: {result.stderr}")
    return result


def new_shell(proc, expected):
    """`C-b c` now opens the new-agent form (decision 26) instead of creating a shell
    directly, so every stage that used to send a bare `\\x02c` and wait for its window to
    focus now has to drive the form's default path first: Shell is the form's third
    runtime, so `3` selects it, and `\\r` submits with the daemon's stock name and the
    default directory.
    """
    proc.send(b"\x02c")
    proc.wait_for("new agent", label="new-agent form opened")
    proc.send(b"3")
    proc.send(b"\r")
    proc.wait_for_focused_window(expected)


def ensure_binary():
    binaries = [BIN, FAKE_AGENT_BIN]
    missing = [path for path in binaries if not os.path.exists(path)]
    if not missing:
        return
    print(f"== building missing smoke binaries: {', '.join(missing)} ==")
    result = subprocess.run(["cargo", "build", "--workspace"], cwd=REPO, timeout=900)
    if result.returncode != 0 or any(not os.path.exists(path) for path in binaries):
        fail("`cargo build --workspace` did not produce anthrex and fake-agent")


def write_fake_agent_script():
    steps = [
        {"hook": "SessionStart", "payload": {}},
        {"hook": "UserPromptSubmit", "payload": {}},
        {"hook": "PreToolUse", "payload": {"tool_name": "Bash"}},
        {"wait_ms": 3000},
        {"hook": "PostToolUse", "payload": {}},
        {"hook": "Stop", "payload": {}},
    ]
    with open(FAKE_AGENT_SCRIPT, "w", encoding="utf-8") as script:
        for step in steps:
            script.write(json.dumps(step) + "\n")


def stop_daemon(timeout=30.0):
    """Stops the daemon and waits until it is really gone.

    `anthrex daemon stop` (`crates/cli/src/main.rs`, `DaemonAction::Stop`) now blocks
    until the daemon has actually exited: it waits for the connection to close, then
    for the daemon's own lifetime lock to be released, up to 10s. This function's own
    socket-removal wait stays as a guard on top of that regardless — a defensive
    backstop for the case where the `daemon stop` subprocess itself times out, errors,
    or is not the one actually holding the socket, so that even a bare `stop_daemon()`
    call from `finally` (which never checks `daemon stop`'s exit code) still confirms
    the daemon is really gone before this run's temp dir is removed. The daemon ends
    every agent process group with SIGHUP (escalating if needed) and removes the
    socket file only as its very last act, so a stale socket here really does mean a
    stale file, not a live daemon's.
    """
    subprocess.run([BIN, "daemon", "stop"], cwd=REPO, env=ENV, capture_output=True, text=True, timeout=timeout)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not os.path.exists(SOCKET):
            return
        time.sleep(0.1)
    # Nothing removed it: it is a stale file, not a live daemon's socket.
    try:
        os.unlink(SOCKET)
    except OSError:
        pass


def calibrate_raw_pty_capacity():
    """Try 32 KiB on a separate non-reading raw PTY; return (blocked, written).

    This calibrates the local kernel, not the agent's writer or the client's PTY.
    """
    master, slave = pty.openpty()
    try:
        tty.setraw(slave)
        os.set_blocking(master, False)
        remaining = memoryview(b"x" * (32 * 1024))
        deadline = time.monotonic() + 1.0
        blocked_since = None
        while remaining:
            if time.monotonic() >= deadline:
                fail("raw PTY calibration exceeded its 1s deadline")
            try:
                written = os.write(master, remaining[:4096])
            except BlockingIOError:
                if blocked_since is None:
                    blocked_since = time.monotonic()
                if time.monotonic() - blocked_since >= 0.2:
                    return True, 32 * 1024 - len(remaining)
                # Let asynchronous line-discipline work finish before claiming
                # backpressure; a transient EAGAIN alone is not confirmation.
                select.select([], [master], [], 0.01)
                continue
            if written == 0:
                fail("raw PTY calibration made no write progress")
            blocked_since = None
            remaining = remaining[written:]
        return False, 32 * 1024
    finally:
        os.close(master)
        os.close(slave)


def _smoke_git(args, cwd=None):
    """Runs git against `SMOKE_REPO` (or `cwd`) with a repo-local identity, the same
    discipline the Rust tests' `TempRepo` follows: a developer's global git config must
    not change what these stages see.
    """
    env = dict(os.environ)
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    env["GIT_TERMINAL_PROMPT"] = "0"
    result = subprocess.run(
        ["git", "-C", cwd or SMOKE_REPO] + args,
        env=env,
        capture_output=True,
        text=True,
        timeout=15,
    )
    if result.returncode != 0:
        fail(f"git {' '.join(args)} in {cwd or SMOKE_REPO} failed: {result.stderr}")
    return result.stdout


def make_repo():
    """Creates the small repository W1 and W2 create linked worktrees against: one
    commit on `main`, repo-local identity, `commit.gpgsign` off. Removed by the script's
    final cleanup.
    """
    shutil.rmtree(SMOKE_REPO, ignore_errors=True)
    os.makedirs(SMOKE_REPO)
    _smoke_git(["init", "-q", "-b", "main"])
    _smoke_git(["config", "user.name", "anthrex smoke"])
    _smoke_git(["config", "user.email", "smoke@anthrex.test"])
    _smoke_git(["config", "commit.gpgsign", "false"])
    with open(os.path.join(SMOKE_REPO, "README"), "w", encoding="utf-8") as handle:
        handle.write("anthrex smoke fixture\n")
    _smoke_git(["add", "README"])
    _smoke_git(["commit", "-q", "-m", "init"])
    return SMOKE_REPO


def run_worktree_cli_stage(repo):
    """W1: `anthrex new --worktree` and `anthrex rm --worktree` end to end, entirely
    through the CLI, against the daemon the rest of the script already started.
    """
    print("== stage W1: worktree from the CLI ==")
    branch = "smoke/cli"
    run_cmd(
        ["new", "--runtime", "shell", "--name", "wt-cli", "--dir", repo, "--worktree", branch],
        timeout=60,
    )
    listed = run_cmd(["ls"]).stdout
    if "wt-cli" not in listed or branch not in listed:
        fail(f"`anthrex ls` did not list wt-cli with {branch}:\n{listed}")

    windows = json.loads(run_cmd(["ls", "--json"]).stdout)
    wt_cli = next((w for w in windows if w["name"] == "wt-cli"), None)
    if wt_cli is None or not wt_cli.get("worktree"):
        fail(f"wt-cli has no worktree recorded: {wt_cli!r}")
    worktree_path = wt_cli["worktree"]
    # The daemon canonicalizes the worktree path it hands back (so it matches the git
    # registry's key), which resolves macOS's `/tmp` -> `/private/tmp` symlink; compare
    # against the same resolution rather than the raw `DATA_DIR` string.
    worktrees_root = os.path.realpath(os.path.join(DATA_DIR, "worktrees"))
    if not worktree_path.startswith(worktrees_root):
        fail(f"worktree path {worktree_path!r} is not under {worktrees_root!r}")

    worktree_list = subprocess.run(
        ["git", "-C", repo, "worktree", "list", "--porcelain"],
        capture_output=True,
        text=True,
        timeout=15,
    ).stdout
    if f"branch refs/heads/{branch}" not in worktree_list or worktrees_root not in worktree_list:
        fail(f"`git worktree list --porcelain` did not show the new checkout:\n{worktree_list}")

    dup = run_cmd(
        ["new", "--runtime", "shell", "--name", "wt-dup", "--dir", repo, "--worktree", branch],
        expect_ok=False,
        timeout=60,
    )
    if dup.returncode == 0 or "already checked out" not in dup.stderr:
        fail(f"a duplicate worktree create should have failed with 'already checked out':\n{dup.stderr}")

    run_cmd(["rm", "wt-cli", "--worktree"], timeout=60)
    if os.path.exists(worktree_path):
        fail(f"worktree directory {worktree_path!r} still exists after `anthrex rm --worktree`")
    branch_list = subprocess.run(
        ["git", "-C", repo, "branch", "--list", branch],
        capture_output=True,
        text=True,
        timeout=15,
    ).stdout
    if branch not in branch_list:
        fail(f"branch {branch} should still exist after removal:\n{branch_list}")
    print("ok: worktree created, listed, refused for a duplicate branch, and removed via the CLI")


def run_worktree_form_stage(repo):
    """W2: the new-agent form's worktree path, a dirty-tree refusal, and the force
    follow-up, driven through the TUI.
    """
    print("== stage W2: form, dirty removal, force ==")
    branch = "smoke/form"
    proc = PtyProc([BIN])
    proc.wait_for("agents", label="worktree-form attach banner")
    proc.send(b"\x02c")
    proc.wait_for("new agent", label="new-agent form opened for the worktree stage")
    # Shell, Tab to Name, type it, Tab to Directory, clear the default and type the repo
    # path, Tab to Worktree, tick it, Tab to Branch (now visible), type it, submit. Sent
    # as separate writes, not one concatenated burst: crossterm's raw-mode reader can
    # coalesce a single large write of mixed control bytes and plain text into fewer
    # events than were sent, silently dropping keystrokes (verified by isolating this
    # exact sequence — splitting it into one `send()` per key is what makes it reliable).
    for chunk in (
        b"3",
        b"\t",
        b"wt-form",
        b"\t",
        b"\x15",
        repo.encode(),
        b"\t",
        b" ",
        b"\t",
        branch.encode(),
        b"\r",
    ):
        proc.send(chunk)
    proc.wait_for_focused_window("wt-form")
    proc.wait_for(branch, label=f"{branch} on screen after submit")

    proc.send(b"pwd\r")
    proc.wait_for("smoke-form", label="worktree directory name in pwd output")

    windows = json.loads(run_cmd(["ls", "--json"]).stdout)
    wt_form = next((w for w in windows if w["name"] == "wt-form"), None)
    if wt_form is None or not wt_form.get("worktree"):
        fail(f"wt-form has no worktree recorded: {wt_form!r}")
    worktree_path = wt_form["worktree"]

    proc.send(b"touch dirty.txt\r")
    dirty_marker = os.path.join(worktree_path, "dirty.txt")
    deadline = time.monotonic() + 10.0
    while not os.path.exists(dirty_marker):
        if time.monotonic() >= deadline:
            fail(f"{dirty_marker} never appeared")
        proc.read_available(timeout=0.2)

    proc.send(b"\x02X")
    proc.wait_for("also remove worktree", label="remove-confirm worktree checkbox")
    proc.send(b" ")
    proc.send(b"\r")
    proc.wait_for("uncommitted or untracked", label="dirty-tree force prompt")

    proc.send(b"f")
    deadline = time.monotonic() + 10.0
    gone = False
    while time.monotonic() < deadline:
        remaining = json.loads(run_cmd(["ls", "--json"]).stdout)
        if not any(w["name"] == "wt-form" for w in remaining):
            gone = True
            break
        # Keep draining proc's own output while polling `ls`, the same discipline
        # `wait_exit` documents above: the removal makes the daemon redraw the sidebar
        # (a window disappearing, a spinner ticking) every 100ms, and a PTY's kernel
        # output buffer is finite — left undrained across a multi-second wait, the
        # client can block on its own write and stop reading input entirely, which
        # looks exactly like the keypress that started this wait was never delivered.
        proc.read_available(timeout=0.2)
    if not gone:
        fail(
            "wt-form was still listed 10s after forcing the dirty removal\n"
            f"--- screen ---\n{proc.screen_text()}\n"
            f"--- windows ---\n{remaining!r}"
        )
    if os.path.exists(worktree_path):
        fail(f"worktree directory {worktree_path!r} still exists after the forced removal")
    branch_list = subprocess.run(
        ["git", "-C", repo, "branch", "--list", branch],
        capture_output=True,
        text=True,
        timeout=15,
    ).stdout
    if branch not in branch_list:
        fail(f"branch {branch} should still exist after the forced removal:\n{branch_list}")

    proc.send(b"\x02d")
    status = proc.wait_exit(timeout=5.0)
    if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
        fail(f"worktree-form detach did not exit cleanly with status 0 (raw status {status})")
    proc.close()
    print("ok: the new-agent form created a worktree, a dirty removal was refused, then forced")


# fix-wave-11 review: this milestone's headline feature - restarting a Claude or Codex
# window and resuming its prior session - had no end-to-end coverage anywhere. Stage 10
# above only ever restarts a `shell`, which has no session identity at all, so it is
# structurally incapable of reaching `manager/restore.rs`/`restart.rs`'s real
# `session_id` logic. `run_resume_stage` below closes that gap using a fake runtime
# script (never a real agent - this suite's own rule, and the pattern task M6.7
# established for `crates/daemon/tests/lifecycle.rs`'s
# `restart_resumes_claude_and_codex_sessions`, replayed here against the real `anthrex`
# binary and a real daemon process instead of `daemon::run` called in-process).
ANTHREX_CONFIG_PATH = ENV["ANTHREX_CONFIG"]
RESUME_CONFIG_DIR = os.path.dirname(ANTHREX_CONFIG_PATH)


def _write_resume_runtime_script(resume_dir):
    """A fake `claude`/`codex` binary: prints every argument it is given to a
    per-window log file, then blocks with `exec sleep 60` so the window stays alive
    to be restarted later. `--version` is answered directly and fast, the same
    special case `fake-agent` makes for itself (`crates/fake-agent/src/main.rs`) for
    the same reason: `crates/daemon/src/lifecycle/codex_version.rs`'s startup probe
    runs this binary with `--version` before any window launch is accepted, and
    without a fast, successful reply every daemon start below would stall for that
    probe's own 5s timeout.
    """
    script_path = os.path.join(resume_dir, "resume-runtime.sh")
    with open(script_path, "w", encoding="utf-8") as handle:
        handle.write(
            "#!/bin/sh\n"
            'if [ "$1" = "--version" ]; then\n'
            "  printf 'codex-cli 0.155.0\\n'\n"
            "  exit 0\n"
            "fi\n"
            f'out="{resume_dir}/argv-$ANTHREX_WINDOW_ID.log"\n'
            "{\n"
            "  printf 'ARGV:'\n"
            "  for a in \"$@\"; do printf ' [%s]' \"$a\"; done\n"
            "  printf '\\n'\n"
            "} > \"$out\"\n"
            "printf 'READY\\n'\n"
            "exec sleep 60\n"
        )
    os.chmod(script_path, 0o755)
    return script_path


def run_hook(window_id, source, payload, timeout=10):
    """Fires a real hook the way a real agent's own hook command would: a real
    `anthrex hook` child process, over the real socket - the same mechanism
    `crates/cli/tests/persistence.rs`'s `session_id_learned_from_a_hook_is_saved`
    checks at the Rust integration level, driven here from outside the window's own
    process (the fake runtime script above never fires a hook itself) rather than
    from within a scripted fake agent. `anthrex hook` always exits 0 regardless of
    whether the daemon actually accepted the payload (`crates/cli/src/hook.rs`'s own
    "the caller always exits zero afterwards"), so a nonzero exit here is a distinct,
    earlier failure - process spawn, argument parsing - not evidence either way about
    whether the daemon learned the session id; that is checked separately by
    `wait_for_session_id`.
    """
    result = subprocess.run(
        [BIN, "hook", "--window", str(window_id), "--source", source, json.dumps(payload)],
        cwd=REPO,
        env=ENV,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if result.returncode != 0:
        fail(
            f"`anthrex hook --window {window_id} --source {source}` exited "
            f"{result.returncode}\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )


def wait_for_session_id(window_id, expected, timeout=5.0):
    deadline = time.monotonic() + timeout
    seen = None
    while time.monotonic() < deadline:
        windows = json.loads(run_cmd(["ls", "--json"]).stdout)
        window = next((w for w in windows if w["id"] == window_id), None)
        seen = window.get("session_id") if window else None
        if seen == expected:
            return
        time.sleep(0.1)
    fail(f"window {window_id} never reported session_id {expected!r} (last saw {seen!r})")


def wait_for_argv_log(path, timeout=10.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if os.path.exists(path):
            with open(path, encoding="utf-8") as handle:
                content = handle.read()
            if content:
                return content
        time.sleep(0.1)
    fail(f"{path} never appeared with content within {timeout}s")


def run_resume_stage():
    """Covers the restore-to-argv path for both runtime shapes: create a window on a
    fake runtime, give it a session id the way the daemon actually learns one (a real
    `SessionStart` hook), stop and restart the daemon so the window is restored from
    `state.json`, restart the window, and assert the resumed process's argv carries
    `--resume <session>` (Claude, design decision 15) or a trailing `resume <session>`
    (Codex, design decision 16), with the original prompt gone either way.

    `runtimes.claude.command`/`runtimes.codex.command` must be resolved from
    `config.toml` at `ANTHREX_CONFIG`, not from `ANTHREX_CLAUDE_BIN`/`ANTHREX_CODEX_BIN`
    - config decision 6's precedence puts the environment variables first, and every
    other stage in this suite relies on exactly that override (`ANTHREX_CLAUDE_BIN`/
    `ANTHREX_CODEX_BIN` point at `FAKE_AGENT_BIN` for the whole module). This stage's
    own daemon starts below run with those two variables removed from the environment,
    so the config file it writes here is what actually resolves `claude_bin`/
    `codex_bin` - the same production config path
    `crates/daemon/tests/lifecycle.rs`'s own `restart_resumes_claude_and_codex_sessions`
    exercises in-process, replayed here against the real binary.

    Reads the resumed argv from a file the fake runtime script writes, rather than
    scraping it off the TUI's rendered screen the way most other stages in this file
    assert things: Claude's `--settings` argument alone is a full JSON blob covering
    all ten hook events, long enough to wrap across many rows of even a 120-column
    pane, and this script's reconstructed screen buffer (see the module docstring)
    has no notion of a soft-wrapped row the way `vt100::Screen::row_wrapped` does -
    joining its rows with a literal newline would split a wrapped argv line exactly
    where a substring check needs it whole. The daemon spawn, the socket, the restart
    and the restore-from-`state.json` this stage exercises are all real regardless of
    which side effect the assertion reads back.
    """
    print("== stage 12b: restart resumes a Claude/Codex window's prior session ==")
    resume_dir = os.path.join(DATA_DIR, "resume-argv")
    os.makedirs(resume_dir, exist_ok=True)
    script_path = _write_resume_runtime_script(resume_dir)

    os.makedirs(RESUME_CONFIG_DIR, exist_ok=True)
    try:
        with open(ANTHREX_CONFIG_PATH, "w", encoding="utf-8") as handle:
            handle.write(
                f"[runtimes.claude]\ncommand = {json.dumps(script_path)}\n\n"
                f"[runtimes.codex]\ncommand = {json.dumps(script_path)}\n"
            )

        resume_env = dict(ENV)
        resume_env.pop("ANTHREX_CLAUDE_BIN", None)
        resume_env.pop("ANTHREX_CODEX_BIN", None)

        run_cmd(["daemon", "start"], env=resume_env)

        seed_prompt = "seed prompt must vanish on resume"
        created_claude = run_cmd(
            ["new", "--runtime", "claude", "--name", "resume-claude", "--prompt", seed_prompt]
        )
        created_codex = run_cmd(
            ["new", "--runtime", "codex", "--name", "resume-codex", "--prompt", seed_prompt]
        )
        try:
            claude_id = int(created_claude.stdout.strip())
            codex_id = int(created_codex.stdout.strip())
        except ValueError:
            fail(
                "`anthrex new` did not print a window id for the resume stage: "
                f"claude={created_claude.stdout!r} codex={created_codex.stdout!r}"
            )

        claude_log = os.path.join(resume_dir, f"argv-{claude_id}.log")
        codex_log = os.path.join(resume_dir, f"argv-{codex_id}.log")
        wait_for_argv_log(claude_log)
        wait_for_argv_log(codex_log)
        print("ok: both windows launched through the fake runtime script")

        claude_session = f"resume-claude-session-{claude_id}"
        codex_session = f"resume-codex-session-{codex_id}"
        run_hook(claude_id, "claude", {"hook_event_name": "SessionStart", "session_id": claude_session})
        run_hook(codex_id, "codex-hook", {"hook_event_name": "SessionStart", "session_id": codex_session})
        wait_for_session_id(claude_id, claude_session)
        wait_for_session_id(codex_id, codex_session)
        print("ok: a real SessionStart hook gave both windows a session id")

        # Removed so the post-restart wait below unambiguously observes the *new*
        # invocation's argv rather than a stale read of this first one's.
        os.remove(claude_log)
        os.remove(codex_log)

        run_cmd(["daemon", "stop"])
        status_result = run_cmd(["daemon", "status"])
        if "not running" not in status_result.stdout:
            fail(
                "`anthrex daemon status` did not report not running ahead of the "
                f"resume restart:\n{status_result.stdout}"
            )

        # The restore-from-state.json path (design decision 14): the daemon that
        # comes back up here has no in-memory record of either window at all: both
        # entries exist only because `restore()` rebuilt them from `state.json` on
        # this fresh start, dormant, with the session id the hooks above saved.
        run_cmd(["daemon", "start"], env=resume_env)
        run_cmd(["restart", "resume-claude"])
        run_cmd(["restart", "resume-codex"])

        claude_argv = wait_for_argv_log(claude_log)
        codex_argv = wait_for_argv_log(codex_log)

        if f"[--resume] [{claude_session}]" not in claude_argv:
            fail(f"resumed claude argv did not carry --resume {claude_session}:\n{claude_argv}")
        if f"[--] [{seed_prompt}]" in claude_argv:
            fail(f"resumed claude argv still carried the original prompt:\n{claude_argv}")
        # Design decision 16: `resume <id>` must be the trailing pair of Codex's argv,
        # not merely present anywhere in it - pinned here the same way
        # `crates/daemon/tests/lifecycle.rs`'s own resume test pins it.
        if not codex_argv.strip().endswith(f"[resume] [{codex_session}]"):
            fail(f"resumed codex argv did not end with resume {codex_session}:\n{codex_argv}")
        if f"[--] [{seed_prompt}]" in codex_argv:
            fail(f"resumed codex argv still carried the original prompt:\n{codex_argv}")
        print(
            "ok: restarting each window after a daemon restart resumed its session - "
            "claude via --resume, codex via a trailing resume - with the original "
            "prompt gone from both"
        )

        run_cmd(["rm", "resume-claude"])
        run_cmd(["rm", "resume-codex"])
        run_cmd(["daemon", "stop"])
        status_result = run_cmd(["daemon", "status"])
        if "not running" not in status_result.stdout:
            fail(
                "`anthrex daemon status` did not report not running after the resume "
                f"stage:\n{status_result.stdout}"
            )
    finally:
        # This stage is the only thing in the suite that ever writes a real file to
        # ANTHREX_CONFIG's fixed path (`ensure_config_path_absent`'s own doc comment
        # explains why every other stage needs that path to start absent). Removed
        # here, unconditionally, on every exit from this stage - success or failure -
        # so this run never poisons the next one.
        shutil.rmtree(RESUME_CONFIG_DIR, ignore_errors=True)


def main():
    ensure_config_path_absent()
    ensure_binary()
    write_fake_agent_script()

    print("== stage 1: initial attach ==")
    proc = PtyProc([BIN])
    proc.wait_for("agents", label="initial banner")
    proc.wait_for("no agents yet", label="empty sidebar hint")
    print("ok: initial frame shows anthrex UI with no agents yet")

    print("== stage 2: create shell-1, run a command ==")
    new_shell(proc, "shell-1")
    proc.send(b"echo smoke-$((40+2))\r")
    proc.wait_for("smoke-42", label="echo output in shell-1")
    print("ok: shell-1 created and command output visible")

    print("== stage 3: create shell-2, exercise keys, help overlay ==")
    new_shell(proc, "shell-2")
    proc.send(b"\x02k")
    time.sleep(0.3)
    proc.read_available(timeout=0.3)
    proc.send(b"\x02?")
    proc.wait_for("send a literal C-b", label="help overlay text")
    proc.send(b" ")  # any key closes the help overlay
    time.sleep(0.2)
    proc.read_available(timeout=0.3)
    print("ok: window switching and help overlay work")

    print("== stage 4: detach ==")
    proc.mark_here()
    proc.send(b"\x02d")
    status = proc.wait_exit(timeout=5.0)
    if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
        fail(f"detach did not exit cleanly with status 0 (raw status {status})")
    print("ok: detach exited with status 0")

    print("== stage 4b: shutdown bytes disable mouse capture and bracketed paste ==")
    # Proves the TerminalGuard (crates/tui/src/lib.rs) actually ran: on a clean detach it must
    # write crossterm's DisableMouseCapture and DisableBracketedPaste sequences to the PTY before
    # exiting, or the user's real shell would be left reading raw mouse/paste escape codes.
    shutdown_bytes = proc.bytes_since_mark()
    has_mouse_disable = b"?1000l" in shutdown_bytes  # one of DisableMouseCapture's several `l` sequences
    has_paste_disable = b"?2004l" in shutdown_bytes  # DisableBracketedPaste
    if not has_mouse_disable or not has_paste_disable:
        fail(
            "shutdown bytes did not contain both disable sequences "
            f"(mouse={has_mouse_disable}, paste={has_paste_disable}):\n{shutdown_bytes!r}"
        )
    proc.close()
    print("ok: shutdown bytes contain both the mouse-capture and bracketed-paste disable sequences")

    print("== stage 5: ls shows both windows while detached ==")
    result = run_cmd(["ls"])
    if "shell-1" not in result.stdout or "shell-2" not in result.stdout:
        fail(f"`anthrex ls` did not list both windows:\n{result.stdout}")
    print("ok: ls lists shell-1 and shell-2")

    print("== stage 6: re-attach, verify persisted output, detach again ==")
    proc2 = PtyProc([BIN])
    proc2.wait_for("agents", label="re-attach banner")
    proc2.send(b"\x021")
    proc2.wait_for("smoke-42", label="smoke-42 visible again after focusing window 1")
    proc2.send(b"\x02d")
    status2 = proc2.wait_exit(timeout=5.0)
    if not os.WIFEXITED(status2) or os.WEXITSTATUS(status2) != 0:
        fail(f"second detach did not exit cleanly with status 0 (raw status {status2})")
    proc2.close()
    print("ok: re-attach showed persisted shell-1 output, detached cleanly")

    print("== stage 7: Alt+Enter reaches the child as ESC CR ==")
    # Regression for the keymap fix: Alt+Enter used to be encoded as a bare CR, which
    # submits a claude/codex prompt instead of inserting a newline. `cat -v` renders the
    # ESC byte it actually receives as the two characters ^ and [.
    proc3 = PtyProc([BIN])
    proc3.wait_for("agents", label="third attach banner")
    new_shell(proc3, "shell-3")
    proc3.send(b"stty -echo; printf '%s%s\\n' CAT_ READY; cat -v\r")
    proc3.wait_for("CAT_READY", label="cat -v readiness in focused shell-3")
    proc3.send(b"\x1b\r")  # ESC CR: how a terminal reports Alt+Enter
    proc3.wait_for("^[", label="ESC rendered by cat -v in the focused window")
    proc3.send(b"\x04")  # Ctrl-D ends cat
    print("ok: Alt+Enter arrived at the child as ESC CR")

    print("== stage 8: a 32 KiB paste freezes neither the daemon nor the client ==")
    # Regression for the blocked-PTY-write fix. `stty raw -echo` is what every full-screen
    # agent does: no canonical line discipline draining the input queue and no echo.
    # The kernel's capacity varies by platform. The daemon used to perform the
    # write inline, on a tokio worker, while holding the window-table mutex - so for the
    # length of the stall NOTHING else worked: no other client, no tick, no kill, no
    # shutdown. `anthrex ls` from outside is the sharpest probe of that, because it needs
    # exactly the mutex the stalled write was holding.
    blocked, written = calibrate_raw_pty_capacity()
    if blocked:
        print(f"ok: local raw PTY calibration retained backpressure for 200ms after {written}/32768 bytes")
    else:
        print("note: local raw PTY accepted all 32768 bytes; platform backpressure unconfirmed")
    print("note: calibration does not observe the agent's writer; both timing limits remain enforced")
    new_shell(proc3, "shell-4")
    child_started = time.monotonic()
    proc3.send(b"stty raw -echo && printf '%s%s' RAW_ READY && exec sleep 30\r")
    proc3.wait_for("RAW_READY", label="shell-4 raw-mode readiness")
    print("ok: child reported raw-mode readiness")
    paste = b"\x1b[200~" + b"x" * (32 * 1024) + b"\x1b[201~"
    proc3.send_large(paste)

    started = time.monotonic()
    try:
        listed = subprocess.run([BIN, "ls"], cwd=REPO, env=ENV, capture_output=True, text=True, timeout=5)
    except subprocess.TimeoutExpired:
        fail("`anthrex ls` never returned after pasting to the non-reading child")
    ls_elapsed = time.monotonic() - started
    if listed.returncode != 0:
        fail(f"`anthrex ls` failed after the paste: {listed.stderr}")
    if "shell-4" not in listed.stdout:
        fail(f"`anthrex ls` did not list shell-4:\n{listed.stdout}")
    if ls_elapsed > 1.5:
        fail(f"`anthrex ls` took {ls_elapsed:.2f}s after the paste (limit 1.5s)")

    started = time.monotonic()
    proc3.send(b"\x02d")
    status3 = proc3.wait_exit(timeout=5.0)
    detach_elapsed = time.monotonic() - started
    if not os.WIFEXITED(status3) or os.WEXITSTATUS(status3) != 0:
        fail(f"detach after the large paste did not exit cleanly (raw status {status3})")
    if detach_elapsed > 2.0:
        fail(f"detach after a 32 KiB paste took {detach_elapsed:.2f}s; the client froze behind the PTY write")
    if time.monotonic() - child_started >= 30:
        fail("non-reading child's 30s lifetime expired before responsiveness checks finished")
    proc3.close()
    print(
        f"ok: daemon answered ls in {ls_elapsed:.2f}s and the client detached in "
        f"{detach_elapsed:.2f}s after a 32 KiB paste to a ready, non-reading child"
    )

    print("== stage 8b: a fake Claude turn reports working, tool, and done ==")
    proc4 = PtyProc([BIN, "attach", "shell-1"])
    proc4.wait_for("agents", label="fourth attach banner")
    proc4.wait_for("smoke-42", label="shell-1 focused before fake Claude creation")
    created = run_cmd(["new", "--runtime", "claude", "--name", "fake-claude"])
    try:
        fake_claude_id = int(created.stdout.strip())
    except ValueError:
        fail(f"`anthrex new` did not print a window id: {created.stdout!r}")
    deadline = time.monotonic() + 3.0
    working = None
    while time.monotonic() < deadline:
        listed = run_cmd(["ls", "--json"], timeout=max(0.01, deadline - time.monotonic()))
        working = next((w for w in json.loads(listed.stdout) if w["id"] == fake_claude_id), None)
        if working and working["status"] == "working" and working["tool"] == "Bash":
            break
        proc4.read_available(timeout=0.05)
    else:
        fail(f"fake-claude never reported working with Bash: {working!r}")
    deadline = time.monotonic() + 3.0
    while not re.search(r"fake-claude\s+cl\b", proc4.screen_text()):
        if time.monotonic() >= deadline:
            fail(f"fake-claude tree row/runtime tag did not appear:\n{proc4.screen_text()}")
        proc4.read_available(timeout=0.05)
    print("ok: fake-claude tree row appeared and JSON reported working with Bash")
    proc4.wait_for("fake-claude finished", label="fake-claude completion toast")

    listed_json = run_cmd(["ls", "--json"])
    try:
        windows = json.loads(listed_json.stdout)
    except json.JSONDecodeError as error:
        fail(f"`anthrex ls --json` returned invalid JSON ({error}):\n{listed_json.stdout}")
    fake_claude = next((window for window in windows if window["id"] == fake_claude_id), None)
    if fake_claude is None:
        fail(f"`anthrex ls --json` omitted fake-claude id {fake_claude_id}:\n{listed_json.stdout}")
    if fake_claude["status"] != "done":
        fail(f"fake-claude status was not done:\n{listed_json.stdout}")
    if fake_claude["tool"] is not None:
        fail(f"fake-claude tool was not cleared after completion:\n{listed_json.stdout}")
    expected_session = f"fake-session-{fake_claude_id}"
    if fake_claude["session_id"] != expected_session:
        fail(f"fake-claude session id was not {expected_session!r}:\n{listed_json.stdout}")
    print("ok: completion toast and JSON done/session metadata appeared with the tool cleared")

    run_cmd(["rm", "fake-claude"])
    remaining = json.loads(run_cmd(["ls", "--json"]).stdout)
    remaining_names = {window["name"] for window in remaining}
    if remaining_names != {"shell-1", "shell-2", "shell-3", "shell-4"}:
        fail(f"unexpected windows after removing fake-claude: {sorted(remaining_names)}")
    proc4.send(b"\x02d")
    status4 = proc4.wait_exit(timeout=5.0)
    if not os.WIFEXITED(status4) or os.WEXITSTATUS(status4) != 0:
        fail(f"fourth detach did not exit cleanly with status 0 (raw status {status4})")
    proc4.close()
    print("ok: fake-claude removed and fourth client detached cleanly")

    run_project_tree_stage(REPO, PtyProc, run_cmd, fail)
    run_tree_connectors_stage(REPO, PtyProc, run_cmd, fail, FAKE_AGENT_SCRIPT)
    run_graph_glyphs_stage(REPO, PtyProc, run_cmd, fail)
    run_inspector_stage(REPO, PtyProc, run_cmd, fail, FAKE_AGENT_SCRIPT)

    smoke_repo = make_repo()
    run_worktree_cli_stage(smoke_repo)
    run_worktree_form_stage(smoke_repo)

    print("== stage 9 stop: stop the daemon ahead of the persistence stages ==")
    # The persistence and reconnect stages below need to observe the daemon actually
    # dying and coming back, so this is the same stop-and-verify shape the old final
    # stage used, just moved earlier: it is the "last stop" the brief calls stage 9,
    # kept in place immediately before the stages that depend on it.
    stop_result = run_cmd(["daemon", "stop"])
    print(f"daemon stop output: {stop_result.stdout.strip()!r}")
    status_result = run_cmd(["daemon", "status"])
    if "not running" not in status_result.stdout:
        fail(
            "`anthrex daemon status` did not report not running ahead of the "
            f"persistence stages:\n{status_result.stdout}"
        )
    print("ok: daemon stopped ahead of the persistence and reconnect stages")

    print("== stage 10: persistence and restart ==")
    run_cmd(["daemon", "start"])
    deadline = time.monotonic() + 5.0
    windows_by_name = {}
    while True:
        windows_by_name = {w["name"]: w for w in json.loads(run_cmd(["ls", "--json"]).stdout)}
        if all(
            windows_by_name.get(name, {}).get("status") == "exited"
            for name in ("shell-1", "shell-2", "shell-3", "shell-4")
        ):
            break
        if time.monotonic() >= deadline:
            fail(
                "shell-1..4 were not all listed with status 'exited' within 5s of the "
                f"daemon restarting:\n{windows_by_name!r}"
            )
        time.sleep(0.1)

    run_cmd(["rename", "shell-1", "kept"])
    renamed = run_cmd(["ls"]).stdout
    if "kept" not in renamed:
        fail(f"`anthrex ls` did not show 'kept' after renaming shell-1:\n{renamed}")

    run_cmd(["restart", "kept"])

    proc5 = PtyProc([BIN])
    proc5.wait_for("agents", label="stage-10 attach banner")
    proc5.send(b"\x021")
    proc5.wait_for_focused_window("kept")
    proc5.send(b"echo back-$((1+1))\r")
    proc5.wait_for("back-2", label="restarted 'kept' shell echo")
    proc5.send(b"\x02d")
    status5 = proc5.wait_exit(timeout=5.0)
    if not os.WIFEXITED(status5) or os.WEXITSTATUS(status5) != 0:
        fail(f"stage-10 detach did not exit cleanly with status 0 (raw status {status5})")
    proc5.close()
    print(
        "ok: a daemon restart marked shell-1..4 exited, 'kept' kept its name and its "
        "restarted shell echoed a command"
    )

    print("== stage 11: reconnect ==")
    proc6 = PtyProc([BIN])
    proc6.wait_for("agents", label="stage-11 attach banner")
    proc6.wait_for("kept", label="'kept' listed on stage-11 attach")
    run_cmd(["daemon", "stop"])
    proc6.wait_for(
        "DISCONNECTED", label="disconnected badge after the daemon stopped under the client"
    )
    run_cmd(["daemon", "start"])
    # 10s is the brief's own literal bound, kept verbatim — it is not derived from a
    # single constant, and in particular it is *not* "five attempts at RETRY_INTERVAL"
    # (fix-wave-11 review, Minor: that was this comment's previous, wrong
    # justification). The client's actual legal retry budget is RETRY_WINDOW, 30s
    # (crates/tui/src/reconnect.rs) — the ceiling after which the client gives up
    # retrying automatically; RETRY_INTERVAL, 2s (same file), only says how often it
    # tries within that window, and citing it alone understates how long a legitimate
    # reconnect is allowed to take by 3x. What this bound actually races is
    # `ensure_daemon`'s own ~3s socket-wait deadline (crates/tui/src/spawn.rs) plus
    # about one more RETRY_INTERVAL — a realistic ~5s, not RETRY_WINDOW's full 30s.
    # 10s held with room to spare in every run measured, including under deliberate
    # CPU contention, but a daemon slow enough to miss it would still be legitimately
    # retrying inside its documented 30s RETRY_WINDOW — a false failure in this
    # script, not a real bug in the client.
    deadline = time.monotonic() + 10.0
    screen = proc6.screen_text()
    while "DISCONNECTED" in screen or "kept" not in screen:
        if time.monotonic() >= deadline:
            fail(
                "reconnect did not clear DISCONNECTED and keep 'kept' listed within "
                f"10s:\n{screen}"
            )
        proc6.read_available(timeout=0.2)
        screen = proc6.screen_text()
    proc6.send(b"\x02d")
    status6 = proc6.wait_exit(timeout=5.0)
    if not os.WIFEXITED(status6) or os.WEXITSTATUS(status6) != 0:
        fail(f"stage-11 detach did not exit cleanly with status 0 (raw status {status6})")
    proc6.close()
    print(
        "ok: the client showed DISCONNECTED across a daemon stop/start cycle, "
        "reconnected, and kept 'kept' listed"
    )

    print("== stage 11b: the client survives its own give-up state and C-b r revives it ==")
    # Whole-branch-review Critical 1: `ConnectionDriver::step`'s `select!` used to have
    # no `else` arm, and decision 31's give-up path (automatic retries exhausted after
    # RETRY_WINDOW, 30s — crates/tui/src/reconnect.rs) sets every one of that
    # `select!`'s three guards false at once. Every other reconnect coverage in this
    # suite (stage 11 above) and in `crates/tui/src/reconnect_tests.rs` brings the
    # daemon back inside that window, so none of it ever reached the all-disabled
    # state — this stage is the one place anything actually lets the full window
    # elapse against the real binary. It also stands in for the CPU measurement no
    # unit test can make: `else => vec![]` (the obvious, wrong fix) returns
    # immediately on every poll and spins `event_loop`'s redraw-every-pass loop at
    # ~95-97% CPU with a normal-looking status bar, which this stage's sampling would
    # catch and a mere "is it still alive" check would not.
    proc6b = PtyProc([BIN])
    proc6b.wait_for("agents", label="stage-11b attach banner")
    proc6b.wait_for("kept", label="'kept' listed on stage-11b attach")
    run_cmd(["daemon", "stop"])
    link_lost_at = time.monotonic()
    proc6b.wait_for(
        "DISCONNECTED",
        label="disconnected badge after the daemon stopped under the client (stage 11b)",
    )

    # RETRY_WINDOW (crates/tui/src/reconnect.rs) is 30s from link loss. Wait past it
    # with margin, without ever starting a daemon back up, so the give-up path is the
    # only way out — then confirm the process is still alive at all (pre-fix, it
    # panicked in the same frame the badge below appeared, rc 101) and is showing
    # decision 34's give-up status line, not still "reconnecting".
    giveup_deadline = link_lost_at + 33.0
    while time.monotonic() < giveup_deadline:
        proc6b.read_available(timeout=0.5)
    pid, status = os.waitpid(proc6b.pid, os.WNOHANG)
    if pid == proc6b.pid:
        fail(
            "the client exited on its own while giving up on reconnecting "
            f"(raw status {status}) — this is the select!-with-no-else panic"
        )
    screen = proc6b.screen_text()
    if "C-b r to reconnect" not in screen:
        fail(f"give-up status line did not appear after RETRY_WINDOW elapsed:\n{screen}")
    print("ok: the client survived past RETRY_WINDOW and shows the give-up status line")

    # Sample CPU while parked, twice a couple of seconds apart. macOS's `ps %cpu` is a
    # decaying average, so one reading right after the retries above could still carry
    # some of their cost; two readings, taken after the process has had nothing to do
    # for several seconds, catch a genuine busy loop (measured 95-97% for the
    # `else => vec![]` shape) without being tunable-flaky the way a tight bound on a
    # single instantaneous sample would be.
    cpu_samples = []
    for _ in range(2):
        time.sleep(2.0)
        try:
            sample = subprocess.run(
                ["ps", "-o", "%cpu=", "-p", str(proc6b.pid)],
                capture_output=True,
                text=True,
                timeout=5,
            )
            cpu_samples.append(float(sample.stdout.strip() or "0"))
        except (subprocess.TimeoutExpired, ValueError) as error:
            fail(f"could not sample CPU for the parked client (pid {proc6b.pid}): {error}")
    print(f"give-up-state CPU samples (ps %cpu, 2s apart): {cpu_samples}")
    if any(sample > 25.0 for sample in cpu_samples):
        fail(
            f"parked give-up state used {cpu_samples} CPU%; a busy loop (the "
            "`else => vec![]` shape this stage guards against) reads 95-97%, so this "
            "is a regression even though the process did not panic"
        )
    print("ok: the parked give-up state used negligible CPU (no busy loop)")

    # Decision 32: a manual `C-b r` must still re-arm the driver from `Link::Lost`.
    run_cmd(["daemon", "start"])
    proc6b.send(b"\x02r")
    reconnect_deadline = time.monotonic() + 10.0
    screen = proc6b.screen_text()
    while "DISCONNECTED" in screen or "kept" not in screen:
        if time.monotonic() >= reconnect_deadline:
            fail(
                "C-b r from Link::Lost did not clear DISCONNECTED and keep 'kept' "
                f"listed within 10s:\n{screen}"
            )
        proc6b.read_available(timeout=0.2)
        screen = proc6b.screen_text()
    print("ok: C-b r re-armed the driver from Link::Lost and reconnected")

    # The user must still be able to quit from here, same as any other state.
    proc6b.send(b"\x02d")
    status6b = proc6b.wait_exit(timeout=5.0)
    if not os.WIFEXITED(status6b) or os.WEXITSTATUS(status6b) != 0:
        fail(f"stage-11b detach did not exit cleanly with status 0 (raw status {status6b})")
    proc6b.close()
    print("ok: the client could still detach cleanly after giving up and reconnecting")

    print("== stage 12: stop the daemon, verify status ==")
    stop_result = run_cmd(["daemon", "stop"])
    print(f"daemon stop output: {stop_result.stdout.strip()!r}")
    status_result = run_cmd(["daemon", "status"])
    if "not running" not in status_result.stdout:
        fail(f"`anthrex daemon status` did not report not running:\n{status_result.stdout}")
    print("ok: daemon stopped and status reports not running")

    run_resume_stage()

    print("== stage 13: quit through the TUI with C-b Q ==")
    # The brief's stages 10 and 11 above only ever stop the daemon from the CLI. This
    # stage presses the most destructive key the TUI has - the one that stops the
    # daemon and kills every agent under it - and is the only thing in the project
    # that does. A real Critical hid from every unit test in exactly this key: the
    # event loop's read arm used to be gated on a flag `DaemonMsg::Bye`'s handler
    # cleared before the socket had actually closed, so `on_link_lost` (and the
    # `Effect::Quit` it produces for a pending `C-b Q`) never ran and the key just
    # hung. The brief's own unit test called the handler directly and never drove the
    # real event loop, so it never saw the hang. Only a real daemon, a real TUI and a
    # real PTY - this script - could have caught it.
    proc7 = PtyProc([BIN])
    proc7.wait_for("agents", label="tui-quit attach banner")
    proc7.send(b"\x02Q")
    proc7.wait_for(
        "Stop the daemon and kill every agent?", label="stop-daemon confirmation modal"
    )
    proc7.send(b"y")
    status7 = proc7.wait_exit(timeout=TUI_QUIT_TIMEOUT)
    if not os.WIFEXITED(status7) or os.WEXITSTATUS(status7) != 0:
        fail(f"C-b Q did not exit the client cleanly with status 0 (raw status {status7})")
    proc7.close()

    # The client quits as soon as it observes the link drop, which can land slightly
    # ahead of the daemon finishing its own teardown (releasing its lifetime lock and
    # unlinking the socket only as its very last act - see `stop_daemon`'s docstring
    # above). Same generous-not-tight reasoning as `TUI_QUIT_TIMEOUT`.
    deadline = time.monotonic() + TUI_QUIT_TIMEOUT
    while os.path.exists(SOCKET):
        if time.monotonic() >= deadline:
            fail(
                f"the daemon socket {SOCKET} was still present {TUI_QUIT_TIMEOUT}s "
                "after C-b Q quit the client"
            )
        time.sleep(0.1)
    status_result = run_cmd(["daemon", "status"])
    if "not running" not in status_result.stdout:
        fail(f"`anthrex daemon status` did not report not running after C-b Q:\n{status_result.stdout}")
    print("ok: C-b Q quit the client through the TUI, and the daemon it stopped is gone")

    print("\nALL SMOKE STAGES PASSED")


if __name__ == "__main__":
    try:
        main()
    finally:
        # Always try to stop the daemon, even on failure, so nothing is left running.
        try:
            stop_daemon()
        except Exception:
            pass
        # Leave nothing behind under /tmp. ANTHREX_SMOKE_KEEP=1 keeps the data
        # directory (and so daemon.log) for debugging a failure.
        if not os.environ.get("ANTHREX_SMOKE_KEEP"):
            shutil.rmtree(DATA_DIR, ignore_errors=True)
        # W1 and W2's fixture repository is not the daemon's data, so it is removed
        # unconditionally, keep-flag or not.
        shutil.rmtree(SMOKE_REPO, ignore_errors=True)
        # ANTHREX_CONFIG's own fixed-path directory is deliberately *not* removed
        # here. Unlike DATA_DIR and SMOKE_REPO, this run does not necessarily own
        # whatever is (or isn't) at that path — `ensure_config_path_absent` may have
        # failed before anything below ever ran, in which case a stray file there was
        # never this run's to begin with, and unconditionally `rmtree`-ing it here
        # would silently delete it anyway, defeating that check's whole point. The one
        # stage that does create a real file there (the resume stage below) owns its
        # own cleanup instead, in its own try/finally, precisely so this block never
        # has to guess whether a given run is the one that created it.
