"""The tuning-and-racing stage for the PTY smoke test (milestone 9.5, task M9.5.22):
stage 11g.

`scripts/pty-smoke.py` imports `tuning_stage` and calls it with its own `PtyProc`,
`BIN`, `run_cmd`, `fail` and `ENV`, right after stage 11j (`keep_going_stage`) and
before `tui_stage`. Like stages 11i and 11j (`scripts/pty_smoke_pr.py`,
`scripts/pty_smoke_keep_going.py`), it runs a daemon of its own: its own socket, data
directory and `ANTHREX_CONFIG` under its own `/tmp` root, a nonexistent `ANTHREX_GH_BIN`,
and `fake-agent` as both runtimes and the decider (from the script's `ENV`), so no real
agent or `gh` can start. The stage always ends with `anthrex daemon stop` under the same
variables, also when it fails, and removes its two paths, so the shared daemon
`tui_stage` uses is never touched. It drives no terminal (`pty_proc` is unused); like
the other stage modules, it has no `__main__` of its own (`ANTHREX_SMOKE_ONLY=11g
python3 scripts/pty-smoke.py` runs it alone).

The repository's history is the recorded `refit.jsonl` (task M9.5.8): `anthrex run
stats` refits the S budget to `55 calls 18m` and lists its proposals. Then one M task
races (`e2e_race_both_pass_crowns_one` of `crates/cli/tests/run_e2e_race.rs`): both
racers commit and finish, their reviewers meet and approve together, exactly one lane is
crowned, and `anthrex run accept` puts `a.txt` on `main`.
"""

import json
import os
import shutil
import subprocess
import sys
import time

from pty_smoke_adapt import STORED_PROFILE
from pty_smoke_pr import DAEMON_START_WAIT, _stop_daemon
from pty_smoke_run import (
    ACCEPT_CMD_TIMEOUT,
    POLL,
    RUN_CMD_TIMEOUT,
    RUN_WAIT,
    _git,
    _write_script,
)

# One task path for each lane, as the e2e test's `2 * RUN_WAIT`.
RACE_WAIT = 2 * RUN_WAIT
# `RELEASE_POLLS` of `crates/cli/tests/support/run_adapt.rs`: a reviewer waits for the
# other at most 600 polls of 0.2 s (120 s, inside `RACE_WAIT`), then approves anyway.
RELEASE_POLLS = 600

HISTORY = os.path.join(
    os.path.dirname(os.path.abspath(__file__)),
    "..",
    "crates",
    "daemon",
    "tests",
    "fixtures",
    "history",
    "refit.jsonl",
)

CONFIG = """[orchestrator]
git_timeout_secs = 5
{unconfined}
[orchestrator.profile]
check_timeout_secs = 10
"""

PLAN = """goal = "Race"

[[task]]
id = "t1"
title = "Task t1"
size = "M"
test_mode = "check"
test_mode_reason = "smoke"
owns = ["a.txt"]
brief = "Do t1"
acceptance = ["t1 is done"]
race = true
"""


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def _racer(lane):
    return [
        {"git_commit": {"file": "a.txt", "content": f"from {lane}\n", "message": "add a.txt"}},
        {"mcp_call": {"tool": "task_done", "args": {"summary": f"added a in lane {lane}"}}},
    ]


def _reviewer(release, me, other):
    """A reviewer that marks `me`, waits for `other` (at most `RELEASE_POLLS` polls),
    then approves: both lanes are in review before either passes."""
    mine, theirs = os.path.join(release, me), os.path.join(release, other)
    wait = (
        f"i=0; while [ ! -e '{theirs}' ] && [ $i -lt {RELEASE_POLLS} ]; "
        "do sleep 0.2; i=$((i+1)); done"
    )
    return [
        {"sh": {"cmd": f"mkdir -p '{release}' && : > '{mine}'"}},
        {"sh": {"cmd": wait}},
        {
            "mcp_call": {
                "tool": "submit_review",
                "args": {"verdict": "approve", "summary": "ok", "findings": []},
            }
        },
    ]


def tuning_stage(pty_proc, bin_path, run_cmd, fail, base_env):
    print("== stage 11g: history refits a budget and a race completes ==")
    del pty_proc  # this stage drives no terminal
    repo = f"/tmp/anthrex-smoke-tuning-{os.getpid()}"
    root = f"{repo}-daemon"
    socket = os.path.join(root, "d.sock")
    config = os.path.join(root, "config.toml")
    plan = os.path.join(root, "plan.toml")
    release = os.path.join(root, "release")
    env = dict(base_env)
    env.update(
        {
            "ANTHREX_SOCKET": socket,
            "ANTHREX_DATA_DIR": os.path.join(root, "data"),
            "ANTHREX_CONFIG": config,
            # Never a real `gh`; the runtimes and the decider stay `fake-agent` (`ENV`).
            "ANTHREX_GH_BIN": "/nonexistent/anthrex-smoke/gh",
            "FAKE_AGENT_DECIDER_DIR": os.path.join(root, "deciders"),
            "FAKE_AGENT_SCRIPT": os.path.join(root, "no-such-script.json"),
            "FAKE_AGENT_TRANSCRIPT": os.path.join(root, "transcript.jsonl"),
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
        }
    )
    for path in (repo, root):
        if os.path.exists(path):
            fail(f"stage 11g: {path} already exists, and is not this stage's to remove")
    daemon = None

    def cmd(args, timeout=RUN_CMD_TIMEOUT, expect_ok=True):
        return run_cmd(args, expect_ok=expect_ok, timeout=timeout, env=env)

    def json_of(args):
        out = cmd(args).stdout
        try:
            return json.loads(out)
        except ValueError:
            fail(f"`anthrex {' '.join(args)}` printed no JSON:\n{out}")

    try:
        os.makedirs(repo)
        os.makedirs(root)
        unconfined = "" if sys.platform == "darwin" else "unconfined_checks = true\n"
        _write(config, CONFIG.format(unconfined=unconfined))

        # 1. A repository with an identity, `check.sh` and one commit.
        _git(["init", "-q", "-b", "main"], repo, fail)
        _git(["config", "user.name", "Smoke Test"], repo, fail)
        _git(["config", "user.email", "smoke@example.com"], repo, fail)
        _git(["config", "commit.gpgsign", "false"], repo, fail)
        _write(os.path.join(repo, "README"), "readme\n")
        _write(os.path.join(repo, "check.sh"), "echo check ok\n")
        _write(os.path.join(repo, "tests", "t_ok.sh"), "echo PASS t_ok\n")
        _git(["add", "-A"], repo, fail)
        _git(["commit", "-q", "-m", "initial"], repo, fail)

        # The daemon is this stage's own child, in its own session, as stage 11i's.
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
                fail(f"stage 11g: its daemon did not bind {socket} within {DAEMON_START_WAIT}s")
            time.sleep(POLL)
        if daemon.poll() is not None:
            fail(f"stage 11g: its daemon exited at its start ({daemon.returncode})")

        # 2. The stored profile and the recorded history, in `ProfileStatus.repo_dir`.
        status = json_of(["profile", "status", "--json", "--dir", repo])
        repo_dir = status["repo_dir"]
        meta = {
            "confirmed_at": int(time.time()),
            "report": None,
            "verification": None,
            "fingerprint": {},
            "edited_keys": [],
            "project": status["project"],
        }
        _write(os.path.join(repo_dir, "profile.meta.json"), json.dumps(meta))
        _write(os.path.join(repo_dir, "profile.toml"), STORED_PROFILE)
        shutil.copyfile(HISTORY, os.path.join(repo_dir, "history.jsonl"))

        # 3. `run stats` refits S and lists the proposals.
        stats = cmd(["run", "stats", "--dir", repo]).stdout
        for needle in ("55 calls 18m", "tuning proposals:"):
            if needle not in stats:
                fail(f"`anthrex run stats` printed no {needle!r}:\n{stats}")
        print("ok: run stats refitted S to 55 calls 18m and listed its proposals")

        # 4. `e2e_race_both_pass_crowns_one`'s scripts and a plan with one M racing task.
        _write_script(repo, "racer-t1-a-1", _racer("a"))
        _write_script(repo, "racer-t1-b-1", _racer("b"))
        _write_script(repo, "reviewer-t1-1", _reviewer(release, "r1", "r2"))
        _write_script(repo, "reviewer-t1-2", _reviewer(release, "r2", "r1"))
        _write(plan, PLAN)

        # 5. Started with `--yes`, polled to `complete`, accepted.
        started = cmd(["run", "start", "--plan", plan, "--dir", repo, "--yes"])
        run_id = started.stdout.strip()
        if not run_id or "\n" in run_id:
            fail(f"`anthrex run start` printed no single run id:\n{started.stdout}")
        deadline = time.monotonic() + RACE_WAIT
        while True:
            runs = json_of(["run", "status", run_id, "--json"]).get("runs") or []
            if len(runs) != 1:
                fail(f"`anthrex run status {run_id} --json` listed {len(runs)} runs")
            run = runs[0]
            if run["state"] == "complete":
                break
            if time.monotonic() >= deadline:
                fail(f"run {run_id} did not complete within {RACE_WAIT}s:\n{json.dumps(run, indent=2)}")
            time.sleep(POLL)
        t1 = run["tasks"][0]
        race = t1.get("race") or {}
        winner = race.get("winner")
        states = {lane["lane"]: lane["state"] for lane in race.get("lanes", [])}
        if t1["state"] != "merged" or winner not in ("a", "b") or sorted(states.values()) != ["lost", "won"]:
            fail(f"run {run_id}'s race did not crown one lane: {t1['state']}, {race}")
        print(f"ok: run {run_id} raced t1 on two lanes and crowned racer {winner}")

        cmd(["run", "accept", run_id, "--yes", "--dir", repo], timeout=ACCEPT_CMD_TIMEOUT)
        content = _git(["show", "main:a.txt"], repo, fail)
        if content != f"from {winner}":
            fail(f"main's a.txt is {content!r}, not racer {winner}'s")
        print(f"ok: run {run_id} was accepted; racer {winner}'s a.txt is on main")
    finally:
        _stop_daemon(run_cmd, env, socket, daemon, "11g")
        if not os.environ.get("ANTHREX_SMOKE_KEEP"):
            shutil.rmtree(repo, ignore_errors=True)
            shutil.rmtree(root, ignore_errors=True)
