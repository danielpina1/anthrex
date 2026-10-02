"""The run-view stage for the PTY smoke test (milestone 8c, task 10): stage 11e.

`scripts/pty-smoke.py` imports `run_view_stage` and calls it with its own `PtyProc`,
`BIN`, `run_cmd` and `fail`, right after stage 11d, so every `anthrex` here runs with
that script's isolated `ENV`: `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under `/tmp`,
and `fake-agent` as both runtimes and as the decider (`ANTHREX_CLAUDE_BIN`,
`ANTHREX_CODEX_BIN`, `ANTHREX_DECIDER_BIN`). Nothing here starts or stops a daemon of
its own. No client is attached when the stage starts (stage 11b's detached), so it
starts its own and detaches it. Like `scripts/pty_smoke_run.py`, whose helpers it
reuses, this module has no `__main__` of its own.
"""

import json
import os
import shutil
import sys
import time

from pty_smoke_run import (
    ACCEPT_CMD_TIMEOUT,
    POLL,
    RUN_CMD_TIMEOUT,
    RUN_WAIT,
    _git,
    _run_state,
    _write_script,
)

# The conversation view's worst case, derived as `CONVERSATION_VIEW_TIMEOUT` in
# `scripts/pty-smoke.py` is (a hyphenated script cannot be imported, so the coupling
# is this comment: change that derivation, change this value):
#   fake-agent runs 5 hook steps, each bounded by STEP_TIMEOUT (5s,
#     crates/fake-agent/src/main.rs)                                        = 25s
#   + the daemon's transcript poll interval (TRANSCRIPT_POLL, 250ms,
#     crates/daemon/src/conversation/watch.rs)                              =  0.25s
#   + one transcript read pass (TRANSCRIPT_READ_TIMEOUT, 2s,
#     crates/daemon/src/transcript/reader.rs)                               =  2s
#   + the client's own event-loop tick (100ms, crates/tui/src/lib.rs)       =  0.1s
#                                                                          = 27.35s
# 45s keeps roughly the same ~60% margin. What this stage waits on is smaller: the
# header names the window as soon as the client lists it and draws one frame, and a
# headless conversation has no transcript to poll; the bound above covers it.
RUN_VIEW_CONVERSATION_TIMEOUT = 45.0

# How often the stage presses Enter again while the worker's window is not listed yet
# (Enter on a round whose window is unlisted only toasts, M8c.6).
ENTER_RETRY = 3.0

PLAN = """goal = "View a"

[profile]
check = "true"

[[task]]
id = "t1"
title = "add a"
size = "S"
test_mode = "check"
test_mode_reason = "smoke"
owns = ["a.txt"]
brief = "Do t1"
acceptance = ["t1 is done"]
route = { runtime = "claude" }
"""

# A turn that stays open, so the worker round is live while the stage looks at it.
# The run is cancelled long before the stall watchdog's default 600 s.
WORKER = [{"wait_ms": 600000}]


def _open_conversation(proc, name, fail):
    """Enter on the selected worker round until its conversation's header names
    `name`, within `RUN_VIEW_CONVERSATION_TIMEOUT`."""
    deadline = time.monotonic() + RUN_VIEW_CONVERSATION_TIMEOUT
    next_enter = 0.0
    while True:
        if name in proc.screen_text():
            return
        now = time.monotonic()
        if now >= deadline:
            fail(
                f"the worker's conversation naming {name!r} did not open within "
                f"{RUN_VIEW_CONVERSATION_TIMEOUT}s\n--- rendered screen ---\n{proc.screen_text()}"
            )
        if now >= next_enter:
            proc.send(b"\r")
            next_enter = now + ENTER_RETRY
        proc.read_available(timeout=0.2)


def _wait_gone(proc, text, label, fail, timeout=10.0):
    """Waits, within `timeout`, until `text` is no longer on the rendered screen."""
    deadline = time.monotonic() + timeout
    while text in proc.screen_text():
        if time.monotonic() >= deadline:
            fail(f"{label} still showed {text!r} after {timeout}s\n--- rendered screen ---\n{proc.screen_text()}")
        proc.read_available(timeout=0.2)


def run_view_stage(pty_proc, bin_path, run_cmd, fail):
    print("== stage 11e: the run view shows the plan gate, approves it, and opens a worker's conversation ==")
    repo = f"/tmp/anthrex-smoke-view-{os.getpid()}"
    plan = f"/tmp/anthrex-smoke-view-plan-{os.getpid()}.toml"
    proc = None
    try:
        os.makedirs(repo)
        _git(["init", "-q", "-b", "main"], repo, fail)
        _git(["config", "user.name", "Smoke Test"], repo, fail)
        _git(["config", "user.email", "smoke@example.com"], repo, fail)
        _git(["config", "commit.gpgsign", "false"], repo, fail)
        with open(os.path.join(repo, "README"), "w") as f:
            f.write("readme\n")
        _git(["add", "-A"], repo, fail)
        _git(["commit", "-q", "-m", "initial"], repo, fail)
        _write_script(repo, "worker-t1-1", WORKER)
        with open(plan, "w") as f:
            f.write(PLAN)

        start = ["run", "start", "--plan", plan, "--dir", repo]
        if sys.platform != "darwin":
            # Checks can be confined only on macOS; elsewhere the run refuses to start
            # without this explicit opt-in, as in stage 11c.
            start.append("--unconfined-checks")
        started = run_cmd(start, timeout=RUN_CMD_TIMEOUT)
        run_id = started.stdout.strip()
        if not run_id or "\n" in run_id:
            fail(f"`anthrex run start` printed no single run id:\n{started.stdout}")
        h4 = run_id[-4:]

        proc = pty_proc([bin_path])
        proc.wait_for("agents", label="run-view attach banner")
        proc.send(b"\x02T")
        proc.wait_for(" tree overview ", label="the project overview")
        proc.send(b"/")
        proc.wait_for(" FILTER ", label="overview filter mode")
        proc.send(h4.encode())
        proc.send(b"\r")
        proc.wait_for(" OVERVIEW ", label="overview navigation after filtering")
        proc.send(b"l")
        proc.wait_for(f"run {h4}  0/1", label="the run's node in the project overview")
        proc.send(b"l")
        proc.wait_for(f" run · View a · {h4} ", label="the run view's title")
        proc.wait_for("t1 add a S", label="t1's node on the run canvas")
        proc.wait_for("a approve", label="the plan gate's hint")
        print(f"ok: run {run_id} opened in the run view at its plan gate")

        proc.send(b"a")
        proc.wait_for(f"Approve run {run_id}?", label="the approve confirm")
        proc.send(b"y")
        proc.wait_for("worker #1 claude", timeout=RUN_WAIT, label="t1's worker round")
        print("ok: the plan gate approved the run from the run view and t1's worker started")

        # To t1, then to its worker round; Enter opens the headless worker's conversation.
        name = f"{h4}/t1.w1"
        if name in proc.screen_text():
            fail(f"{name!r} was on screen before the conversation opened:\n{proc.screen_text()}")
        proc.send(b"l")
        proc.send(b"l")
        _open_conversation(proc, name, fail)
        print(f"ok: Enter on the worker round opened {name}'s conversation")

        proc.send(b"q")
        proc.wait_for(
            f" run · View a · {h4} ", label="the run view after closing the conversation"
        )
        proc.send(b"\x1b")
        proc.wait_for(" tree overview ", label="the project overview after leaving the run view")
        proc.send(b"\x1b")
        # The client must have read this Esc on its own before the prefix follows it: an
        # Esc and a C-b in one read decode as Alt+C-b, and the detach is lost.
        _wait_gone(proc, " tree overview ", "the project overview after its Esc", fail)
        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=5.0)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"stage-11e detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None

        run_cmd(["run", "cancel", run_id], timeout=RUN_CMD_TIMEOUT)
        # `discard` applies only to a complete run (or a cancelled halted one whose
        # tasks are all finished), and a cancelled run completes once its killed
        # worker's session has ended.
        deadline = time.monotonic() + RUN_WAIT
        while True:
            run = _run_state(run_cmd, run_id, fail)
            if run["state"] in ("complete", "halted"):
                break
            if time.monotonic() >= deadline:
                fail(f"cancelled run {run_id} did not complete within {RUN_WAIT}s; last state {run['state']}:\n{json.dumps(run, indent=2)}")
            time.sleep(POLL)
        run_cmd(["run", "discard", run_id, "--confirm", run_id], timeout=ACCEPT_CMD_TIMEOUT)
        print(f"ok: run {run_id} was cancelled and discarded; the client detached cleanly")
    finally:
        if proc is not None:
            proc.close()
            # Reap the client this stage spawned (its exact pid only); closing its pty
            # hangs it up.
            deadline = time.monotonic() + 5.0
            while time.monotonic() < deadline:
                try:
                    pid, _ = os.waitpid(proc.pid, os.WNOHANG)
                except ChildProcessError:
                    break
                if pid == proc.pid:
                    break
                time.sleep(0.1)
        shutil.rmtree(repo, ignore_errors=True)
        try:
            os.remove(plan)
        except FileNotFoundError:
            pass
