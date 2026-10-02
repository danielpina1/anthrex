"""The planned-goal stage for the PTY smoke test (milestone 9, task 17): stage 11f.

`scripts/pty-smoke.py` imports `orch_stage` and calls it with its own `PtyProc`, `BIN`,
`run_cmd` and `fail`, right after stage 11e, so every `anthrex` here runs with that
script's isolated `ENV`: `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under `/tmp`,
`ANTHREX_GIT=off`, and `fake-agent` as both runtimes and as the decider
(`ANTHREX_CLAUDE_BIN`, `ANTHREX_CODEX_BIN`, `ANTHREX_DECIDER_BIN`), answering from
`DECIDER_DIR` (`FAKE_AGENT_DECIDER_DIR`). No real agent can start. Nothing here starts
or stops a daemon of its own. Like `scripts/pty_smoke_run.py`, whose helpers it
reuses, this module has no `__main__` of its own.

A goal takes the plan path: a scripted orchestrator (`orchestrator-run-1`) plans two
tasks and submits them; the user approves the plan in the run view, focuses the
orchestrator from the run's root and tells it the run is done; the orchestrator writes
its summary, and the run is accepted. On macOS the goal starts through the TUI's goal
form (`C-b g`, decision 44); elsewhere, where a run needs `--unconfined-checks` and the
form has no such toggle, through `run start --goal`.
"""

import json
import os
import shutil
import sys
import time

from pty_smoke_adapt import DECIDER_DIR, GOAL_CMD_TIMEOUT, STORED_PROFILE
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

GOAL = "add two files"
SUMMARY = "Both files were added, a.txt and b.txt."
# The run view's status-bar hint while the plan awaits approval
# (`crates/tui/src/ui/statusbar.rs`, `navigate_hint`).
GATE_HINT = "a approve  x reject"
# The name of the shell window that makes the repository a project the TUI lists, so
# its project node can be selected for `C-b g`.
SHELL = "orch-smoke"

# `triage_plan()` of `crates/cli/tests/support/run_orch.rs`: the goal takes the plan
# path.
TRIAGE = {
    "answer": {"kinds": ["code"], "scale": "plan", "reason": "several modules", "task": None}
}


def _task(task_id, file):
    return {
        "id": task_id,
        "title": f"Task {task_id}",
        "brief": f"Add {file}.",
        "acceptance": [f"{file} exists"],
        "owns": [file],
        "size": "S",
        "test_mode": "check",
        "test_mode_reason": "a text file",
    }


def _until(pointer, equals, timeout):
    return {
        "mcp_until": {
            "tool": "run_status",
            "args": {"wait_secs": 5},
            "until": {"pointer": pointer, "equals": equals},
            "timeout_ms": int(timeout * 1000),
        }
    }


# The orchestrator: it reads its context, plans and submits two S tasks, waits in
# `run_status` for the user's approval and for the run to complete, then for the user's
# `done`, and writes its summary. It then waits for a message that never comes, so its
# window stays live until the stage removes it.
ORCHESTRATOR = [
    {"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}},
    {"mcp_call": {"tool": "get_context", "args": {}}},
    {
        "mcp_call": {
            "tool": "edit_plan",
            "args": {
                "edits": [
                    {"op": "add_task", "task": _task("t1", "a.txt")},
                    {"op": "add_task", "task": _task("t2", "b.txt")},
                ],
                "submit": True,
            },
        }
    },
    {"expect": {"pointer": "/awaiting_approval", "equals": True}},
    _until("/gate/state", "approved", RUN_WAIT),
    _until("/run/complete", True, RUN_WAIT * 2),
    {"read_message": {"expect": "done"}},
    {"mcp_call": {"tool": "edit_plan", "args": {"edits": [], "summary": SUMMARY}}},
    {"read_message": {}},
]


def _worker(file):
    return [
        {"git_commit": {"file": file, "content": f"{file}\n", "message": f"add {file}"}},
        {"mcp_call": {"tool": "task_done", "args": {"summary": f"added {file}"}}},
    ]


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def _store_profile(run_cmd, repo, fail):
    """Stores the profile a goal needs (M8b decision 22 step 2), as stage 11d does."""
    status = run_cmd(["profile", "status", "--json", "--dir", repo], timeout=RUN_CMD_TIMEOUT)
    try:
        status = json.loads(status.stdout)
        repo_dir, project = status["repo_dir"], status["project"]
    except (ValueError, KeyError) as error:
        fail(f"`anthrex profile status --json` printed no repo_dir ({error}):\n{status.stdout}")
    meta = {
        "confirmed_at": int(time.time()),
        "report": None,
        "verification": None,
        "fingerprint": {},
        "edited_keys": [],
        "project": project,
    }
    _write(os.path.join(repo_dir, "profile.meta.json"), json.dumps(meta))
    _write(os.path.join(repo_dir, "profile.toml"), STORED_PROFILE)


# A run state that ends a run: an earlier run of the same goal is not this one.
TERMINAL = ("accepted", "discarded", "failed")


def _goal_run(run_cmd, fail, deadline):
    """The id of the one unfinished run of this goal, once `run status` lists it,
    before `deadline`."""
    while True:
        result = run_cmd(["run", "status", "--json"], timeout=RUN_CMD_TIMEOUT)
        try:
            runs = json.loads(result.stdout)["runs"]
        except (ValueError, KeyError) as error:
            fail(f"`anthrex run status --json` printed no snapshot ({error}):\n{result.stdout}")
        ours = [r for r in runs if r["goal"] == GOAL and r["state"] not in TERMINAL]
        if len(ours) > 1:
            fail(f"more than one run of {GOAL!r}: {[r['run_id'] for r in ours]}")
        if ours:
            return ours[0]["run_id"]
        if time.monotonic() >= deadline:
            fail(f"no run of {GOAL!r} was listed within {GOAL_CMD_TIMEOUT}s")
        time.sleep(POLL)


def _start_in_the_form(proc, run_cmd, repo, fail):
    """macOS: select the repository's project node, `C-b g`, type the goal, Enter; the
    run view opens on the new run. Its id."""
    project = os.path.realpath(repo)
    proc.send(b"\x02t")
    proc.wait_for(" TREE ", label="tree navigation mode")
    proc.send(f"/{SHELL}".encode())
    proc.wait_for(" FILTER ", label="tree filter mode")
    proc.send(b"\r")
    proc.wait_for(" TREE ", label="tree navigation after the filter")
    # From the shell's row up to its project's.
    proc.send(b"k")
    proc.send(b"\x02g")
    proc.wait_for(f" start a goal in {project} ", label="the goal form on the repository")
    proc.send(GOAL.encode())
    proc.wait_for(GOAL, label="the typed goal")
    proc.send(b"\r")
    run_id = _goal_run(run_cmd, fail, time.monotonic() + GOAL_CMD_TIMEOUT)
    proc.wait_for(
        f" run · {GOAL} · {run_id[-4:]} ",
        timeout=GOAL_CMD_TIMEOUT,
        label="the run view on the new run",
    )
    return run_id


def _start_from_the_cli(proc, run_cmd, repo, fail):
    """Off macOS: `run start --goal … --unconfined-checks`, then `C-b T`, the run's
    filter, and its node, as stage 11e opens a run. Its id."""
    start = ["run", "start", "--goal", GOAL, "--dir", repo, "--unconfined-checks"]
    started = run_cmd(start, timeout=GOAL_CMD_TIMEOUT)
    run_id = started.stdout.strip()
    if not run_id or "\n" in run_id:
        fail(f"`anthrex run start --goal` printed no single run id:\n{started.stdout}")
    h4 = run_id[-4:]
    proc.send(b"\x02T")
    proc.wait_for(" tree overview ", label="the project overview")
    proc.send(b"/")
    proc.wait_for(" FILTER ", label="overview filter mode")
    proc.send(h4.encode())
    proc.send(b"\r")
    proc.wait_for(" OVERVIEW ", label="overview navigation after filtering")
    proc.send(b"l")
    # The run's own node (its canvas text, `graph/run_text.rs::run_text`), not the
    # filter's echo of `h4`.
    _wait_either(
        proc,
        ["orchestrator  planning", "orchestrator  0/2"],
        10.0,
        "the run's node in the project overview",
        fail,
    )
    proc.send(b"l")
    proc.wait_for(f" run · {GOAL} · {h4} ", label="the run view's title")
    return run_id


def _wait_either(proc, texts, timeout, label, fail):
    """Waits until one of `texts` is on the rendered screen; the first found."""
    deadline = time.monotonic() + timeout
    while True:
        screen = proc.screen_text()
        for text in texts:
            if text in screen:
                return text
        if time.monotonic() >= deadline:
            fail(f"timed out waiting for {label}\n--- rendered screen ---\n{screen}")
        proc.read_available(timeout=0.2)


def _wait_run(proc, run_cmd, run_id, pred, timeout, label, fail):
    """Waits, within `timeout`, until `pred` holds for the run's status; the run."""
    deadline = time.monotonic() + timeout
    while True:
        run = _run_state(run_cmd, run_id, fail)
        if pred(run):
            return run
        if time.monotonic() >= deadline:
            fail(
                f"timed out waiting for {label} in run {run_id}; last state {run['state']}"
                f"\n--- rendered screen ---\n{proc.screen_text()}"
            )
        proc.read_available(timeout=POLL)


def _reap(proc):
    """Closes the client this stage spawned and reaps its exact pid."""
    proc.close()
    deadline = time.monotonic() + 5.0
    while time.monotonic() < deadline:
        try:
            pid, _ = os.waitpid(proc.pid, os.WNOHANG)
        except ChildProcessError:
            break
        if pid == proc.pid:
            break
        time.sleep(0.1)


def orch_stage(pty_proc, bin_path, run_cmd, fail):
    print("== stage 11f: a goal is planned by a scripted orchestrator ==")
    repo = f"/tmp/anthrex-smoke-orch-{os.getpid()}"
    proc = None
    windows = []
    try:
        os.makedirs(repo)
        _git(["init", "-q", "-b", "main"], repo, fail)
        _git(["config", "user.name", "Smoke Test"], repo, fail)
        _git(["config", "user.email", "smoke@example.com"], repo, fail)
        _git(["config", "commit.gpgsign", "false"], repo, fail)
        _write(os.path.join(repo, "README"), "readme\n")
        _write(os.path.join(repo, "check.sh"), "echo check ok\n")
        _write(os.path.join(repo, "tests", "t_ok.sh"), "echo PASS t_ok\n")
        _git(["add", "-A"], repo, fail)
        _git(["commit", "-q", "-m", "initial"], repo, fail)
        _store_profile(run_cmd, repo, fail)

        # Stage 11d removed the directory in its `finally`.
        os.makedirs(DECIDER_DIR, exist_ok=True)
        _write(os.path.join(DECIDER_DIR, "triage-1.json"), json.dumps(TRIAGE))
        _write_script(repo, "orchestrator-run-1", ORCHESTRATOR)
        for task_id, file in (("t1", "a.txt"), ("t2", "b.txt")):
            _write_script(repo, f"worker-{task_id}-1", _worker(file))
            _write_script(repo, f"reviewer-{task_id}-1", REVIEWER)

        proc = pty_proc([bin_path])
        proc.wait_for("agents", label="stage-11f attach banner")
        if sys.platform == "darwin":
            run_cmd(["new", "--runtime", "shell", "--name", SHELL, "--dir", repo])
            windows.append(SHELL)
            proc.wait_for(SHELL, label="the repository's shell in the sidebar")
            run_id = _start_in_the_form(proc, run_cmd, repo, fail)
            how = "the goal form"
        else:
            run_id = _start_from_the_cli(proc, run_cmd, repo, fail)
            how = "run start --goal"
        h4 = run_id[-4:]
        # The scripted orchestrator submits within moments, so the header may already
        # have moved on from `planning`; the run itself must be on the plan path.
        seen = _wait_either(
            proc,
            ["orchestrator  planning", GATE_HINT],
            RUN_WAIT,
            "the planned run's header",
            fail,
        )
        run = _run_state(run_cmd, run_id, fail)
        if run.get("path") != "plan" or not run.get("orchestrator"):
            fail(f"run {run_id} is not a planned run: path {run.get('path')!r}")
        print(f"ok: {how} started planned run {run_id}; its run view showed {seen!r}")

        proc.wait_for(GATE_HINT, timeout=RUN_WAIT, label="the submitted plan's gate")
        proc.send(b"a")
        proc.wait_for(f"Approve run {run_id}?", label="the approve confirm")
        proc.send(b"y")
        approved = lambda r: r["state"] != "awaiting_approval"  # noqa: E731
        # A diagnostic: a failure names the approval, not the run's completion.
        _wait_run(proc, run_cmd, run_id, approved, RUN_CMD_TIMEOUT, "the approval", fail)
        complete = lambda r: r["state"] == "complete"  # noqa: E731
        run = _wait_run(proc, run_cmd, run_id, complete, RUN_WAIT, "complete", fail)
        tasks = {t["id"]: t["state"] for t in run["tasks"]}
        if tasks != {"t1": "merged", "t2": "merged"}:
            fail(f"run {run_id} completed with {tasks}")
        proc.wait_for("orchestrator  2/2", timeout=RUN_WAIT, label="the run's root with both tasks merged")
        print(f"ok: the plan was approved in the run view and run {run_id} completed")

        # Enter on the root focuses the orchestrator (M8c); the user tells it.
        window = run["orchestrator"]["window_id"]
        windows.append(str(window))
        proc.send(b"\r")
        proc.wait_for(f"{h4}/orchestrator · ", label="the focused orchestrator window")
        # Typed at once, as a user types: decision 39's quiet time keeps a pending
        # wake-up from being pasted into it.
        proc.send(b"done\r")
        deadline = time.monotonic() + RUN_WAIT
        while True:
            status = run_cmd(["run", "status", run_id], timeout=RUN_CMD_TIMEOUT)
            if "summary: written" in status.stdout:
                break
            if time.monotonic() >= deadline:
                fail(
                    f"run {run_id}'s summary was not written within {RUN_WAIT}s:\n{status.stdout}"
                    f"\n--- rendered screen ---\n{proc.screen_text()}"
                )
            proc.read_available(timeout=POLL)
        print("ok: the orchestrator read the user's line and wrote its summary")

        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=5.0)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"stage-11f detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None

        run_cmd(["run", "accept", run_id, "--yes"], timeout=ACCEPT_CMD_TIMEOUT)
        for file in ("a.txt", "b.txt"):
            if _git(["show", f"main:{file}"], repo, fail) != file:
                fail(f"{file} is not on main after the accept")
        print(f"ok: run {run_id} was accepted; both files are on main")
    finally:
        if proc is not None:
            _reap(proc)
        # The orchestrator's window is a plain window once its run is terminal, and the
        # shell is this stage's own.
        for name in windows:
            try:
                run_cmd(["rm", name], expect_ok=False)
            except Exception:
                pass
        shutil.rmtree(repo, ignore_errors=True)
        shutil.rmtree(DECIDER_DIR, ignore_errors=True)
