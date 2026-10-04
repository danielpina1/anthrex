"""The tiered-testing stage for the PTY smoke test (milestone 9.1, task M9.1.22): stage 11h.

`scripts/pty-smoke.py` imports `tiers_stage` and calls it with its own `PtyProc`, `BIN`,
`run_cmd` and `fail`, right after stage 11f (`orch_stage`), so every `anthrex` here runs
with that script's isolated `ENV`: `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under
`/tmp`, and `fake-agent` as both runtimes and as the decider. Nothing here starts or
stops a daemon of its own; the script's stage 12 ends with `anthrex daemon stop`. No
client is attached when the stage starts, so it starts its own and detaches it. Like
the other stage modules, this one has no `__main__` of its own.

A two-stage plan on a scratch repository with a script profile (a module graph, a
build check, module tests and a full `check`) runs to completion; `run status` prints
each stage's `tier 3 green` line, and the TUI's `C-b T` run view shows both stage
nodes with tier 3's green mark.
"""

import os
import shutil
import sys
import time

from pty_smoke_run import (
    ACCEPT_CMD_TIMEOUT,
    POLL,
    RUN_CMD_TIMEOUT,
    _git,
    _run_state,
    _write_script,
)

# `TIER_WAIT` in `crates/cli/tests/support/run_tiers.rs` (670 s): one tiered task path,
# M8a's `RUN_WAIT` (300 s) plus the tier commands and their git calls. This stage's two
# tasks sit in different stages and run side by side, so one path, then completion's
# tier 3 on each stage (inside the same derivation's 32 commands). A language boundary,
# so the coupling is this comment: change the Rust derivation, change this value.
TIER_WAIT = 670.0

SCRIPTS = {
    "graph.sh": "printf '{\"a\":[],\"b\":[\"a\"]}\\n'\n",
    "build.sh": "echo build ok\n",
    "test.sh": "echo \"test $1::works ... ok\"\necho \"PASS $2\"\n",
    "check.sh": "echo 'test result: ok. 2 passed; 0 failed'\n",
}

PLAN = """goal = "Tier a and b"

[profile]
check = "sh check.sh"
modules = ["mods/*"]
module_graph = "sh graph.sh"
module_names = "dir"
build_check = "sh build.sh"
module_test = "sh test.sh {module}"
single_test = "sh test.sh --one {test}"
test_passed = "PASS {test}"
source = ["mods/**"]

[[task]]
id = "t1"
title = "Task t1"
size = "S"
test_mode = "check"
test_mode_reason = "smoke"
owns = ["mods/a/src.txt"]
brief = "Do t1"
acceptance = ["t1 is done"]

[[task]]
id = "t2"
title = "Task t2"
size = "S"
stage = 2
test_mode = "check"
test_mode_reason = "smoke"
owns = ["mods/b/src.txt"]
brief = "Do t2"
acceptance = ["t2 is done"]
"""


def _worker(path):
    return [
        {"git_commit": {"file": path, "content": "changed\n", "message": f"change {path}"}},
        {"mcp_call": {"tool": "task_done", "args": {"summary": f"changed {path}"}}},
    ]


REVIEWER = [
    {"mcp_call": {"tool": "submit_review", "args": {"verdict": "approve", "summary": "ok", "findings": []}}},
]


def _wait_gone(proc, text, label, fail, timeout=10.0):
    """Waits, within `timeout`, until `text` is no longer on the rendered screen."""
    deadline = time.monotonic() + timeout
    while text in proc.screen_text():
        if time.monotonic() >= deadline:
            fail(f"{label} still showed {text!r} after {timeout}s\n--- rendered screen ---\n{proc.screen_text()}")
        proc.read_available(timeout=0.2)


def tiers_stage(pty_proc, bin_path, run_cmd, fail):
    print("== stage 11h: a two-stage tiered run shows its stages and tier 3 green ==")
    repo = f"/tmp/anthrex-smoke-tiers-{os.getpid()}"
    plan = f"/tmp/anthrex-smoke-tiers-plan-{os.getpid()}.toml"
    proc = None
    try:
        os.makedirs(os.path.join(repo, "mods", "a"))
        os.makedirs(os.path.join(repo, "mods", "b"))
        _git(["init", "-q", "-b", "main"], repo, fail)
        _git(["config", "user.name", "Smoke Test"], repo, fail)
        _git(["config", "user.email", "smoke@example.com"], repo, fail)
        _git(["config", "commit.gpgsign", "false"], repo, fail)
        for name, text in SCRIPTS.items():
            with open(os.path.join(repo, name), "w") as f:
                f.write(text)
        for module in ("a", "b"):
            with open(os.path.join(repo, "mods", module, "src.txt"), "w") as f:
                f.write(f"{module}\n")
        _git(["add", "-A"], repo, fail)
        _git(["commit", "-q", "-m", "initial"], repo, fail)
        _write_script(repo, "worker-t1-1", _worker("mods/a/src.txt"))
        _write_script(repo, "worker-t2-1", _worker("mods/b/src.txt"))
        _write_script(repo, "reviewer-t1-1", REVIEWER)
        _write_script(repo, "reviewer-t2-1", REVIEWER)
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
        run_cmd(["run", "approve", run_id], timeout=RUN_CMD_TIMEOUT)

        deadline = time.monotonic() + TIER_WAIT
        while True:
            run = _run_state(run_cmd, run_id, fail)
            if run["state"] == "complete":
                break
            if time.monotonic() >= deadline:
                fail(f"tiered run {run_id} did not complete within {TIER_WAIT}s; last state {run['state']}")
            time.sleep(POLL)
        tasks = {t["id"]: t["state"] for t in run["tasks"]}
        if tasks != {"t1": "merged", "t2": "merged"}:
            fail(f"run {run_id} completed with {tasks}")
        status = run_cmd(["run", "status", run_id], timeout=RUN_CMD_TIMEOUT).stdout
        for n in (1, 2):
            line = next((l for l in status.splitlines() if l.startswith(f"stage {n}/2 ")), "")
            if "tier 3 green" not in line:
                fail(f"`run status` has no `tier 3 green` line for stage {n}:\n{status}")
        print(f"ok: run {run_id} completed with tier 3 green on both stages")

        proc = pty_proc([bin_path])
        proc.wait_for("agents", label="tiers-stage attach banner")
        proc.send(b"\x02T")
        proc.wait_for(" tree overview ", label="the project overview")
        proc.send(b"/")
        proc.wait_for(" FILTER ", label="overview filter mode")
        proc.send(h4.encode())
        proc.send(b"\r")
        proc.wait_for(" OVERVIEW ", label="overview navigation after filtering")
        proc.send(b"l")
        proc.wait_for(f"run {h4}  2/2", label="the run's node in the project overview")
        proc.send(b"l")
        proc.wait_for(f" run · Tier a and b · {h4} ", label="the run view's title")
        proc.wait_for("stage 1/2  tier 3 ✓", label="stage 1's node, tier 3 green")
        proc.wait_for("stage 2/2  tier 3 ✓", label="stage 2's node, tier 3 green")
        print("ok: the run view shows both stage nodes with tier 3 green")

        proc.send(b"\x1b")
        proc.wait_for(" tree overview ", label="the project overview after leaving the run view")
        proc.send(b"\x1b")
        _wait_gone(proc, " tree overview ", "the project overview after its Esc", fail)
        proc.send(b"\x02d")
        exit_status = proc.wait_exit(timeout=5.0)
        if not os.WIFEXITED(exit_status) or os.WEXITSTATUS(exit_status) != 0:
            fail(f"stage-11h detach did not exit cleanly with status 0 (raw status {exit_status})")
        proc.close()
        proc = None

        run_cmd(["run", "discard", run_id, "--confirm", run_id], timeout=ACCEPT_CMD_TIMEOUT)
        print(f"ok: run {run_id} was discarded; the client detached cleanly")
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
