"""The dispatch-hold stage for the PTY smoke test (milestone 9.16, task M9.16.11): stage
11o.

`scripts/pty-smoke.py` imports `brakes_stage` and calls it with its own `PtyProc`, `BIN`,
`run_cmd`, `fail` and `ENV`, after stage 11l (`profile_stage`) and before `tui_stage`.
Like stages 11g and 11n, it runs a daemon of its own: its own socket, data directory,
`ANTHREX_CONFIG` and `CODEX_HOME` under its own `/tmp` root, a nonexistent
`ANTHREX_GH_BIN`, and `fake-agent` as both runtimes and the decider (from the script's
`ENV`), so no real agent or `gh` can start, and the hold it sets can never reach the
shared daemon the other stages use. Its config keeps U11's brake defaults (ruling C19:
the quota brake on); only lessons are off. The stage always ends with `anthrex daemon
stop` under the same variables, also when it fails, and removes its root.

Decision 32 from a real terminal: `C-b H`, `y` puts ` HELD ` in the status bar;
`anthrex hold status`, in a second PTY, prints `hold: on`; `C-b H`, `y` again releases
it and the badge goes. Every wait is a deadline loop on the rendered screen or the
process's exit. `ANTHREX_SMOKE_ONLY=11o python3 scripts/pty-smoke.py` runs it alone.
"""

import os
import shutil
import subprocess
import time

from pty_smoke_pr import DAEMON_START_WAIT, _stop_daemon
from pty_smoke_run import POLL
from smoke_task_tmp import teardown

# A key's own frame: the client redraws on its 100 ms tick, and the confirm is built
# from the snapshot it already holds (`pty_smoke_tui.py`'s `SCREEN_WAIT`).
SCREEN_WAIT = 10.0

# The badge after `y`: the daemon saves `brakes.json` within `BRAKES_IO_WAIT` (5 s,
# `crates/daemon/src/run/driver/brakes_io.rs`), steps the engine and publishes the
# snapshot the client draws; 5 s plus the frame's 10 s.
HOLD_WAIT = 15.0

# `anthrex hold status`'s legal worst case: `ensure_daemon` (`SPAWN_HANDOFF_GRACE` 0.25 s
# + `ENSURE_DAEMON_SOCKET_WAIT` 10 s), `HANDSHAKE_TIMEOUT` (5 s), then its one `List`
# within `HOLD_REQUEST_TIMEOUT` (`RUN_REQUEST_TIMEOUT`, 180 s; `crates/cli/src/hold_cmd.rs`,
# ruling C34): 195.25 s. A language boundary, so the coupling is this comment.
HOLD_STATUS_WAIT = 210.0

# The client's exit after `C-b d`: local, as in every other stage.
DETACH_WAIT = 5.0

# The confirm's questions (`crates/tui/src/app/brakes.rs`: `HOLD_ASK`, `RELEASE_ASK`).
HOLD_ASK = "Hold dispatch daemon-wide?"
RELEASE_ASK = "Release dispatch?"
# `crates/tui/src/ui/statusbar.rs`'s `HELD_BADGE`.
HELD_BADGE = " HELD "

CONFIG = """# Milestone 9.15 ruling C3: lessons off. The quota brake keeps U11's defaults.
[lessons]
enabled = false
risk_note = false
"""


def _status_bar(proc):
    """The rendered screen's bottom row: the status bar."""
    return proc.screen_text().split("\n")[-1]


def _wait_bar(proc, held, fail, timeout=HOLD_WAIT):
    """Waits, within `timeout`, until the status bar shows the badge (`held`) or not."""
    deadline = time.monotonic() + timeout
    while (HELD_BADGE in _status_bar(proc)) != held:
        if time.monotonic() >= deadline:
            want = "shows" if held else "no longer shows"
            fail(
                f"stage 11o: the status bar {want} {HELD_BADGE!r} within {timeout}s\n"
                f"--- rendered screen ---\n{proc.screen_text()}"
            )
        proc.read_available(timeout=0.2)


def _hold_status(pty_proc, bin_path, env, fail):
    """`anthrex hold status` in a PTY of its own: its rendered output, once it exited 0."""
    proc = pty_proc([bin_path, "hold", "status"], env=env)
    try:
        status = proc.wait_exit(timeout=HOLD_STATUS_WAIT)
        # Whatever it wrote before it exited.
        while proc.read_available(timeout=0.2):
            pass
        if not (os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0):
            fail(f"stage 11o: `anthrex hold status` exited with {status}:\n{proc.screen_text()}")
        return proc.screen_text()
    finally:
        proc.close()


def _toggle(proc, ask, fail):
    """`C-b H`: waits for the confirm asking `ask`, then confirms it with `y`."""
    proc.send(b"\x02H")
    proc.wait_for(ask, timeout=SCREEN_WAIT, label=f"stage-11o confirm {ask!r}")
    proc.send(b"y")


def brakes_stage(pty_proc, bin_path, run_cmd, fail, base_env):
    print("== stage 11o: C-b H holds and releases dispatch ==")
    root = f"/tmp/anthrex-smoke-brakes-{os.getpid()}"
    socket = os.path.join(root, "d.sock")
    config = os.path.join(root, "config.toml")
    env = dict(base_env)
    env.update(
        {
            "ANTHREX_SOCKET": socket,
            "ANTHREX_DATA_DIR": os.path.join(root, "data"),
            "ANTHREX_CONFIG": config,
            "CODEX_HOME": os.path.join(root, "codex-home"),
            # Never a real `gh`; the runtimes and the decider stay `fake-agent` (`ENV`).
            "ANTHREX_GH_BIN": "/nonexistent/anthrex-smoke/gh",
            "FAKE_AGENT_DECIDER_DIR": os.path.join(root, "deciders"),
            "FAKE_AGENT_SCRIPT": os.path.join(root, "no-such-script.json"),
            "FAKE_AGENT_TRANSCRIPT": os.path.join(root, "transcript.jsonl"),
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
        }
    )
    if os.path.exists(root):
        fail(f"stage 11o: {root} already exists, and is not this stage's to remove")
    daemon = None
    proc = None
    try:
        os.makedirs(root)
        with open(config, "w") as f:
            f.write(CONFIG)

        # The daemon is this stage's own child, in its own session, as stage 11g's.
        daemon = subprocess.Popen(
            [bin_path, "daemon", "start", "--foreground"],
            cwd=root,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        deadline = time.monotonic() + DAEMON_START_WAIT
        while not os.path.exists(socket) and daemon.poll() is None:
            if time.monotonic() >= deadline:
                fail(f"stage 11o: its daemon did not bind {socket} within {DAEMON_START_WAIT}s")
            time.sleep(POLL)
        if daemon.poll() is not None:
            fail(f"stage 11o: its daemon exited at its start ({daemon.returncode})")

        proc = pty_proc([bin_path], env=env)
        proc.wait_for("agents", timeout=SCREEN_WAIT, label="stage-11o attach banner")
        _wait_bar(proc, False, fail, timeout=SCREEN_WAIT)

        # 1. Hold: the badge, and the CLI agrees from another terminal.
        _toggle(proc, HOLD_ASK, fail)
        _wait_bar(proc, True, fail)
        print("ok: C-b H, y put the HELD badge in the status bar")
        text = _hold_status(pty_proc, bin_path, env, fail)
        if "hold: on since " not in text or "(user)" not in text:
            fail(f"stage 11o: `anthrex hold status` did not print `hold: on`:\n{text}")
        print("ok: anthrex hold status in a second PTY printed hold: on")

        # 2. Release: the badge goes, and the CLI says so.
        _toggle(proc, RELEASE_ASK, fail)
        _wait_bar(proc, False, fail)
        text = _hold_status(pty_proc, bin_path, env, fail)
        if "hold: off" not in text:
            fail(f"stage 11o: `anthrex hold status` did not print `hold: off`:\n{text}")
        print("ok: C-b H, y released dispatch and the badge is gone")

        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=DETACH_WAIT)
        if not (os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0):
            fail(f"stage 11o: detach did not exit cleanly with status 0 (raw status {status})")
    finally:
        if proc is not None:
            proc.close()
        _stop_daemon(run_cmd, env, socket, daemon, "11o")
        if not os.environ.get("ANTHREX_SMOKE_KEEP"):
            teardown(root)
            shutil.rmtree(root, ignore_errors=True)
