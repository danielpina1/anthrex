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
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import termios
import time


REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(REPO, "target/debug/anthrex")
SOCKET = "/tmp/anthrex-smoke.sock"
DATA_DIR = "/tmp/anthrex-smoke-data"
ROWS, COLS = 40, 120

ENV = dict(os.environ)
ENV["ANTHREX_SOCKET"] = SOCKET
ENV["ANTHREX_DATA_DIR"] = DATA_DIR
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

    def send(self, data: bytes):
        os.write(self.fd, data)

    def send_large(self, data: bytes, timeout=5.0):
        """Writes a lot of bytes, draining output in between so neither side deadlocks.

        A client that has stopped reading its own stdin would make this block; that is
        itself a failure, so it is bounded by `timeout`.
        """
        deadline = time.monotonic() + timeout
        view = memoryview(data)
        while view:
            if time.monotonic() > deadline:
                fail(f"writing to the client's pty blocked with {len(view)} bytes left")
            _, w, _ = select.select([], [self.fd], [], 0.2)
            if self.fd in w:
                written = os.write(self.fd, view[:4096])
                view = view[written:]
            self.read_available(timeout=0.0)

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


def run_cmd(args, expect_ok=True):
    result = subprocess.run([BIN] + args, cwd=REPO, env=ENV, capture_output=True, text=True, timeout=15)
    if expect_ok and result.returncode != 0:
        fail(f"`anthrex {' '.join(args)}` exited {result.returncode}\nstdout: {result.stdout}\nstderr: {result.stderr}")
    return result


def ensure_binary():
    if os.path.exists(BIN):
        return
    print(f"== building {BIN} ==")
    result = subprocess.run(["cargo", "build"], cwd=REPO, timeout=900)
    if result.returncode != 0 or not os.path.exists(BIN):
        fail("`cargo build` did not produce target/debug/anthrex")


def reset_state():
    """Starts from a clean slate: no leftover daemon, no leftover socket or data dir."""
    subprocess.run([BIN, "daemon", "stop"], cwd=REPO, env=ENV, capture_output=True, text=True, timeout=30)
    shutil.rmtree(DATA_DIR, ignore_errors=True)
    try:
        os.unlink(SOCKET)
    except OSError:
        pass


def main():
    ensure_binary()
    reset_state()

    print("== stage 1: initial attach ==")
    proc = PtyProc([BIN])
    proc.wait_for("anthrex", label="initial banner")
    proc.wait_for("no agents yet", label="empty sidebar hint")
    print("ok: initial frame shows anthrex UI with no agents yet")

    print("== stage 2: create shell-1, run a command ==")
    proc.send(b"\x02c")
    proc.wait_for("shell-1", label="shell-1 card")
    proc.send(b"echo smoke-$((40+2))\r")
    proc.wait_for("smoke-42", label="echo output in shell-1")
    print("ok: shell-1 created and command output visible")

    print("== stage 3: create shell-2, exercise keys, help overlay ==")
    proc.send(b"\x02c")
    proc.wait_for("shell-2", label="shell-2 card")
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
    proc2.wait_for("anthrex", label="re-attach banner")
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
    proc3.wait_for("anthrex", label="third attach banner")
    proc3.send(b"\x02c")
    proc3.wait_for("shell-3", label="shell-3 card")
    proc3.send(b"cat -v\r")
    time.sleep(0.5)
    proc3.read_available(timeout=0.5)
    proc3.send(b"\x1b\r")  # ESC CR: how a terminal reports Alt+Enter
    proc3.wait_for("^[", label="ESC rendered by cat -v in the focused window")
    proc3.send(b"\x04")  # Ctrl-D ends cat
    time.sleep(0.3)
    proc3.read_available(timeout=0.3)
    print("ok: Alt+Enter arrived at the child as ESC CR")

    print("== stage 8: a 32 KiB paste freezes neither the daemon nor the client ==")
    # Regression for the blocked-PTY-write fix. `stty raw -echo` is what every full-screen
    # agent does: no canonical line discipline draining the input queue and no echo, so
    # the PTY master write stalls after about a kilobyte. The daemon used to perform that
    # write inline, on a tokio worker, while holding the window-table mutex - so for the
    # length of the stall NOTHING else worked: no other client, no tick, no kill, no
    # shutdown. `anthrex ls` from outside is the sharpest probe of that, because it needs
    # exactly the mutex the stalled write was holding.
    proc3.send(b"\x02c")
    proc3.wait_for("shell-4", label="shell-4 card")
    proc3.send(b"stty raw -echo; sleep 5\r")
    time.sleep(0.8)
    proc3.read_available(timeout=0.5)
    paste = b"\x1b[200~" + b"x" * (32 * 1024) + b"\x1b[201~"
    proc3.send_large(paste)

    started = time.monotonic()
    try:
        listed = subprocess.run([BIN, "ls"], cwd=REPO, env=ENV, capture_output=True, text=True, timeout=5)
    except subprocess.TimeoutExpired:
        fail("`anthrex ls` never returned while a paste was stalled in a PTY write")
    ls_elapsed = time.monotonic() - started
    if listed.returncode != 0:
        fail(f"`anthrex ls` failed during the stalled paste: {listed.stderr}")
    if "shell-4" not in listed.stdout:
        fail(f"`anthrex ls` did not list shell-4:\n{listed.stdout}")
    if ls_elapsed > 1.5:
        fail(f"`anthrex ls` took {ls_elapsed:.2f}s; the daemon was blocked behind the PTY write")

    started = time.monotonic()
    proc3.send(b"\x02d")
    status3 = proc3.wait_exit(timeout=5.0)
    detach_elapsed = time.monotonic() - started
    if not os.WIFEXITED(status3) or os.WEXITSTATUS(status3) != 0:
        fail(f"detach after the large paste did not exit cleanly (raw status {status3})")
    if detach_elapsed > 2.0:
        fail(f"detach after a 32 KiB paste took {detach_elapsed:.2f}s; the client froze behind the PTY write")
    proc3.close()
    print(
        f"ok: daemon answered ls in {ls_elapsed:.2f}s and the client detached in "
        f"{detach_elapsed:.2f}s while 32 KiB sat unread in the PTY"
    )

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
            subprocess.run([BIN, "daemon", "stop"], cwd=REPO, env=ENV, capture_output=True, text=True, timeout=10)
        except Exception:
            pass
        # Leave nothing behind under /tmp.
        shutil.rmtree(DATA_DIR, ignore_errors=True)
        try:
            os.unlink(SOCKET)
        except OSError:
            pass
