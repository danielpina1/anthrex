"""The fast-path stage for the PTY smoke test (milestone 8b, task 19): stage 11d.

`scripts/pty-smoke.py` imports `adapt_stage` and calls it with its own `run_cmd` and
`fail`, right after stage 11c, so every `anthrex` here runs with that script's
isolated `ENV`: `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under `/tmp`, `fake-agent` as
both runtimes and, from the daemon's start, as the decider (`ANTHREX_DECIDER_BIN`),
answering from `DECIDER_DIR` (`FAKE_AGENT_DECIDER_DIR`). Nothing here starts or stops
a daemon of its own. Like `scripts/pty_smoke_run.py`, whose helpers it reuses, this
module has no `__main__` of its own.
"""

import json
import os
import shutil
import sys
import time

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

# `FAKE_AGENT_DECIDER_DIR` for the whole smoke daemon. `pty-smoke.py` sets it in its
# `ENV` before the daemon starts; a directory that does not exist has no script, so a
# decider an earlier stage causes falls back (M8b decision 36). This stage creates it
# and removes it again.
DECIDER_DIR = f"/tmp/anthrex-smoke-adapt-deciders-{os.getpid()}"

# `run start --goal`'s legal worst case: `ensure_daemon` (3.25 s) + `HANDSHAKE_TIMEOUT`
# (5 s) + `GOAL_REQUEST_TIMEOUT` (1175 s, `crates/cli/src/run_cmd/adapt.rs`: the 545 s
# `RUN_START_TIMEOUT`, the largest `deciders.timeout_secs` of 600 s, and 30 s of its own
# git calls) = 1183.25 s, rounded up to 1300 s (about 10% over). Derived as
# `pty_smoke_run.py` derives `RUN_CMD_TIMEOUT`; change the Rust constant, change this value.
GOAL_CMD_TIMEOUT = 1300.0

# The stored profile of `crates/cli/tests/support/run_adapt.rs` (`STORED_PROFILE`).
STORED_PROFILE = """check = "sh check.sh"
check_timeout_secs = 10
single_test = "sh tests/{test}.sh"
test_passed = "PASS {test}"
sample_test = "t_ok"
filter_prefixes = ["sh tests/"]
generated = ["Cargo.lock"]
"""

# `triage_single(["a.txt"])` of the same file: one S `check`-mode code task.
TRIAGE = {
    "answer": {
        "kinds": ["code"],
        "scale": "single",
        "reason": "one small change",
        "task": {
            "title": "Add a",
            "brief": "Create the file the goal asks for.",
            "acceptance": ["the file exists"],
            "owns": ["a.txt"],
            "size": "S",
            "interface_change": False,
            "test_mode": "check",
            "test_mode_reason": "a text file",
            "test_to_write": None,
        },
    }
}

# The worker script of `e2e_green_s_task_on_the_fast_path` (`green_scripts`); the
# reviewer approves (`REVIEWER`).
WORKER = [
    {"git_commit": {"file": "a.txt", "content": "a\n", "message": "add a.txt"}},
    {"mcp_call": {"tool": "task_done", "args": {"summary": "added a"}}},
]


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def adapt_stage(run_cmd, fail):
    print("== stage 11d: a goal takes the fast path ==")
    repo = f"/tmp/anthrex-smoke-adapt-{os.getpid()}"
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

        status = run_cmd(["profile", "status", "--json", "--dir", repo], timeout=RUN_CMD_TIMEOUT)
        try:
            status = json.loads(status.stdout)
            repo_dir, project = status["repo_dir"], status["project"]
        except (ValueError, KeyError) as error:
            fail(f"`anthrex profile status --json` printed no repo_dir ({error}):\n{status.stdout}")
        # As `anthrex profile confirm` would store it, with an empty fingerprint (the
        # profile names no convention or manifest file, so nothing can go stale).
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

        os.makedirs(DECIDER_DIR, exist_ok=True)
        _write(os.path.join(DECIDER_DIR, "triage-1.json"), json.dumps(TRIAGE))
        _write_script(repo, "worker-t1-1", WORKER)
        _write_script(repo, "reviewer-t1-1", REVIEWER)

        start = ["run", "start", "--goal", "add a", "--dir", repo]
        if sys.platform != "darwin":
            # Checks can be confined only on macOS; elsewhere the run refuses to start
            # without this explicit opt-in (M8a F1c round 2), as in stage 11c.
            start.append("--unconfined-checks")
        started = run_cmd(start, timeout=GOAL_CMD_TIMEOUT)
        run_id = started.stdout.strip()
        if not run_id or "\n" in run_id:
            fail(f"`anthrex run start --goal` printed no single run id:\n{started.stdout}")
        if "fast path" not in started.stderr:
            fail(f"`anthrex run start --goal` did not take the fast path:\n{started.stderr}")

        deadline = time.monotonic() + RUN_WAIT
        while True:
            run = _run_state(run_cmd, run_id, fail)
            if run["state"] == "complete":
                break
            if time.monotonic() >= deadline:
                fail(f"run {run_id} did not complete within {RUN_WAIT}s; last state {run['state']}:\n{json.dumps(run, indent=2)}")
            time.sleep(POLL)
        tasks = {t["id"]: t["state"] for t in run["tasks"]}
        if tasks != {"t1": "merged"} or run.get("path") != "fast":
            fail(f"run {run_id} completed with path {run.get('path')!r} and {tasks}")

        run_cmd(["run", "accept", run_id, "--yes"], timeout=ACCEPT_CMD_TIMEOUT)
        if _git(["show", "main:a.txt"], repo, fail) != "a":
            fail("a.txt is not on main after the accept")
        print(f"ok: goal run {run_id} took the fast path, merged t1 and was accepted onto main")
    finally:
        shutil.rmtree(repo, ignore_errors=True)
        shutil.rmtree(DECIDER_DIR, ignore_errors=True)
