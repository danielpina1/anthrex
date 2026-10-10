"""The simple repository profile's stage for the PTY smoke test (milestone 9.10, task 12):
stage 11l.

`scripts/pty-smoke.py` imports `profile_stage` and calls it with its own `PtyProc`, `BIN`,
`run_cmd`, `fail` and `ENV`, after stage 11k (`design_stage`) and before stage 11t
(`tui_stage`), so every `anthrex` here runs with that script's isolated `ENV`:
`ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under `/tmp`, `fake-agent` as both runtimes, as
the onboarding scout and as the decider (`ANTHREX_DECIDER_BIN`, answering from
`DECIDER_DIR`). Like 11d and 11t it runs on the shared daemon, so it is not in `only`'s
list. Nothing here starts or stops a daemon of its own. No client is attached when the
stage starts, so it starts its own and detaches it.

A first goal in a repository with no stored profile waits for it (decision 39: exit 3,
nothing on stdout); anthrex sets the repository up (a scout, then the checks), the
Alerts box asks for one review, and the card's Use this stores the profile and starts
the goal, which runs to completion and is accepted onto `main`. Like
`scripts/pty_smoke_run.py`, whose helpers it reuses, this module has no `__main__`.
"""

import json
import os
import shutil
import sys
import time

from pty_smoke_adapt import DECIDER_DIR, GOAL_CMD_TIMEOUT, TRIAGE, WORKER
from pty_smoke_run import (
    ACCEPT_CMD_TIMEOUT,
    POLL,
    REVIEWER,
    RUN_CMD_TIMEOUT,
    RUN_WAIT,
    _git,
    _run_state,
    _write_script,
)
from pty_smoke_tui import DETACH_WAIT, SCREEN_WAIT

GOAL = "Profile l"

# Decision 39's stderr line for `GOAL` (`crates/cli/src/run_cmd/adapt.rs`).
QUEUED_LINE = f"queued: {GOAL} waits for the repository profile (anthrex profile); no run started yet"

# `EXIT_QUEUED` (`crates/cli/src/run_cmd/adapt.rs`).
EXIT_QUEUED = 3

# The scout's report: the set-up proposes one command, `check`.
SCOUT = [
    {
        "mcp_call": {
            "tool": "submit_scout_report",
            "args": {
                "summary": "A shell project; check.sh checks it.",
                "files": [{"path": "check.sh", "why": "the check"}],
                "profile": {"check": "sh check.sh"},
            },
        }
    }
]

# The set-up, from the goal's reply to the review alert: `PROFILE_WAIT`'s derivation
# (`crates/cli/tests/support/run_adapt.rs`: the scout's timeout + the verification's
# commands at `onboarding.verify_timeout_secs` + at most 40 engine git calls at
# `git_timeout_secs`), with this daemon's defaults (it reads no `[orchestrator]` table):
# `scouts.timeout_secs` 900 s + one command (`check`) at 1800 s + 40 git calls at 60 s
# (2400 s) = 5100 s, rounded up. A hang guard: the set-up takes a few seconds.
SETUP_WAIT = 5200.0

# The Profile screen's card once it opens: the screen asks the daemon for its status and
# the proposal (`ProfileRequest::Status`, `Show`), answered off the engine; the client
# gives up at `REPLY_TIMEOUT` (30 s, `crates/tui/src/app/replies.rs`). 30 s + 15 s of
# margin, as stage 11t's `SETTINGS_LOAD_WAIT`.
CARD_WAIT = 45.0

# The sidebar's width (`crate::ui::DEFAULT_SIDEBAR_WIDTH`), as stage 11k reads its
# Alerts box (`scripts/pty_smoke_design_fixtures.py`).
SIDEBAR = 34


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def _repo(repo, fail):
    os.makedirs(repo)
    _git(["init", "-q", "-b", "main"], repo, fail)
    _git(["config", "user.name", "Smoke Test"], repo, fail)
    _git(["config", "user.email", "smoke@example.com"], repo, fail)
    _git(["config", "commit.gpgsign", "false"], repo, fail)
    _write(os.path.join(repo, "README"), "readme\n")
    _write(os.path.join(repo, "check.sh"), "echo check ok\n")
    _git(["add", "-A"], repo, fail)
    _git(["commit", "-q", "-m", "initial"], repo, fail)


def _sidebar_alerts(proc):
    """The sidebar's Alerts box as one line, its rows joined so a wrapped row reads whole
    (stage 11k's `sidebar_alerts`)."""
    rows = proc.screen_region_text(0, SIDEBAR).splitlines()
    tops = [i for i, row in enumerate(rows) if "Alerts" in row and row.startswith("╭")]
    if not tops:
        return ""
    inside = []
    for row in rows[tops[-1] + 1 :]:
        if row.startswith("╰"):
            break
        inside.append(row.strip("│ "))
    return " ".join(" ".join(inside).split())


def _until(proc, what, within, probe, fail):
    """`probe()` until it is truthy, for at most `within` s, draining the client."""
    deadline = time.monotonic() + within
    while True:
        value = probe()
        if value:
            return value
        if time.monotonic() >= deadline:
            screen = f"\n--- rendered screen ---\n{proc.screen_text()}" if proc else ""
            fail(f"stage 11l: {what} did not happen within {within:.0f}s{screen}")
        if proc is not None:
            proc.read_available(timeout=POLL)
        else:
            time.sleep(POLL)


def _goal_runs(run_cmd, fail):
    result = run_cmd(["run", "status", "--json"], timeout=RUN_CMD_TIMEOUT)
    try:
        runs = json.loads(result.stdout)["runs"]
    except (ValueError, KeyError) as error:
        fail(f"`anthrex run status --json` printed no snapshot ({error}):\n{result.stdout}")
    return [r for r in runs if r.get("goal") == GOAL]


def profile_stage(pty_proc, bin_path, run_cmd, fail, env):
    print("== stage 11l: a first goal sets the profile up, one review, Use this ==")
    repo = f"/tmp/anthrex-smoke-profile-{os.getpid()}"
    proc = None
    try:
        _repo(repo, fail)
        _write_script(repo, "scout-onboarding-1", SCOUT)
        os.makedirs(DECIDER_DIR, exist_ok=True)
        _write(os.path.join(DECIDER_DIR, "triage-1.json"), json.dumps(TRIAGE))
        _write_script(repo, "worker-t1-1", WORKER)
        _write_script(repo, "reviewer-t1-1", REVIEWER)

        status = run_cmd(["profile", "status", "--json", "--dir", repo], timeout=RUN_CMD_TIMEOUT)
        try:
            source = json.loads(status.stdout)["source"]
        except (ValueError, KeyError) as error:
            fail(f"`anthrex profile status --json` printed no source ({error}):\n{status.stdout}")
        if source != "none":
            fail(f"stage 11l's repository already has a profile ({source!r})")

        start = ["run", "start", "--goal", GOAL, "--dir", repo]
        if sys.platform != "darwin":
            # Checks can be confined only on macOS (stage 11c).
            start.append("--unconfined-checks")
        queued = run_cmd(start, expect_ok=False, timeout=GOAL_CMD_TIMEOUT, env=env)
        if queued.returncode != EXIT_QUEUED or queued.stdout != "" or queued.stderr.rstrip("\n") != QUEUED_LINE:
            fail(
                f"`anthrex run start --goal` did not queue the goal (exit {queued.returncode}, "
                f"want {EXIT_QUEUED})\nstdout: {queued.stdout!r}\nstderr: {queued.stderr!r}"
            )
        print("ok: the goal was queued for the profile: exit 3, nothing on stdout")

        proc = pty_proc([bin_path])
        proc.wait_for("agents", timeout=SCREEN_WAIT, label="stage-11l attach banner")
        review = "review how anthrex will work here"
        _until(proc, f"the alert {review!r}", SETUP_WAIT, lambda: review in _sidebar_alerts(proc), fail)
        print("ok: the Alerts box asks for the review once the set-up is done")

        proc.send(b"\x02a")
        _until(proc, "the Alerts box focused", SCREEN_WAIT, lambda: review in _sidebar_alerts(proc), fail)
        proc.send(b"\r")
        proc.wait_for(" PROFILE ", timeout=SCREEN_WAIT, label="the Profile screen's badge")
        proc.wait_for("anthrex learned how to work in this repo", timeout=CARD_WAIT, label="the card's title")
        proc.wait_for("How anthrex checks your work", timeout=CARD_WAIT, label="the card's first section")
        print("ok: Enter on the review alert opened the card")

        # Use this: the card's proposal is stored and the queued goal starts. Its start
        # is any goal's (`GOAL_CMD_TIMEOUT`, stage 11d's derivation).
        proc.send(b"\r")
        runs = _until(proc, f"a run for {GOAL!r}", GOAL_CMD_TIMEOUT, lambda: _goal_runs(run_cmd, fail), fail)
        if len(runs) != 1:
            fail(f"`anthrex run status --json` listed {len(runs)} runs for {GOAL!r}:\n{json.dumps(runs, indent=2)}")
        run_id = runs[0]["run_id"]
        print(f"ok: Use this started run {run_id} for the queued goal")

        # The card is gone and the screen, still open, shows the stored profile.
        proc.wait_for("Ready · verified", timeout=CARD_WAIT, label="the stored profile's status line")
        proc.send(b"\x1b")
        _until(proc, "the Profile screen closing", SCREEN_WAIT, lambda: " PROFILE " not in proc.screen_text(), fail)
        # `C-b P` opens the selected node's project, and on the shared daemon another
        # stage's project may be selected, so the goal's run is selected first, as in
        # stage 11t: the tree filtered to the run's short id, then `j` onto the run.
        proc.send(b"\x02t")
        proc.wait_for(" TREE ", timeout=SCREEN_WAIT, label="the tree's navigation")
        proc.send(b"/")
        proc.wait_for(" FILTER ", timeout=SCREEN_WAIT, label="the tree's filter")
        proc.send(run_id[-4:].encode())
        proc.send(b"\r")
        proc.wait_for(" TREE ", timeout=SCREEN_WAIT, label="the tree after filtering")
        proc.wait_for(GOAL, timeout=SCREEN_WAIT, label="the run's node in the tree")
        proc.send(b"j")
        proc.send(b"\x02P")
        proc.wait_for(" PROFILE ", timeout=SCREEN_WAIT, label="the Profile screen's badge")
        proc.wait_for(f"profile · {os.path.basename(repo)}", timeout=SCREEN_WAIT, label="the Profile screen's project")
        proc.wait_for("Ready · verified", timeout=CARD_WAIT, label="the stored profile's status line after C-b P")
        print("ok: C-b P on the goal's run shows its stored profile: Ready · verified")
        proc.send(b"\x1b")
        _until(proc, "the Profile screen closing", SCREEN_WAIT, lambda: " PROFILE " not in proc.screen_text(), fail)
        # Out of the tree; the client must read this Esc on its own before the prefix
        # follows it (an Esc and a C-b in one read decode as Alt+C-b, stage 11e).
        proc.send(b"\x1b")
        _until(proc, "the tree closing", SCREEN_WAIT, lambda: " TREE " not in proc.screen_text(), fail)
        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=DETACH_WAIT)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"stage-11l detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None

        # One task path (`RUN_WAIT`, stage 11c's).
        deadline = time.monotonic() + RUN_WAIT
        while True:
            run = _run_state(run_cmd, run_id, fail)
            if run["state"] == "complete":
                break
            if time.monotonic() >= deadline:
                fail(f"run {run_id} did not complete within {RUN_WAIT}s; last state {run['state']}:\n{json.dumps(run, indent=2)}")
            time.sleep(POLL)
        run_cmd(["run", "accept", run_id, "--yes"], timeout=ACCEPT_CMD_TIMEOUT)
        if _git(["show", "main:a.txt"], repo, fail) != "a":
            fail("a.txt is not on main after the accept")
        print(f"ok: run {run_id} for the queued goal completed and was accepted onto main")
    finally:
        if proc is not None:
            proc.close()
            # Reap the client this stage spawned (its exact pid only); closing its pty
            # hangs it up.
            deadline = time.monotonic() + DETACH_WAIT
            while time.monotonic() < deadline:
                try:
                    pid, _ = os.waitpid(proc.pid, os.WNOHANG)
                except ChildProcessError:
                    break
                if pid == proc.pid:
                    break
                time.sleep(0.1)
        shutil.rmtree(repo, ignore_errors=True)
        shutil.rmtree(DECIDER_DIR, ignore_errors=True)
