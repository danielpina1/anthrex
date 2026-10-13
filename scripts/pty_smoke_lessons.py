"""The lessons stage for the PTY smoke test (milestone 9.15, task M9.15.16): stage 11n.

`scripts/pty-smoke.py` imports `lessons_stage` and calls it with its own `PtyProc`,
`BIN`, `run_cmd`, `fail` and `ENV`, right after stage 11g (`tuning_stage`). Like stages
11g, 11i and 11j it runs a daemon of its own: its own socket, data directory and
`ANTHREX_CONFIG` under its own `/tmp` root, a nonexistent `ANTHREX_GH_BIN`, and
`fake-agent` as both runtimes and the decider (from the script's `ENV`), so no real
agent or `gh` can start. The stage always ends with `anthrex daemon stop` under the same
variables, also when it fails, and removes its paths. Its config opts into lessons
(ruling C3: every other stage runs with them off), risk notes off. Like the other stage
modules it has no `__main__` (`ANTHREX_SMOKE_ONLY=11n python3 scripts/pty-smoke.py`
runs it alone).

1. Run 1: task t1's first check fails (it commits `mods/alpha/broken`), the worker
   removes the marker and the task merges: recovered. Its distill entry is answered by
   the scripted `distill-1.json` (one episodic item, one procedural item), and the
   stage waits for the `distilled` line in `lessons.jsonl`.
2. `anthrex lessons list` shows one `on_trial` and one `proposed` lesson;
   `anthrex lessons approve <id> --yes` prints `approved 1`.
3. Run 2: task t2's worker's first turn (read from `fake-agent`'s stdin log,
   `FAKE_AGENT_STDIN_FILE`, ruling C22) carries the worker block's heading and both
   lessons.
4. Run 3 is discarded with `--reason "smoke reason"`; its run record carries it.
5. The TUI: `C-b P`, `l` lists the two lessons; `g` draws the graph, its three hubs
   and two lesson dots.
"""

import json
import os
import shutil
import subprocess
import sys
import time

from pty_smoke_pr import DAEMON_START_WAIT, _stop_daemon
from pty_smoke_run import ACCEPT_CMD_TIMEOUT, POLL, RUN_CMD_TIMEOUT, RUN_WAIT, _git, _write_script
from pty_smoke_tui import DETACH_WAIT, SCREEN_WAIT
from smoke_task_tmp import teardown

# `DISTILL_BOUND` (`crates/daemon/src/lessons/service.rs`, 120 s): one distill call's
# own bound, routing probe included. Ruling C22: the wait for the `distilled` line is
# the stage's run wait plus it (the worker is woken once the run's `QueueDistill` op is
# written, which the run's completion waits for). A language boundary: change the Rust
# constant, change this value.
DISTILL_BOUND = 120.0
DISTILLED_WAIT = RUN_WAIT + DISTILL_BOUND

# The Lessons page's list and the graph page's graph: one tagged request each, answered
# off the engine on `spawn_blocking`; the client gives up at `REPLY_TIMEOUT` (30 s,
# `crates/tui/src/app/replies.rs`). 30 s + 15 s of margin, as stage 11l's `CARD_WAIT`.
PAGE_WAIT = 45.0

# The worker block's heading (decision 23, the brief's "Exact user-visible text").
WORKER_HEADING = (
    "Notes from earlier runs in this repository. They are advisory; your task and its "
    "acceptance criteria above take precedence."
)

EPISODIC = {
    "layer": "episodic",
    "title": "Delete the broken marker first",
    "body": "The check fails while mods/alpha/broken exists.",
    "files": ["mods/alpha/broken"],
}
PROCEDURAL = {
    "layer": "procedural",
    "title": "Run the check before claiming done",
    "body": "Run sh check.sh in the checkout before task_done.",
    "files": ["mods/alpha/**"],
}
DISTILL = {"answer": {"items": [EPISODIC, PROCEDURAL]}}

CHECK_SH = "if [ -e mods/alpha/broken ]; then echo 'error: broken is present'; exit 1; fi\necho check ok\n"

CONFIG = """[orchestrator]
git_timeout_secs = 5
deciders.mode = "claude"
deciders.timeout_secs = 30
{unconfined}
[orchestrator.profile]
check_timeout_secs = 10

# Milestone 9.15 ruling C3: this stage opts into lessons; risk notes stay off.
[lessons]
enabled = true
distill = true
risk_note = false
"""

# Written once the runs are done, for the graph page's hubs (decision 41: the stored
# profile's `modules`, matched in the checkout, when no module-graph cache exists).
STORED_PROFILE = 'check = "sh check.sh"\ncheck_timeout_secs = 10\nmodules = ["mods/*"]\n'


def _plan(task, owns):
    return f"""goal = "Lessons {task}"

[profile]
check = "sh check.sh"

[[task]]
id = "{task}"
title = "Task {task}"
size = "S"
test_mode = "check"
test_mode_reason = "smoke"
owns = ["{owns}"]
brief = "Do {task}"
acceptance = ["{task} is done"]
"""


def _commit(path, content):
    return {"git_commit": {"file": path, "content": content, "message": f"add {path}"}}


def _done(summary):
    return {"mcp_call": {"tool": "task_done", "args": {"summary": summary}}}


APPROVE = {
    "mcp_call": {
        "tool": "submit_review",
        "args": {"verdict": "approve", "summary": "ok", "findings": []},
    }
}

# `recovered_task_gives_an_on_trial_lesson_listed_by_the_cli`'s worker.
RECOVERED = [
    _commit("mods/alpha/src.txt", "alpha 2\n"),
    _commit("mods/alpha/broken", "x\n"),
    _done("changed alpha"),
    {"read_message": {"expect": "The check failed"}},
    {"sh": {"cmd": "git rm -q mods/alpha/broken && git commit -qm 'remove broken'"}},
    _done("fixed the check"),
]


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def _store_lines(path):
    """`lessons.jsonl`'s lines as JSON (none while it does not exist); a torn last line
    is left for the next read."""
    try:
        with open(path) as f:
            text = f.read()
    except FileNotFoundError:
        return []
    lines = []
    for line in text.splitlines():
        try:
            lines.append(json.loads(line))
        except ValueError:
            pass
    return lines


def _strings(value):
    """Every string inside a JSON value (a stdin line is a stream-json message)."""
    if isinstance(value, str):
        return [value]
    if isinstance(value, list):
        return [s for v in value for s in _strings(v)]
    if isinstance(value, dict):
        return [s for v in value.values() for s in _strings(v)]
    return []


def _first_turn(path):
    """The first stdin line `fake-agent` logged at `path`, its text, or None."""
    try:
        with open(path) as f:
            first = f.readline()
    except FileNotFoundError:
        return None
    if not first.endswith("\n"):
        return None
    try:
        return "\n".join(_strings(json.loads(first)))
    except ValueError:
        return first


def _frame(screen, title):
    """The rows inside the box whose top border carries `title`, each cut to the box's
    columns; [] when no such box is drawn."""
    rows = screen.splitlines()
    for i, row in enumerate(rows):
        at = row.find(title)
        if at < 0:
            continue
        left = max(row.rfind(c, 0, at) for c in "╭┌")
        right = min((j for j in (row.find(c, at) for c in "╮┐") if j >= 0), default=-1)
        if left < 0 or right < 0:
            continue
        inside = []
        for below in rows[i + 1 :]:
            if below[left : left + 1] in ("╰", "└"):
                return inside
            inside.append(below[left + 1 : right])
        return inside
    return []


def lessons_stage(pty_proc, bin_path, run_cmd, fail, base_env):
    print("== stage 11n: a recovered task's lessons reach the next worker and the pages ==")
    repo = f"/tmp/anthrex-smoke-lessons-{os.getpid()}"
    root = f"{repo}-daemon"
    socket = os.path.join(root, "d.sock")
    config = os.path.join(root, "config.toml")
    deciders = os.path.join(root, "deciders")
    io = os.path.join(root, "io")
    env = dict(base_env)
    env.update(
        {
            "ANTHREX_SOCKET": socket,
            "ANTHREX_DATA_DIR": os.path.join(root, "data"),
            "ANTHREX_CONFIG": config,
            # Never a real `gh`; the runtimes and the decider stay `fake-agent` (`ENV`).
            "ANTHREX_GH_BIN": "/nonexistent/anthrex-smoke/gh",
            "FAKE_AGENT_DECIDER_DIR": deciders,
            "FAKE_AGENT_SCRIPT": os.path.join(root, "no-such-script.json"),
            "FAKE_AGENT_STDIN_FILE": io,
            "FAKE_AGENT_TRANSCRIPT": os.path.join(root, "transcript.jsonl"),
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
        }
    )
    for path in (repo, root):
        if os.path.exists(path):
            fail(f"stage 11n: {path} already exists, and is not this stage's to remove")
    daemon = None
    proc = None

    def cmd(args, timeout=RUN_CMD_TIMEOUT, expect_ok=True):
        return run_cmd(args, expect_ok=expect_ok, timeout=timeout, env=env)

    def json_of(args):
        out = cmd(args).stdout
        try:
            return json.loads(out)
        except ValueError:
            fail(f"`anthrex {' '.join(args)}` printed no JSON:\n{out}")

    def run_to_complete(task, owns):
        _write(os.path.join(root, f"plan-{task}.toml"), _plan(task, owns))
        started = cmd(["run", "start", "--plan", os.path.join(root, f"plan-{task}.toml"), "--dir", repo, "--yes"])
        run_id = started.stdout.strip()
        if not run_id or "\n" in run_id:
            fail(f"`anthrex run start` printed no single run id:\n{started.stdout}")
        deadline = time.monotonic() + RUN_WAIT
        while True:
            runs = json_of(["run", "status", run_id, "--json"]).get("runs") or []
            if len(runs) != 1:
                fail(f"`anthrex run status {run_id} --json` listed {len(runs)} runs")
            run = runs[0]
            if run["state"] == "complete":
                break
            if time.monotonic() >= deadline:
                fail(f"run {run_id} did not complete within {RUN_WAIT}s:\n{json.dumps(run, indent=2)}")
            time.sleep(POLL)
        states = {t["id"]: t["state"] for t in run["tasks"]}
        if states != {task: "merged"}:
            fail(f"run {run_id} completed with {states}")
        return run_id, run

    def until(what, within, probe):
        deadline = time.monotonic() + within
        while True:
            value = probe()
            if value:
                return value
            if time.monotonic() >= deadline:
                screen = f"\n--- rendered screen ---\n{proc.screen_text()}" if proc else ""
                fail(f"stage 11n: {what} did not happen within {within:.0f}s{screen}")
            if proc is not None:
                proc.read_available(timeout=POLL)
            else:
                time.sleep(POLL)

    try:
        os.makedirs(repo)
        os.makedirs(io)
        unconfined = "" if sys.platform == "darwin" else "unconfined_checks = true\n"
        _write(config, CONFIG.format(unconfined=unconfined))
        _write(os.path.join(deciders, "distill-1.json"), json.dumps(DISTILL))

        # A repository with an identity, `check.sh`, two module directories and a commit.
        _git(["init", "-q", "-b", "main"], repo, fail)
        _git(["config", "user.name", "Smoke Test"], repo, fail)
        _git(["config", "user.email", "smoke@example.com"], repo, fail)
        _git(["config", "commit.gpgsign", "false"], repo, fail)
        _write(os.path.join(repo, "README"), "readme\n")
        _write(os.path.join(repo, "check.sh"), CHECK_SH)
        _write(os.path.join(repo, "mods", "alpha", "src.txt"), "alpha\n")
        _write(os.path.join(repo, "mods", "beta", "src.txt"), "beta\n")
        _git(["add", "-A"], repo, fail)
        _git(["commit", "-q", "-m", "initial"], repo, fail)
        _write_script(repo, "worker-t1-1", RECOVERED)
        _write_script(repo, "reviewer-t1-1", [APPROVE])
        _write_script(repo, "worker-t2-1", [_commit("mods/alpha/src.txt", "alpha 3\n"), _done("changed alpha")])
        _write_script(repo, "reviewer-t2-1", [APPROVE])
        _write_script(repo, "worker-t3-1", [_commit("mods/beta/src.txt", "beta 2\n"), _done("changed beta")])
        _write_script(repo, "reviewer-t3-1", [APPROVE])

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
                fail(f"stage 11n: its daemon did not bind {socket} within {DAEMON_START_WAIT}s")
            time.sleep(POLL)
        if daemon.poll() is not None:
            fail(f"stage 11n: its daemon exited at its start ({daemon.returncode})")
        repo_dir = json_of(["profile", "status", "--json", "--dir", repo])["repo_dir"]
        store = os.path.join(repo_dir, "lessons.jsonl")

        # 1. A recovered task, distilled in the background.
        run1, run = run_to_complete("t1", "mods/alpha/**")
        if run["tasks"][0]["bounces"]["check"] != 1:
            fail(f"run {run1}'s t1 did not bounce once on its check: {run['tasks'][0]['bounces']}")

        def distilled():
            lines = _store_lines(store)
            for line in lines:
                # The scripted answer is the first attempt's; a fallback is a failure now,
                # not after the retries (60 s, 300 s) and the `gave_up`.
                if line.get("type") in ("attempt", "gave_up") and (line.get("error") or line.get("reason")):
                    fail(f"stage 11n: the distill call fell back:\n{json.dumps(line)}")
            return [line for line in lines if line.get("type") == "distilled"]

        got = until("a distilled line in lessons.jsonl", DISTILLED_WAIT, distilled)
        if len(got) != 1 or len(got[0].get("items", [])) != 2:
            fail(f"stage 11n: lessons.jsonl's distilled lines are not one of two items:\n{got}")
        print(f"ok: run {run1} recovered t1, and its distill entry was answered with two items")

        # 2. The CLI lists them, and approves the proposed one.
        lessons = json_of(["--dir", repo, "lessons", "list", "--json"])
        by_status = {lesson["status"]: lesson for lesson in lessons}
        if len(lessons) != 2 or set(by_status) != {"on_trial", "proposed"}:
            fail(f"`anthrex lessons list --json` did not list one on_trial and one proposed:\n{lessons}")
        on_trial, proposed = by_status["on_trial"], by_status["proposed"]
        if (on_trial["title"], proposed["title"]) != (EPISODIC["title"], PROCEDURAL["title"]):
            fail(f"the listed lessons are not the distilled ones:\n{lessons}")
        text = cmd(["--dir", repo, "lessons", "list"]).stdout
        for lesson, status in ((on_trial, "on_trial"), (proposed, "proposed")):
            row = [line for line in text.splitlines() if line.startswith(f"{lesson['id']}  {status}  ")]
            if not row or lesson["title"] not in row[0]:
                fail(f"`anthrex lessons list` has no {status} row for {lesson['id']}:\n{text}")
        approved = cmd(["--dir", repo, "lessons", "approve", proposed["id"], "--yes"]).stdout
        if approved != "approved 1\n":
            fail(f"`anthrex lessons approve {proposed['id']} --yes` printed {approved!r}")
        print(f"ok: lessons list showed {on_trial['id']} on_trial and {proposed['id']} proposed; approved 1")

        # 3. The next run's worker sees both lessons under the worker heading.
        run2, _ = run_to_complete("t2", "mods/alpha/**")
        prompt = _first_turn(os.path.join(io, "worker-t2-1.stdin"))
        if prompt is None:
            fail(f"run {run2}'s worker logged no first turn in {io}: {sorted(os.listdir(io))}")
        wanted = [WORKER_HEADING] + [
            f"[{lesson['id']} · {lesson['layer']}] {lesson['title']}" for lesson in (on_trial, proposed)
        ]
        missing = [w for w in wanted if w not in prompt]
        if missing:
            fail(f"run {run2}'s worker prompt lacks {missing}:\n{prompt}")
        print(f"ok: run {run2}'s worker prompt carries the worker heading and both lessons")

        # 4. A discard's reason reaches the run record.
        run3, _ = run_to_complete("t3", "mods/beta/**")
        cmd(["run", "discard", run3, "--reason", "smoke reason", "--confirm", run3], timeout=ACCEPT_CMD_TIMEOUT)
        history = os.path.join(repo_dir, "history.jsonl")

        def run_record():
            return [
                line
                for line in _store_lines(history)
                if line.get("type") == "run" and line.get("run_id") == run3
            ]

        record = until(f"run {run3}'s run record", RUN_WAIT, run_record)[0]
        if record.get("outcome") != "discarded" or record.get("discard_reason") != "smoke reason":
            fail(f"run {run3}'s run record does not carry the reason:\n{json.dumps(record)}")
        print(f"ok: run {run3} was discarded, and its run record carries \"smoke reason\"")

        # 5. The pages. The stored profile's modules give the graph its hubs.
        meta = {
            "confirmed_at": int(time.time()),
            "report": None,
            "verification": None,
            "fingerprint": {},
            "edited_keys": [],
            "project": json_of(["profile", "status", "--json", "--dir", repo])["project"],
        }
        _write(os.path.join(repo_dir, "profile.meta.json"), json.dumps(meta))
        _write(os.path.join(repo_dir, "profile.toml"), STORED_PROFILE)
        name = os.path.basename(repo)
        # `--dir`: the start directory `C-b P` falls back to; every run here is the repo's.
        proc = pty_proc([bin_path, "--dir", repo], env=env)
        proc.wait_for("agents", timeout=SCREEN_WAIT, label="stage-11n attach banner")
        proc.send(b"\x02P")
        proc.wait_for(" PROFILE ", timeout=SCREEN_WAIT, label="the Profile screen's badge")
        proc.wait_for(f"profile · {name}", timeout=SCREEN_WAIT, label="the Profile screen's project")
        proc.wait_for("l lessons", timeout=PAGE_WAIT, label="the Profile screen's `l lessons` hint")
        proc.send(b"l")

        def listed():
            inside = "\n".join(_frame(proc.screen_text(), f"lessons · {name}"))
            return all(lesson["title"] in inside for lesson in (on_trial, proposed))

        until("the Lessons page listing both lessons", PAGE_WAIT, listed)
        print("ok: C-b P, l: the Lessons page lists the two lessons")

        proc.send(b"g")
        hubs = ("◉ (root)", "◉ alpha", "◉ beta")

        def drawn():
            inside = "\n".join(_frame(proc.screen_text(), f"Lessons · {name}"))
            return all(hub in inside for hub in hubs) and inside.count("•") == 2

        until("the graph's three hubs and two lesson dots", PAGE_WAIT, drawn)
        print("ok: g: the graph page draws the hubs (root), alpha and beta and two lesson dots")

        # Back out one level at a time (an Esc and a C-b in one read decode as Alt+C-b).
        proc.send(b"\x1b")
        until("the graph page closing", SCREEN_WAIT, lambda: f"Lessons · {name}" not in proc.screen_text())
        proc.send(b"\x1b")
        until("the Lessons page closing", SCREEN_WAIT, lambda: f"lessons · {name}" not in proc.screen_text())
        proc.send(b"\x1b")
        until("the Profile screen closing", SCREEN_WAIT, lambda: " PROFILE " not in proc.screen_text())
        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=DETACH_WAIT)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"stage-11n detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None
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
        _stop_daemon(run_cmd, env, socket, daemon, "11n")
        if not os.environ.get("ANTHREX_SMOKE_KEEP"):
            shutil.rmtree(repo, ignore_errors=True)
            # M9.12.13: its runs' task temporary directories go with it.
            teardown(root)
            shutil.rmtree(root, ignore_errors=True)
