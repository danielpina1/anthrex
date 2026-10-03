"""The rounds-and-next-goals stage for the PTY smoke test (milestone 9.3, task M9.3.12):
stage 11j.

`scripts/pty-smoke.py` imports `keep_going_stage` and calls it with its own `PtyProc`,
`BIN`, `run_cmd`, `fail` and `ENV`, right after stage 11i (`pr_stage`) and before
`tui_stage`. Like stage 11i (`scripts/pty_smoke_pr.py`), it runs a daemon of its own:
its own socket, data directory and `ANTHREX_CONFIG` under one fresh `/tmp` directory, a
nonexistent `ANTHREX_GH_BIN`, and its own decider directory, so `fake-agent` is both
runtimes and the decider (from the script's `ENV`) and no real agent or `gh` can start.
The stage always ends with `anthrex daemon stop` under the same variables, also when it
fails, so the shared daemon `tui_stage` uses is never touched. Like the other stage
modules, this one has no `__main__` of its own (`ANTHREX_SMOKE_ONLY=11j python3
scripts/pty-smoke.py` runs it alone).

In the TUI: `C-b g` on the project, a two-line goal (Enter between the lines, KG §1)
started with Ctrl-S, so Ctrl-S reaches the client through a real PTY (KG §11); the plan
approved at the gate and the run complete; `.` on the run, `iterate`, the request and
Ctrl-S; round 2's plan approved, the run view's title reading ` · round 2`, and the run
complete again; `anthrex run accept`; then `C-b g` on the project once more, its
orchestrator row continuing the idle orchestrator, and Ctrl-S: the next goal's run has
the first run's orchestrator window (decision 23), renamed for the new run.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

from pty_smoke_adapt import GOAL_CMD_TIMEOUT, STORED_PROFILE
from pty_smoke_pr import DAEMON_START_WAIT, _reap, _stop_daemon
from pty_smoke_run import (
    ACCEPT_CMD_TIMEOUT,
    POLL,
    REVIEWER,
    RUN_CMD_TIMEOUT,
    RUN_WAIT,
    _git,
    _write_script,
)

# The bounds, restated across the language boundary from `crates/cli/tests/support`
# (`docs/timing-budgets.md`, "Recorded, from M9.3.12"): `REQUEST_WAIT` (60 s, one
# request), `GIT_TIMEOUT_SECS` (5 s, set below as the harness sets it) and M9.3.11's
# `CONTINUE_GIT_CALLS` (24, a continued start's git calls).
REQUEST_WAIT, GIT_TIMEOUT_SECS, CONTINUE_GIT_CALLS = 60.0, 5.0, 24
# `run start --goal`'s legal worst case (stage 11d's `GOAL_CMD_TIMEOUT`,
# `scripts/pty_smoke_adapt.py`, imported so the two cannot drift): the first goal's start
# through the form, and the run view opening on it.
GOAL_WAIT = GOAL_CMD_TIMEOUT
# A continued start (`CONTINUE_WAIT` of `support/run_rounds.rs`): every git call at
# `GIT_TIMEOUT_SECS`, then one engine step and the request's round trip: 180 s.
CONTINUE_WAIT = CONTINUE_GIT_CALLS * GIT_TIMEOUT_SECS + REQUEST_WAIT
# A round's completion with its summary (`SUMMARY_WAIT`): one task path, then the
# scripted orchestrator's poll that sees it and its `summary` edit: 360 s.
SUMMARY_WAIT = RUN_WAIT + REQUEST_WAIT
# Every screen this stage waits for (`PtyProc.wait_for`'s default, as 11f and 11i).
SCREEN_WAIT = 10.0
# The scripted orchestrator's poll for round 2, from its summary: the stage's snapshot
# that sees the summary (one request), its steps in the TUI (the action menu, at most
# `MENU_STEPS` moves of its selection, the iterate dialog and its text, each one screen
# wait) and the iterate (one request): 2 * 60 + 12 * 10 = 240 s.
MENU_STEPS = 8
ITERATE_WAIT = 2 * REQUEST_WAIT + (MENU_STEPS + 4) * SCREEN_WAIT
# The scripted orchestrator's poll for the accept: `run accept`'s own bound
# (`ACCEPT_CMD_TIMEOUT`, 900 s, `scripts/pty_smoke_run.py`), after the stage's detach
# (5 s) and its poll that sees round 2's summary (one request).
ACCEPT_WAIT = ACCEPT_CMD_TIMEOUT + 5 + REQUEST_WAIT

GOAL_LINES = ("add a", "then add b")
GOAL = "\n".join(GOAL_LINES)
# The run view's title cleans the goal to one line (`safe_text::one_line`).
GOAL_TITLE = " ".join(GOAL_LINES)
REQUEST = "also add b"
NEXT_GOAL = "next goal"
SUMMARY_1 = "a.txt was added."
SUMMARY_2 = "b.txt was added too."
# The run view's status-bar hint while a plan awaits approval (stage 11f's).
GATE_HINT = "a approve  x reject"
# The shell window that makes the repository a project the TUI lists.
SHELL = "kg-smoke"
# The scripted orchestrator; a chain's later runs keep its session (decision 23).
ORCH = "orchestrator-run-1"

CONFIG = """[orchestrator]
git_timeout_secs = {git}
wake_quiet_secs = 1
planners.timeout_secs = 120
{unconfined}
[orchestrator.profile]
check_timeout_secs = 10
"""

# `triage_plan()` of `crates/cli/tests/support/run_orch.rs`: the goal takes the plan
# path. A continued run is not triaged (D14).
TRIAGE = {"answer": {"kinds": ["code"], "scale": "plan", "reason": "several modules", "task": None}}


def _task(task_id, file, stage):
    return {
        "op": "add_task",
        "task": {
            "id": task_id,
            "title": f"Task {task_id}",
            "brief": f"Add {file}.",
            "acceptance": [f"{file} exists"],
            "owns": [file],
            "size": "S",
            "stage": stage,
            "test_mode": "check",
            "test_mode_reason": "a text file",
        },
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


def _edit(edits, **more):
    return {"mcp_call": {"tool": "edit_plan", "args": {"edits": edits, **more}}}


# The orchestrator, M9.3.11's `two_rounds` and what follows it in
# `e2e_next_goal_continues_the_same_orchestrator_session`: round 1 plans t1, waits for
# the approval and completion, writes its summary; it polls until the user's iterate made
# the run round 2 (a read clears the notes, so the next paste is the round wake), reads
# the round wake, plans t2 in stage 2, waits, writes round 2's summary, polls until the
# user accepted the run, and reads the next-goal wake. It then waits for a message that
# never comes, so its window stays live until the stage's daemon stops.
ORCHESTRATOR = [
    {"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}},
    _edit([_task("t1", "a.txt", 1)], submit=True),
    _until("/gate/state", "approved", RUN_WAIT),
    _until("/run/complete", True, RUN_WAIT),
    _edit([], summary=SUMMARY_1),
    _until("/run/round", 2, ITERATE_WAIT),
    {"read_message": {"expect": "the user asks for round 2 of run "}},
    _edit([_task("t2", "b.txt", 2)], submit=True),
    _until("/run/complete", True, SUMMARY_WAIT + RUN_WAIT),
    _edit([], summary=SUMMARY_2),
    _until("/run/state", "accepted", ACCEPT_WAIT),
    {"read_message": {"expect": "a new goal, run "}},
    {"mcp_call": {"tool": "run_status", "args": {"wait_secs": 0}}},
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


def _deadline_loop(what, within, probe, fail, proc=None):
    """Calls `probe` until it returns something truthy, for at most `within` seconds,
    draining the client's output (`proc`) between calls; that value."""
    deadline = time.monotonic() + within
    while True:
        value = probe()
        if value:
            return value
        if time.monotonic() >= deadline:
            screen = f"\n--- rendered screen ---\n{proc.screen_text()}" if proc else ""
            fail(f"stage 11j: {what} did not happen within {within:.0f}s{screen}")
        if proc is not None:
            proc.read_available(timeout=POLL)
        else:
            time.sleep(POLL)


def _mcp_calls(path):
    """The calls `FAKE_AGENT_MCP_LOG` at `path` holds so far, one JSON object a line (a
    line still being written is skipped)."""
    calls = []
    try:
        with open(path) as f:
            for line in f:
                try:
                    calls.append(json.loads(line))
                except ValueError:
                    pass
    except FileNotFoundError:
        pass
    return calls


def _menu_selection(screen):
    """The label of the action menu's selected item (`ui/action_menu.rs`: a row `│ ▌
    <label>` inside the menu's frame), or `None` while no menu row is selected. Only the
    rows inside the menu's frame are read (task 12 review m4): its top border is the
    screen's first `┌ ` (a dialog's square corner, then its padded title; the panes'
    corners are rounded, and the graph's connectors run `┌─`), and its rows are the ones
    whose cell under that corner is `│`, down to its bottom `└`."""
    lines = screen.splitlines()
    for top, line in enumerate(lines):
        col = line.find("┌ ")
        if col >= 0:
            break
    else:
        return None
    for line in lines[top + 1 :]:
        if line[col : col + 1] != "│":
            break
        if line[col : col + 4] == "│ ▌ ":
            return line[col + 4 :].split("│")[0].strip()
    return None


def keep_going_stage(pty_proc, bin_path, run_cmd, fail, base_env):
    print("== stage 11j: rounds and next goals ==")
    root = tempfile.mkdtemp(prefix="anthrex-smoke-kg-", dir="/tmp")
    socket = os.path.join(root, "d.sock")
    repo = os.path.join(root, "repo")
    config = os.path.join(root, "config.toml")
    deciders = os.path.join(root, "deciders")
    mcp_log = os.path.join(root, "mcp.jsonl")
    env = dict(base_env)
    env.update(
        {
            "ANTHREX_SOCKET": socket,
            "ANTHREX_DATA_DIR": os.path.join(root, "data"),
            "ANTHREX_CONFIG": config,
            # Never a real `gh`; the runtimes and the decider stay `fake-agent` (`ENV`).
            "ANTHREX_GH_BIN": "/nonexistent/anthrex-smoke/gh",
            "FAKE_AGENT_DECIDER_DIR": deciders,
            # A session without a script of its own finds none, and its hook payloads
            # name a transcript of this stage (stage 11i's review C, M1).
            "FAKE_AGENT_SCRIPT": os.path.join(root, "no-such-script.json"),
            "FAKE_AGENT_TRANSCRIPT": os.path.join(root, "transcript.jsonl"),
            # Every MCP call a scripted session makes, one JSON line each (M9.12).
            "FAKE_AGENT_MCP_LOG": mcp_log,
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
        }
    )
    proc = None
    daemon = None

    def cmd(args, timeout=RUN_CMD_TIMEOUT, expect_ok=True):
        return run_cmd(args, expect_ok=expect_ok, timeout=timeout, env=env)

    def json_of(args):
        out = cmd(args).stdout
        try:
            return json.loads(out)
        except ValueError:
            fail(f"`anthrex {' '.join(args)}` printed no JSON:\n{out}")

    def run_of(run_id):
        runs = json_of(["run", "status", run_id, "--json"]).get("runs") or []
        if len(runs) != 1:
            fail(f"`anthrex run status {run_id} --json` listed {len(runs)} runs")
        return runs[0]

    def runs_of_goal(goal):
        return [r for r in json_of(["run", "status", "--json"])["runs"] if r["goal"] == goal]

    def wait_run(run_id, what, pred, within):
        return _deadline_loop(
            f"run {run_id[-4:]}: {what}",
            within,
            lambda: (lambda r: r if pred(r) else None)(run_of(run_id)),
            fail,
            proc,
        )

    def select_project():
        """`C-b t`, the shell's filter, and `k` from its row up to its project's: the
        project node `C-b g` starts a goal in (stage 11f's way)."""
        proc.send(b"\x02t")
        proc.wait_for(" TREE ", label="tree navigation mode")
        proc.send(f"/{SHELL}".encode())
        proc.wait_for(" FILTER ", label="tree filter mode")
        proc.send(b"\r")
        proc.wait_for(" TREE ", label="tree navigation after the filter")
        proc.send(b"k")
        proc.send(b"\x02g")
        proc.wait_for(f" start a goal in {project} ", label="the goal dialog on the repository")

    def approve(run_id, n):
        """Round `n`'s plan at the gate, approved in the run view (`a`, then `y`)."""
        gate = lambda r: r["state"] == "awaiting_approval" and r["round"] == n  # noqa: E731
        wait_run(run_id, f"round {n}'s plan at the gate", gate, RUN_WAIT)
        proc.wait_for(GATE_HINT, timeout=SCREEN_WAIT, label=f"round {n}'s gate in the run view")
        proc.send(b"a")
        proc.wait_for(f"Approve run {run_id}?", label=f"round {n}'s approve confirm")
        proc.send(b"y")
        approved = lambda r: r["state"] != "awaiting_approval"  # noqa: E731
        wait_run(run_id, f"round {n} approved", approved, REQUEST_WAIT)

    def complete_with(run_id, summary):
        def done(r):
            orch = r.get("orchestrator") or {}
            return r["state"] == "complete" and orch.get("summary") == summary

        return wait_run(run_id, f"complete with the summary {summary!r}", done, SUMMARY_WAIT)

    def detach():
        nonlocal proc
        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=5.0)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"stage-11j detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None

    try:
        os.makedirs(repo)
        os.makedirs(deciders)
        unconfined = "" if sys.platform == "darwin" else "unconfined_checks = true\n"
        _write(config, CONFIG.format(git=int(GIT_TIMEOUT_SECS), unconfined=unconfined))
        _git(["init", "-q", "-b", "main"], repo, fail)
        _git(["config", "user.name", "Smoke Test"], repo, fail)
        _git(["config", "user.email", "smoke@example.com"], repo, fail)
        _git(["config", "commit.gpgsign", "false"], repo, fail)
        _write(os.path.join(repo, "README"), "readme\n")
        _write(os.path.join(repo, "check.sh"), "echo check ok\n")
        _write(os.path.join(repo, "tests", "t_ok.sh"), "echo PASS t_ok\n")
        _git(["add", "-A"], repo, fail)
        _git(["commit", "-q", "-m", "initial"], repo, fail)
        project = os.path.realpath(repo)

        _write(os.path.join(deciders, "triage-1.json"), json.dumps(TRIAGE))
        _write_script(repo, ORCH, ORCHESTRATOR)
        for task_id, file in (("t1", "a.txt"), ("t2", "b.txt")):
            _write_script(repo, f"worker-{task_id}-1", _worker(file))
            _write_script(repo, f"reviewer-{task_id}-1", REVIEWER)

        # The daemon is this stage's own child, in its own session, as stage 11i's: the
        # `finally` knows its pid and stops it through its socket.
        daemon = subprocess.Popen(
            [bin_path, "daemon", "start", "--foreground"],
            cwd=root,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        _deadline_loop(
            "the stage's daemon binding its socket",
            DAEMON_START_WAIT,
            lambda: os.path.exists(socket) or daemon.poll() is not None,
            fail,
        )
        if daemon.poll() is not None:
            fail(f"the stage's daemon exited at its start ({daemon.returncode})")

        # The profile a goal needs (M8b decision 22 step 2), stored as stage 11f does.
        status = json_of(["profile", "status", "--json", "--dir", repo])
        meta = {
            "confirmed_at": int(time.time()),
            "report": None,
            "verification": None,
            "fingerprint": {},
            "edited_keys": [],
            "project": status["project"],
        }
        _write(os.path.join(status["repo_dir"], "profile.meta.json"), json.dumps(meta))
        _write(os.path.join(status["repo_dir"], "profile.toml"), STORED_PROFILE)
        cmd(["new", "--runtime", "shell", "--name", SHELL, "--dir", repo])

        # 1. `C-b g`, a two-line goal, Ctrl-S: the run view opens on the new run.
        proc = pty_proc([bin_path], env=env)
        proc.wait_for("agents", label="stage-11j attach banner")
        proc.wait_for(SHELL, label="the repository's shell in the sidebar")
        select_project()
        proc.send(GOAL_LINES[0].encode())
        proc.send(b"\r")
        proc.send(GOAL_LINES[1].encode())
        proc.wait_for(GOAL_LINES[1], label="the goal's second line")
        proc.send(b"\x13")
        # The run's goal is the two lines, the newline kept: Enter in the editor is a
        # newline, and only Ctrl-S starts.
        first = _deadline_loop(
            "the goal's run listed",
            GOAL_WAIT,
            lambda: (lambda rs: rs[0]["run_id"] if rs else None)(runs_of_goal(GOAL)),
            fail,
            proc,
        )
        h4 = first[-4:]
        proc.wait_for(
            f" run · {GOAL_TITLE} · {h4} ",
            timeout=GOAL_WAIT,
            label="the run view on the new run",
        )
        run = run_of(first)
        if run.get("path") != "plan" or not run.get("orchestrator"):
            fail(f"run {first} is not a planned run: path {run.get('path')!r}")
        print(f"ok: Ctrl-S in the goal dialog started run {first} with a two-line goal")

        # 2. Round 1 approved at the gate; the run completes with its summary.
        approve(first, 1)
        run = complete_with(first, SUMMARY_1)
        window = run["orchestrator"]["window_id"]
        print(f"ok: round 1 approved in the run view; run {h4} complete")

        # 3. `.` on the run, `iterate`, the request, Ctrl-S; round 2 approved; complete.
        proc.send(b".")
        proc.wait_for("j/k move", label="the action menu")
        proc.wait_for("iterate", label="the action menu listing iterate")
        # One `j` at a time, each waited for until the menu draws its selection on another
        # item, so no key overshoots `iterate` (the run view's clock redraws the screen
        # too, so a changed screen alone is not a moved selection).
        for _ in range(MENU_STEPS):
            label = _menu_selection(proc.screen_text())
            if label == "iterate":
                break
            proc.send(b"j")
            _deadline_loop(
                f"the action menu's selection moving off {label!r}",
                SCREEN_WAIT,
                lambda: _menu_selection(proc.screen_text()) not in (None, label),
                fail,
                proc,
            )
        if _menu_selection(proc.screen_text()) != "iterate":
            fail(f"iterate was not selected in the action menu\n{proc.screen_text()}")
        proc.send(b"\r")
        proc.wait_for(f"iterate run {h4} · round 2", label="the iterate dialog")
        proc.send(REQUEST.encode())
        proc.wait_for(REQUEST, label="the typed request")
        proc.send(b"\x13")
        wait_run(first, "round 2 started", lambda r: r["round"] == 2, 2 * REQUEST_WAIT)
        proc.wait_for(
            f" run · {GOAL_TITLE} · {h4} · round 2 ",
            timeout=SCREEN_WAIT,
            label="the run view's title with ` · round 2`",
        )
        print("ok: iterate from the action menu started round 2; the title reads ` · round 2`")
        approve(first, 2)
        run = complete_with(first, SUMMARY_2)
        tasks = {t["id"]: t["state"] for t in run["tasks"]}
        if tasks != {"t1": "merged", "t2": "merged"}:
            fail(f"run {first} completed round 2 with {tasks}")
        print(f"ok: round 2 approved; run {h4} complete with t1 and t2 merged")
        detach()

        # 4. `anthrex run accept`: both rounds' files on main.
        cmd(["run", "accept", first, "--yes", "--dir", repo], timeout=ACCEPT_CMD_TIMEOUT)
        for file in ("a.txt", "b.txt"):
            if _git(["show", f"main:{file}"], repo, fail) != file:
                fail(f"{file} is not on main after the accept")
        wait_run(first, "accepted", lambda r: r["state"] == "accepted", REQUEST_WAIT)
        print(f"ok: run {h4} was accepted; both rounds' files are on main")

        # The orchestrator's script polls until it sees the accept. Once the next goal
        # starts, its calls resolve to the new run (D16), which is never `accepted`, so
        # the next goal waits for that poll to end, as the e2e test's `wait_saw` does.
        def saw_accept():
            for call in _mcp_calls(mcp_log):
                if call.get("script") != ORCH or call.get("tool") != "run_status":
                    continue
                try:
                    if json.loads(call.get("result") or "")["run"]["state"] == "accepted":
                        return True
                except (ValueError, KeyError, TypeError):
                    pass
            return None

        _deadline_loop("the orchestrator seeing the accept", ACCEPT_WAIT, saw_accept, fail)

        # 5. `C-b g` on the project: the orchestrator row continues the idle orchestrator.
        proc = pty_proc([bin_path], env=env)
        proc.wait_for("agents", label="stage-11j second attach banner")
        proc.wait_for(SHELL, label="the repository's shell in the sidebar again")
        select_project()
        proc.wait_for(
            f"‹ continue o-{h4} (after {h4}) ›",
            label="the orchestrator row continuing the idle orchestrator",
        )
        proc.send(NEXT_GOAL.encode())
        proc.wait_for(NEXT_GOAL, label="the next goal's text")
        proc.send(b"\x13")
        nxt = _deadline_loop(
            "the next goal's run listed",
            CONTINUE_WAIT,
            lambda: (lambda rs: rs[0]["run_id"] if rs else None)(runs_of_goal(NEXT_GOAL)),
            fail,
            proc,
        )
        n4 = nxt[-4:]
        proc.wait_for(
            f" run · {NEXT_GOAL} · {n4} ",
            timeout=CONTINUE_WAIT,
            label="the run view on the next goal's run",
        )

        # 6. The same orchestrator window, renamed for the new run, in the same chain.
        bound = lambda r: (r.get("orchestrator") or {}).get("window_id")  # noqa: E731
        run = wait_run(nxt, "its orchestrator window", bound, CONTINUE_WAIT)
        if bound(run) != window:
            fail(f"run {n4}'s orchestrator window is {bound(run)}, not {window}")
        if run.get("chain") != f"o-{h4}" or run_of(first).get("chain") != f"o-{h4}":
            fail(f"runs {h4} and {n4} are not both in chain o-{h4}: {run.get('chain')!r}")

        def renamed():
            windows = json_of(["ls", "--json"])
            return any(w["id"] == window and w["name"] == f"{n4}/orchestrator" for w in windows)

        what = f"window {window} renamed {n4}/orchestrator"
        _deadline_loop(what, CONTINUE_WAIT, renamed, fail, proc)

        # The same session read the next-goal wake: its next `run_status` (the marker,
        # `wait_secs` 0, which no other step sends) names the new run (rule 46).
        def marker():
            for call in _mcp_calls(mcp_log):
                if call.get("script") == ORCH and call.get("args") == {"wait_secs": 0}:
                    try:
                        return json.loads(call.get("result") or "")["run"]["id"]
                    except (ValueError, KeyError, TypeError):
                        fail(f"the orchestrator's marker answered no run: {call}")
            return None

        what = "the orchestrator reading the next-goal wake"
        seen = _deadline_loop(what, CONTINUE_WAIT, marker, fail, proc)
        if seen != nxt:
            fail(f"after the next-goal wake, the orchestrator's run_status named {seen}, not {nxt}")
        print(f"ok: run {n4} continues o-{h4} in the same window ({window}) and the same session")
        detach()
    finally:
        if proc is not None:
            _reap(proc)
        _stop_daemon(run_cmd, env, socket, daemon, "11j")
        if not os.environ.get("ANTHREX_SMOKE_KEEP"):
            shutil.rmtree(root, ignore_errors=True)
