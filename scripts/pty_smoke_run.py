"""The run-engine stage for the PTY smoke test (milestone 8a, task 25).

`scripts/pty-smoke.py` imports `run_engine_stage` and calls it with its own `run_cmd`
and `fail`, so every `anthrex` here runs with that script's isolated `ENV`:
`ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under `/tmp`, `fake-agent` as both runtimes,
`ANTHREX_GIT=off`. Nothing here starts or stops a daemon of its own. Like
`scripts/pty_tree_smoke.py`, this module has no `__main__` of its own.
"""

import json
import os
import shutil
import subprocess
import sys
import time

# `RUN_WAIT` in `crates/cli/tests/support/run_harness.rs` (300 s), the bound on one
# task path: at most 40 sequential engine git calls at 5 s (200 s), at most 4 check or
# proof runs at 10 s (40 s) and at most 20 s of scripted waits, together 260 s, rounded
# up. This stage runs one task path. Neither side can import the other's constant (a
# language boundary), so the coupling is this comment: change the Rust derivation,
# change this value.
RUN_WAIT = 300.0

# Every `anthrex run` command but accept: `run start`'s legal worst case is
# `ensure_daemon` (`SPAWN_HANDOFF_GRACE` 0.25 s + `ENSURE_DAEMON_SOCKET_WAIT` 3 s,
# `crates/tui/src/spawn.rs`) + `HANDSHAKE_TIMEOUT` (5 s, `crates/proto/src/lib.rs`) +
# `RUN_REQUEST_TIMEOUT` (180 s, `crates/cli/src/run_cmd.rs`) = 188.25 s, above
# `run_cmd`'s default of 28 s. 240 s clears it by about 27%.
RUN_CMD_TIMEOUT = 240.0

# `run accept` waits `FINISH_REQUEST_TIMEOUT` (`ACCEPT_MERGE_TIMEOUT` 600 s + 60 s,
# `crates/cli/src/run_cmd.rs`) for its reply, so its legal worst case is 3.25 + 5 + 660
# = 668.25 s, above `RUN_CMD_TIMEOUT`. 900 s clears it by about 35%.
ACCEPT_CMD_TIMEOUT = 900.0

# How often the stage polls `anthrex run status --json`.
POLL = 0.5

GIT_ENV_DROP = ("GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE", "GIT_PREFIX")


def _git(args, cwd, fail):
    env = {k: v for k, v in os.environ.items() if k not in GIT_ENV_DROP}
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    env["GIT_CONFIG_GLOBAL"] = "/dev/null"
    env["GIT_TERMINAL_PROMPT"] = "0"
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=15,
        )
    except subprocess.TimeoutExpired as error:
        fail(f"git {args!r} timed out after 15s: {error.stderr!r}")
    if result.returncode != 0:
        fail(f"git {args!r} failed ({result.returncode}): {result.stderr}")
    return result.stdout.strip()


def _write_script(repo, name, steps):
    directory = os.path.join(repo, ".git", "fake-agent")
    os.makedirs(directory, exist_ok=True)
    with open(os.path.join(directory, f"{name}.jsonl"), "w") as f:
        for step in steps:
            f.write(json.dumps(step) + "\n")


# The worker and reviewer scripts of `e2e_green_s_task_runs_to_merged`.
WORKER = [
    {"git_commit": {"file": "a.txt", "content": "a\n", "message": "add a.txt"}},
    {"usage": {"input": 11, "output": 23, "cache_read": 37, "cache_write": 41}},
    {"mcp_call": {"tool": "task_done", "args": {"summary": "added a"}}},
]
REVIEWER = [
    {
        "mcp_call": {
            "tool": "submit_review",
            "args": {"verdict": "approve", "summary": "ok", "findings": []},
        }
    },
]
PLAN = """goal = "Add a"

[profile]
check = "true"

[[task]]
id = "t1"
title = "Task t1"
size = "S"
test_mode = "check"
test_mode_reason = "smoke"
owns = ["a.txt"]
brief = "Do t1"
acceptance = ["t1 is done"]
"""


def _run_state(run_cmd, run_id, fail):
    result = run_cmd(["run", "status", run_id, "--json"], timeout=RUN_CMD_TIMEOUT)
    try:
        runs = json.loads(result.stdout)["runs"]
    except (ValueError, KeyError) as error:
        fail(f"`anthrex run status {run_id} --json` printed no snapshot ({error}):\n{result.stdout}")
    if len(runs) != 1:
        fail(f"`anthrex run status {run_id} --json` listed {len(runs)} runs:\n{result.stdout}")
    return runs[0]


def run_engine_stage(run_cmd, fail):
    print("== stage 11c: a one-task run is approved, merged and accepted ==")
    repo = f"/tmp/anthrex-smoke-run-{os.getpid()}"
    plan = f"/tmp/anthrex-smoke-plan-{os.getpid()}.toml"
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
        _write_script(repo, "reviewer-t1-1", REVIEWER)
        with open(plan, "w") as f:
            f.write(PLAN)

        start = ["run", "start", "--plan", plan, "--dir", repo]
        if sys.platform != "darwin":
            # Checks can be confined only on macOS; elsewhere the run refuses to start
            # without this explicit opt-in (M8a F1c round 2).
            start.append("--unconfined-checks")
        started = run_cmd(start, timeout=RUN_CMD_TIMEOUT)
        run_id = started.stdout.strip()
        if not run_id or "\n" in run_id:
            fail(f"`anthrex run start` printed no single run id:\n{started.stdout}")
        run_cmd(["run", "approve", run_id], timeout=RUN_CMD_TIMEOUT)

        deadline = time.monotonic() + RUN_WAIT
        state = None
        while True:
            run = _run_state(run_cmd, run_id, fail)
            state = run["state"]
            if state == "complete":
                break
            if time.monotonic() >= deadline:
                fail(f"run {run_id} did not complete within {RUN_WAIT}s; last state {state}:\n{json.dumps(run, indent=2)}")
            time.sleep(POLL)
        tasks = {t["id"]: t["state"] for t in run["tasks"]}
        if tasks != {"t1": "merged"}:
            fail(f"run {run_id} completed with {tasks}")

        run_cmd(["run", "accept", run_id, "--yes"], timeout=ACCEPT_CMD_TIMEOUT)
        subject = _git(["log", "-1", "--format=%s"], repo, fail)
        if not subject.startswith("anthrex: accept run"):
            fail(f"main's last commit is not the accept merge: {subject!r}")
        if not os.path.exists(os.path.join(repo, "a.txt")):
            fail("a.txt is not in the repository after the accept")
        print(f"ok: run {run_id} was approved, merged t1 and was accepted onto main")
    finally:
        shutil.rmtree(repo, ignore_errors=True)
        try:
            os.remove(plan)
        except FileNotFoundError:
            pass
