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

from pty_tree_smoke import run_project_tree_stage


REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(REPO, "target/debug/anthrex")
FAKE_AGENT_BIN = os.path.join(REPO, "target/debug/fake-agent")
DATA_DIR = tempfile.mkdtemp(prefix="anthrex-smoke-", dir="/tmp")
SOCKET = os.path.join(DATA_DIR, "daemon.sock")
FAKE_AGENT_SCRIPT = os.path.join(DATA_DIR, "fake-agent.jsonl")
ROWS, COLS = 40, 120

ENV = dict(os.environ)
ENV["ANTHREX_SOCKET"] = SOCKET
ENV["ANTHREX_DATA_DIR"] = DATA_DIR
ENV["ANTHREX_CLAUDE_BIN"] = FAKE_AGENT_BIN
ENV["ANTHREX_CODEX_BIN"] = FAKE_AGENT_BIN
ENV["FAKE_AGENT_SCRIPT"] = FAKE_AGENT_SCRIPT
ENV["TERM"] = "xterm-256color"


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


def run_cmd(args, expect_ok=True, timeout=15):
    result = subprocess.run([BIN] + args, cwd=REPO, env=ENV, capture_output=True, text=True, timeout=timeout)
    if expect_ok and result.returncode != 0:
        fail(f"`anthrex {' '.join(args)}` exited {result.returncode}\nstdout: {result.stdout}\nstderr: {result.stderr}")
    return result


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

    `anthrex daemon stop` returns as soon as the daemon closes its listener, but the
    daemon still has to end every agent process group with SIGHUP (escalating if needed)
    and removes the socket file only as its very last act. Returning before that lets the
    next run bind a socket at the same path that the dying daemon then unlinks out from it.
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


def main():
    ensure_binary()
    write_fake_agent_script()

    print("== stage 1: initial attach ==")
    proc = PtyProc([BIN])
    proc.wait_for("agents", label="initial banner")
    proc.wait_for("no agents yet", label="empty sidebar hint")
    print("ok: initial frame shows anthrex UI with no agents yet")

    print("== stage 2: create shell-1, run a command ==")
    proc.send(b"\x02c")
    proc.wait_for_focused_window("shell-1")
    proc.send(b"echo smoke-$((40+2))\r")
    proc.wait_for("smoke-42", label="echo output in shell-1")
    print("ok: shell-1 created and command output visible")

    print("== stage 3: create shell-2, exercise keys, help overlay ==")
    proc.send(b"\x02c")
    proc.wait_for_focused_window("shell-2")
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
    proc3.send(b"\x02c")
    proc3.wait_for_focused_window("shell-3")
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
    proc3.send(b"\x02c")
    proc3.wait_for_focused_window("shell-4")
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

    print("== stage 9: stop the daemon, verify status ==")
    stop_result = run_cmd(["daemon", "stop"])
    print(f"daemon stop output: {stop_result.stdout.strip()!r}")
    status_result = run_cmd(["daemon", "status"])
    if "not running" not in status_result.stdout:
        fail(f"`anthrex daemon status` did not report not running:\n{status_result.stdout}")
    print("ok: daemon stopped and status reports not running")

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
