"""The stacked-PR delivery stage for the PTY smoke test (milestone 9.2, task M9.2.17): stage 11i.

`scripts/pty-smoke.py` imports `pr_stage` and calls it with its own `PtyProc`, `BIN`,
`run_cmd`, `fail` and `ENV`, right after stage 11h (`tiers_stage`). Unlike the stages
before it, this one runs a daemon of its own: its own socket, data directory and
`ANTHREX_CONFIG` under one `/tmp` directory, and `ANTHREX_CODE_HOST=fake` with its own
`ANTHREX_FAKE_HOST_DIR`, set only in this stage's environment, never in the script's
`ENV`. `ANTHREX_GH_BIN` points at a path that does not exist, so nothing can run a real
`gh`, and the fake GitHub's remote is a bare repository inside the fake's directory
(`run fake-github create-repo` refuses any other), reached through `insteadOf`, so no
push or fetch leaves the machine. The stage always ends with `anthrex daemon stop`
under the same variables, also when it fails. Like the other stage modules, this one
has no `__main__` of its own (`ANTHREX_SMOKE_ONLY=11i python3 scripts/pty-smoke.py`
runs it alone).

A two-stage plan in `pr` mode, with `fake-agent` workers: both stage PRs open; CI on
stage 1 is scripted red on what its task wrote, and `fake-agent`'s CI fix repairs it;
a writer's review comment becomes a fix task that is pushed and replied to; the TUI's
run view shows `#1  ci ✓` on stage 1's row; the user squash-merges stage 1 (deleting
its branch), stage 2 is synced and retargeted to `main`, the user merges it, and the
run completes. Nothing asked the fake GitHub to merge, approve or enable auto-merge
(`forbidden.jsonl` never exists).
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

from pty_smoke_run import POLL, RUN_CMD_TIMEOUT, _git, _write_script, git_env

# The bounds of `crates/cli/tests/support/run_pr.rs`, restated across the language
# boundary from the daemon's host constants (`crates/daemon/src/host/mod.rs`:
# `HOST_READ_TIMEOUT` 30 s, `HOST_WRITE_TIMEOUT` 60 s, `PUSH_TIMEOUT` 120 s,
# `LOG_TIMEOUT` 120 s), the executor's 5 s margin per op, this stage's configuration
# (`check_timeout_secs = 10`, `git_timeout_secs = 5`, `poll_max_secs = 2`,
# `review_batch_secs = 1`) and `RUN_WAIT` (300 s) and `TIER_WAIT` (670 s). Change the
# Rust derivation, change these (`docs/timing-budgets.md`, "Recorded, from M9.2.17").
READ, WRITE, PUSH, LOG, MARGIN = 30.0, 60.0, 120.0, 120.0, 5.0
RUN_WAIT, TIER_WAIT, CHECK, GIT = 300.0, 670.0, 10.0, 5.0
# One poll, then `ViewPr`'s two reads and the margin: 67 s.
VIEW_WAIT = 2 + MARGIN + 2 * READ
# The push and the seal's two reads: 185 s. A base fetch and its six reads: 305 s.
PUSH_WAIT = PUSH + 2 * READ + MARGIN
FETCH_WAIT = PUSH + 6 * READ + MARGIN
# `OpenPr`: `pr list` (one read) and `pr create` (one write), and the margin: 95 s.
OPEN_WAIT = READ + WRITE + MARGIN
# One task path; tier 3's two commands and its own git calls (at most 10 at `GIT`);
# then each op's own bound (the push, the open, the first view after one poll); 3 s
# of scheduling: 720 s (the final fix wave, review C M3).
TIER3_GIT_CALLS = 10
PR_OPEN_WAIT = RUN_WAIT + 2 * CHECK + TIER3_GIT_CALLS * GIT + 3 + PUSH_WAIT + OPEN_WAIT + VIEW_WAIT
# A red head to its fix task with deciders off: the view, the failed log, then one
# reproduction of the stage's tier-2 steps (five git calls and one check): 227 s.
CI_FIX_WAIT = VIEW_WAIT + LOG + MARGIN + 5 * GIT + CHECK
# A fix task's path, its push, and the view of the new head: 922 s.
FIX_PUSH_WAIT = TIER_WAIT + PUSH_WAIT + VIEW_WAIT
# A writer's comment to its fix task: the view, the author's permission read, the
# batch's quiet and the tick that closes it, 2 s of scheduling: 106 s.
COMMENT_WAIT = VIEW_WAIT + READ + MARGIN + 1 + 1 + 2
# One reply: the viewer and the listing (two reads), the post (one write): 125 s.
REPLY_WAIT = 2 * READ + WRITE + MARGIN
# A landing to the next stage's push and retarget: the view, the base fetch, the base
# sync bounded as one task path, the push, one write: 1 292 s.
LAND_WAIT = VIEW_WAIT + FETCH_WAIT + TIER_WAIT + PUSH_WAIT + WRITE + MARGIN

# `DAEMON_START_WAIT` (60 s) of `crates/cli/tests/support/run_daemon.rs`, the bound on a
# daemon binding its socket, and `DAEMON_STOP_CMD_TIMEOUT` (40 s) of
# `scripts/pty-smoke.py`, `daemon stop`'s 19 s worst case with margin.
DAEMON_START_WAIT, DAEMON_STOP_WAIT = 60.0, 40.0

URL = "https://github.com/fake/app.git"

CONFIG = """[orchestrator]
git_timeout_secs = 5
deciders.mode = "off"
onboarding.auto = false
{unconfined}
[orchestrator.profile]
check_timeout_secs = 10

[delivery]
poll_secs = 1
poll_max_secs = 2
review_batch_secs = 1
"""

PLAN = """goal = "Deliver a and b"

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

[[task]]
id = "t2"
title = "Task t2"
size = "S"
stage = 2
test_mode = "check"
test_mode_reason = "smoke"
owns = ["b.txt"]
brief = "Do t2"
acceptance = ["t2 is done"]
"""

# CI's check `test` is red while `a.txt` holds `bad`, which t1 writes.
CI_RULES = [
    {
        "check": "test",
        "fail_if": {"path": "a.txt", "text": "bad"},
        "conclusion": "failure",
        "failing_tests": [],
        "log": "a.txt still says bad",
    }
]

REVIEWER = [
    {"mcp_call": {"tool": "submit_review", "args": {"verdict": "approve", "summary": "ok", "findings": []}}},
]


def _commit(path, content):
    return [
        {"git_commit": {"file": path, "content": content, "message": f"change {path}"}},
        {"mcp_call": {"tool": "task_done", "args": {"summary": f"changed {path}"}}},
    ]


def _wait_for_file(path, within):
    """Worker steps that wait, with the turn open, for `path` for at least `within`
    seconds: `sh` steps of at most 300 s each (inside `fake-agent`'s 330 s
    `SH_TIMEOUT`), a `.missed` marker when the wait ran out, and an `expect` that
    fails the worker (`crates/cli/tests/run_e2e_pr_ci.rs`'s `wait_for`)."""
    missed = path + ".missed"
    steps = []
    for _ in range(int(-(-within // 300))):
        steps.append(
            {
                "sh": {
                    "cmd": f"for i in $(seq 1 1500); do [ -e '{path}' ] && break; sleep 0.2; done; "
                    f"if [ -e '{path}' ]; then echo '{{\"go\":true}}'; else touch '{missed}'; echo '{{\"go\":false}}'; fi"
                }
            }
        )
    steps.append({"expect": {"pointer": "/go", "equals": True}})
    return steps


def _wait_row(proc, start, suffix, label, fail, timeout=10.0):
    """Waits, within `timeout`, until one rendered line holds `start` and, after it,
    `suffix`: a stage node's content row."""
    deadline = time.monotonic() + timeout
    while True:
        for line in proc.screen_text().splitlines():
            at = line.find(start)
            if at >= 0 and suffix in line[at:]:
                return
        if time.monotonic() >= deadline:
            fail(f"timed out waiting for {label}\n--- rendered screen ---\n{proc.screen_text()}")
        proc.read_available(timeout=0.2)


def _poll(what, within, probe, fail):
    """Calls `probe` every `POLL` seconds until it returns something truthy, for at
    most `within` seconds; that value."""
    deadline = time.monotonic() + within
    while True:
        value = probe()
        if value:
            return value
        if time.monotonic() >= deadline:
            fail(f"stage 11i: {what} did not happen within {within:.0f}s")
        time.sleep(POLL)


def pr_stage(pty_proc, bin_path, run_cmd, fail, base_env):
    print("== stage 11i: stacked-PR delivery through FakeHost ==")
    root = tempfile.mkdtemp(prefix="anthrex-smoke-pr-", dir="/tmp")
    socket = os.path.join(root, "d.sock")
    fake = os.path.join(root, "github")
    bare = os.path.join(fake, "remote.git")
    repo = os.path.join(root, "repo")
    plan = os.path.join(root, "plan.toml")
    go = os.path.join(root, "go")
    config = os.path.join(root, "config.toml")
    env = dict(base_env)
    env.update(
        {
            "ANTHREX_SOCKET": socket,
            "ANTHREX_DATA_DIR": os.path.join(root, "data"),
            "ANTHREX_CONFIG": config,
            # Never a real `gh`: the fake, on this stage's own directory.
            "ANTHREX_GH_BIN": "/nonexistent/anthrex-smoke/gh",
            "ANTHREX_CODE_HOST": "fake",
            "ANTHREX_FAKE_HOST_DIR": fake,
            # A push or fetch that `insteadOf` did not rewrite fails, never reaching
            # the network.
            "GIT_ALLOW_PROTOCOL": "file",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
            # The stage's own fake-agent fallbacks (review C, M1): a session without a
            # script of its own finds none, whichever way the smoke was started, and
            # its hook payloads name a transcript of this stage.
            "FAKE_AGENT_SCRIPT": os.path.join(root, "no-such-script.json"),
            "FAKE_AGENT_TRANSCRIPT": os.path.join(root, "transcript.jsonl"),
        }
    )
    proc = None
    daemon = None

    def cmd(args, timeout=RUN_CMD_TIMEOUT, expect_ok=True):
        return run_cmd(args, expect_ok=expect_ok, timeout=timeout, env=env)

    def fake_github(*args):
        return cmd(["run", "fake-github", "--dir", fake, *args]).stdout

    def prs_of_run(run_id):
        out = cmd(["run", "prs", run_id, "--json"]).stdout
        try:
            return json.loads(out)
        except ValueError:
            fail(f"`anthrex run prs {run_id} --json` printed no JSON:\n{out}")

    def run_of(run_id):
        out = cmd(["run", "status", run_id, "--json"]).stdout
        try:
            return json.loads(out)["runs"][0]
        except (ValueError, KeyError, IndexError):
            fail(f"`anthrex run status {run_id} --json` printed no run:\n{out}")

    def stage_is(run_id, n, key, value):
        def probe():
            prs = prs_of_run(run_id)
            entry = prs[n - 1] if len(prs) >= n else None
            return entry if entry and entry.get(key) == value else None

        return probe

    def task_of(run, task_id):
        return next((t for t in run["tasks"] if t["id"] == task_id), None)

    try:
        os.makedirs(fake)
        os.makedirs(repo)
        with open(config, "w") as f:
            unconfined = "" if sys.platform == "darwin" else "unconfined_checks = true\n"
            f.write(CONFIG.format(unconfined=unconfined))
        # The remote: a bare repository with its own identity, inside the fake's
        # directory.
        _git(["init", "-q", "--bare", "-b", "main", bare], root, fail)
        _git(["config", "user.name", "Remote User"], bare, fail)
        _git(["config", "user.email", "remote@example.com"], bare, fail)
        _git(["init", "-q", "-b", "main"], repo, fail)
        _git(["config", "user.name", "Smoke Test"], repo, fail)
        _git(["config", "user.email", "smoke@example.com"], repo, fail)
        _git(["config", "commit.gpgsign", "false"], repo, fail)
        with open(os.path.join(repo, "README"), "w") as f:
            f.write("readme\n")
        _git(["add", "-A"], repo, fail)
        _git(["commit", "-q", "-m", "initial"], repo, fail)
        _git(["config", "remote.origin.url", URL], repo, fail)
        for key in ("insteadOf", "pushInsteadOf"):
            _git(["config", f"url.{bare}.{key}", URL], repo, fail)
        # `insteadOf` sends the push to the bare repository; `GIT_ALLOW_PROTOCOL=file`
        # makes a mis-set rewrite fail instead of reaching github.com.
        _git(["push", "-q", "origin", "main"], repo, fail, {"GIT_ALLOW_PROTOCOL": "file"})

        _write_script(repo, "worker-t1-1", _commit("a.txt", "bad\n"))
        # Stage 2's task waits until stage 1's CI fix is on its PR, so stage 2's PR opens
        # on a head that already holds the fix and its CI is never red.
        within = PR_OPEN_WAIT + CI_FIX_WAIT + FIX_PUSH_WAIT + VIEW_WAIT
        _write_script(repo, "worker-t2-1", _wait_for_file(go, within) + _commit("b.txt", "b\n"))
        # The CI fix (the stage's first fix task) and the review fix (its second).
        _write_script(repo, "worker-fix1-1", _commit("a.txt", "good\n"))
        _write_script(repo, "worker-fix2-1", _commit("a.txt", "good, and polished\n"))
        for task_id in ("t1", "t2", "fix1", "fix2"):
            _write_script(repo, f"reviewer-{task_id}-1", REVIEWER)
        with open(plan, "w") as f:
            f.write(PLAN)

        # The fake GitHub, through the control subcommand.
        fake_github("create-repo", "fake", "app", bare)
        fake_github("log-in", "github.com")
        fake_github("set-permission", "tester", "write")
        fake_github("set-ci", json.dumps(CI_RULES))

        # The daemon is this stage's own child (`daemon start --foreground`, as the Rust
        # harness starts its daemons), so the `finally` knows its pid and waits for it to
        # exit, whenever it binds its socket: a detached `daemon start` that fails or
        # times out can leave a daemon that binds only after the cleanup has looked.
        # Its own session and process group (deferred from task 17; `run_daemon.rs`
        # gives its daemons their own group), so a terminal's ^C or hang-up reaches the
        # smoke, never the stage's daemon, which the `finally` stops through its socket.
        daemon = subprocess.Popen(
            [bin_path, "daemon", "start", "--foreground"],
            cwd=root,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        _poll(
            "the stage's daemon binding its socket",
            DAEMON_START_WAIT,
            lambda: os.path.exists(socket) or daemon.poll() is not None,
            fail,
        )
        if daemon.poll() is not None:
            fail(f"the stage's daemon exited at its start ({daemon.returncode})")
        start = ["run", "start", "--plan", plan, "--dir", repo, "--delivery", "pr", "--yes"]
        if sys.platform != "darwin":
            start.append("--unconfined-checks")
        run_id = cmd(start).stdout.strip()
        if not run_id or "\n" in run_id:
            fail(f"`anthrex run start --delivery pr` printed no single run id: {run_id!r}")
        h4 = run_id[-4:]

        # PR 1 opens; its CI is red on t1's `bad`; the CI fix task repairs it.
        _poll("PR 1 opening", PR_OPEN_WAIT, stage_is(run_id, 1, "state", "open"), fail)
        fix1 = _poll(
            "the CI fix task",
            CI_FIX_WAIT,
            lambda: task_of(run_of(run_id), "fix1"),
            fail,
        )
        if fix1.get("origin") != "ci":
            fail(f"fix1 is not a CI fix task: {fix1.get('origin')!r}")
        _poll(
            "the CI fix on PR 1, green",
            FIX_PUSH_WAIT + VIEW_WAIT,
            stage_is(run_id, 1, "ci", "green"),
            fail,
        )
        print(f"ok: run {run_id}'s PR 1 went red on CI, and fix task fix1 turned it green")

        if os.path.exists(go + ".missed"):
            fail("stage 2's task stopped waiting before PR 1 was green")
        with open(go, "w") as f:
            f.write("go\n")
        _poll("PR 2 opening", PR_OPEN_WAIT, stage_is(run_id, 2, "state", "open"), fail)
        _poll("PR 2 green", VIEW_WAIT, stage_is(run_id, 2, "ci", "green"), fail)
        prs = json.loads(fake_github("prs"))
        bases = [(p["number"], p["base"]) for p in prs]
        if bases != [(1, "main"), (2, f"anthrex/{run_id}/stage-1")]:
            fail(f"the stage PRs are not stacked: {bases}")
        print("ok: both stage PRs are open and stacked, PR 2 on stage 1's branch")

        # The run view: stage 1's row carries its PR and its green CI.
        proc = pty_proc([bin_path], env=env)
        proc.wait_for("agents", label="pr-stage attach banner")
        proc.send(b"\x02T")
        proc.wait_for(" tree overview ", label="the project overview")
        proc.send(b"/")
        proc.wait_for(" FILTER ", label="overview filter mode")
        proc.send(h4.encode())
        proc.send(b"\r")
        proc.wait_for(" OVERVIEW ", label="overview navigation after filtering")
        proc.send(b"l")
        proc.wait_for(f"run {h4}", label="the run's node in the project overview")
        proc.send(b"l")
        proc.wait_for(f" run · Deliver a and b · {h4} ", label="the run view's title")
        # Stage 1's tier-3 mark is not asserted: its head moved with fix1 after the PR
        # opened, and on an open PR CI, not tier 3, is the authority (decision 28).
        _wait_row(proc, "stage 1/2  tier 3 ", "  #1  ci ✓", "stage 1's row: PR #1, CI green", fail)
        _wait_row(proc, "stage 2/2  tier 3 ", "  #2  ci ✓", "stage 2's row: PR #2, CI green", fail)
        proc.send(b"\x1b")
        proc.wait_for(" tree overview ", label="the project overview after leaving the run view")
        proc.send(b"\x1b")
        deadline = time.monotonic() + 10.0
        while " tree overview " in proc.screen_text():
            if time.monotonic() >= deadline:
                fail(f"the project overview stayed open after its Esc\n{proc.screen_text()}")
            proc.read_available(timeout=0.2)
        proc.send(b"\x02d")
        exit_status = proc.wait_exit(timeout=5.0)
        if not os.WIFEXITED(exit_status) or os.WEXITSTATUS(exit_status) != 0:
            fail(f"stage-11i detach did not exit cleanly with status 0 (raw status {exit_status})")
        proc.close()
        proc = None
        print("ok: the run view shows `#1  ci ✓` on stage 1's row and `#2  ci ✓` on stage 2's")

        # A writer comments on PR 1: a review fix task, pushed and replied to.
        first = int(fake_github("review-comment", "1", "tester", "a.txt", "1", "polish this line").strip())
        fix2 = _poll(
            "the review fix task",
            COMMENT_WAIT,
            lambda: task_of(run_of(run_id), "fix2"),
            fail,
        )
        if fix2.get("origin") != "review":
            fail(f"fix2 is not a review fix task: {fix2.get('origin')!r}")

        def replied():
            pr = next(p for p in json.loads(fake_github("prs")) if p["number"] == 1)
            return [r for r in pr["replies"] if r.get("thread") == first]

        replies = _poll("the reply on the thread", FIX_PUSH_WAIT + REPLY_WAIT, replied, fail)
        if len(replies) != 1 or "<!-- anthrex:reply " not in replies[0]["body"]:
            fail(f"the thread did not get exactly one marked reply: {replies}")
        print("ok: a writer's comment became fix task fix2, pushed and replied to once")

        # The user squash-merges stage 1, deleting its branch: stage 2 is synced, pushed
        # and retargeted to main. Then the user merges stage 2, and the run completes.
        fake_github("merge", "1", "--method", "squash", "--delete-branch")
        main_tip = _git(["rev-parse", "refs/heads/main"], bare, fail)

        def retargeted():
            pr = next(p for p in json.loads(fake_github("prs")) if p["number"] == 2)
            if pr["base"] != "main":
                return None
            held = subprocess_ok(["git", "merge-base", "--is-ancestor", main_tip, pr["head_oid"]], bare)
            return pr if held else None

        _poll("stage 2 synced and retargeted to main", LAND_WAIT, retargeted, fail)
        fake_github("merge", "2", "--method", "merge")

        def complete():
            run = run_of(run_id)
            return run if run["state"] == "complete" else None

        run = _poll("the run's completion", LAND_WAIT, complete, fail)
        tasks = {t["id"]: t["state"] for t in run["tasks"]}
        if tasks != {"t1": "merged", "t2": "merged", "fix1": "merged", "fix2": "merged"}:
            fail(f"run {run_id} completed with {tasks}")
        if os.path.exists(os.path.join(fake, "forbidden.jsonl")):
            fail(f"something asked the fake GitHub to land: {fake_github('forbidden')}")
        if json.loads(fake_github("forbidden")) != []:
            fail("the fake GitHub recorded an ask to land something")
        print(f"ok: run {run_id} completed once the user had merged both PRs; nothing asked to land")
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
        _stop_daemon(run_cmd, env, socket, daemon)
        if not os.environ.get("ANTHREX_SMOKE_KEEP"):
            shutil.rmtree(root, ignore_errors=True)


def _stop_daemon(run_cmd, env, socket, daemon, stage="11i"):
    """The stage's cleanup of its own daemon, on every way out (stage 11j's too; `stage`
    names it in what is reported): `anthrex daemon stop` under the stage's variables
    whenever its socket is there, until the child it spawned has exited (it may bind its
    socket late), for at most `DAEMON_START_WAIT` plus `DAEMON_STOP_WAIT`. A stop that
    times out (`run_cmd`'s `fail`, a `SystemExit`) is reported and the cleanup goes on.
    Nothing is signalled: a daemon that will not stop is reported by its pid."""
    deadline = time.monotonic() + DAEMON_START_WAIT + DAEMON_STOP_WAIT
    while time.monotonic() < deadline:
        child_gone = daemon is None or daemon.poll() is not None
        if child_gone and daemon is not None and os.path.exists(socket):
            # Deferred from task 17: the child exited without removing its socket; no
            # daemon of this stage answers there, so say so instead of stopping it.
            print(
                f"stage {stage}: its daemon, pid {daemon.pid}, exited ({daemon.returncode}) "
                f"and left a stale socket {socket}",
                file=sys.stderr,
            )
            return
        if os.path.exists(socket):
            try:
                run_cmd(["daemon", "stop"], expect_ok=False, timeout=DAEMON_STOP_WAIT, env=env)
            except SystemExit:
                print(f"stage {stage}: `anthrex daemon stop` on {socket} timed out", file=sys.stderr)
        elif child_gone:
            return
        time.sleep(0.2)
    if daemon is not None and daemon.poll() is None:
        print(f"stage {stage}: LEAKED its daemon, pid {daemon.pid} (socket {socket})", file=sys.stderr)
    elif os.path.exists(socket):
        print(f"stage {stage}: a daemon still answers on {socket}", file=sys.stderr)


def subprocess_ok(argv, cwd):
    """Whether a read-only `git` command succeeds in `cwd` (`merge-base --is-ancestor`),
    with `_git`'s environment (`git_env`)."""
    try:
        result = subprocess.run(
            argv, cwd=cwd, env=git_env(), stdin=subprocess.DEVNULL, capture_output=True, timeout=15
        )
    except subprocess.TimeoutExpired:
        return False
    return result.returncode == 0
