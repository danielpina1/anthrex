"""The design-flow stage for the PTY smoke test (milestone 9.6, task M9.6.21): stage 11k.

`scripts/pty-smoke.py` imports `design_stage` and calls it with its own `PtyProc`, `BIN`,
`run_cmd`, `fail` and `ENV`, right after stage 11g (`tuning_stage`) and before
`tui_stage`. Like stages 11g, 11i and 11j, it runs a daemon of its own: its own socket,
data directory and `ANTHREX_CONFIG` under one fresh `/tmp` directory. Every `*_BIN` is
pinned here, not inherited: `ANTHREX_CLAUDE_BIN`, `ANTHREX_CODEX_BIN` and
`ANTHREX_DECIDER_BIN` are the `fake-agent` beside `BIN`, and `ANTHREX_GH_BIN` is a
nonexistent path, so no real agent or `gh` can start. The stage always ends with
`anthrex daemon stop` under the same variables, also when it fails, so no other daemon
is ever touched. Like the other stage modules, it has no `__main__` of its own
(`ANTHREX_SMOKE_ONLY=11k python3 scripts/pty-smoke.py` runs it alone).

In the TUI: `C-b g` on the project, the goal typed, the dialog's `design` row moved from
`configured (off)` to `full`, and Ctrl-S. The scripted orchestrator asks one question in
its window; Enter on the run's root focuses it and the user types `skip`. Two scripted
brainstormers (`claude` and `codex`) submit their drafts and the orchestrator merges
them. Each gate is reached from its Alerts row (`C-b a`, Enter, the menu's preselected
entry): the brainstorm is approved on the gate screen (`a`, then `y`); the spec v1 gets
`c` with a note, the orchestrator writes v2, and v2 is approved; the plan is approved on
the plan review. The run completes, `anthrex run accept` lands it, and the approved
spec v2 is on `main` under `docs/anthrex/specs/`.

The bounds, the configuration, the documents and the agents' scripts are in
`scripts/pty_smoke_design_fixtures.py`. The bounds restate the Rust rows across the
language boundary (`docs/timing-budgets.md`, "Recorded, from M9.6.21"): `ORCH_WAIT`
(120 s, `support/run_orch.rs`) for each gate, review and round of drafts;
`DESIGN_RUN_WAIT` (`RUN_WAIT` + the documents commit's `IO_WAIT + 30 ×
git_timeout_secs`, `run_e2e_design/common.rs`) for the run after its plan's approval.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

from pty_smoke_adapt import GOAL_CMD_TIMEOUT, STORED_PROFILE
from pty_smoke_design_fixtures import (
    ORCH_WAIT,
    DESIGN_RUN_WAIT,
    GIT_TIMEOUT_SECS,
    GOAL,
    LABELS,
    ANSWER,
    NOTE,
    SHELL,
    SIDEBAR,
    ORCH,
    CONFIG,
    TRIAGE,
    ORCHESTRATOR,
    SPEC_REVIEWER,
    PLAN_REVIEWER,
    WORKER,
    _brainstormer,
    _git_bytes,
    _write,
)
from pty_smoke_keep_going import REQUEST_WAIT, SCREEN_WAIT, _mcp_calls
from pty_smoke_pr import DAEMON_START_WAIT, _reap, _stop_daemon
from pty_smoke_run import (
    ACCEPT_CMD_TIMEOUT,
    POLL,
    REVIEWER,
    RUN_CMD_TIMEOUT,
    _git,
    _write_script,
)


def design_stage(pty_proc, bin_path, run_cmd, fail, base_env):
    print("== stage 11k: a goal through the brainstorm, spec and plan gates ==")
    root = tempfile.mkdtemp(prefix="anthrex-smoke-design-", dir="/tmp")
    socket = os.path.join(root, "d.sock")
    data = os.path.join(root, "data")
    repo = os.path.join(root, "repo")
    config = os.path.join(root, "config.toml")
    deciders = os.path.join(root, "deciders")
    mcp_log = os.path.join(root, "mcp.jsonl")
    fake = os.path.join(os.path.dirname(bin_path), "fake-agent")
    env = dict(base_env)
    env.update(
        {
            "ANTHREX_SOCKET": socket,
            "ANTHREX_DATA_DIR": data,
            "ANTHREX_CONFIG": config,
            # Pinned here, never inherited: no real agent, decider or `gh` can start.
            "ANTHREX_CLAUDE_BIN": fake,
            "ANTHREX_CODEX_BIN": fake,
            "ANTHREX_DECIDER_BIN": fake,
            "ANTHREX_GH_BIN": "/nonexistent/anthrex-smoke/gh",
            "FAKE_AGENT_DECIDER_DIR": deciders,
            "FAKE_AGENT_SCRIPT": os.path.join(root, "no-such-script.json"),
            "FAKE_AGENT_TRANSCRIPT": os.path.join(root, "transcript.jsonl"),
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

    def until(what, within, probe):
        """`probe()` until it is truthy, for at most `within` s, draining the client."""
        deadline = time.monotonic() + within
        while True:
            value = probe()
            if value:
                return value
            if time.monotonic() >= deadline:
                screen = f"\n--- rendered screen ---\n{proc.screen_text()}" if proc else ""
                fail(f"stage 11k: {what} did not happen within {within:.0f}s{screen}")
            if proc is not None:
                proc.read_available(timeout=POLL)
            else:
                time.sleep(POLL)

    def wait_run(run_id, what, pred, within):
        def probe():
            run = run_of(run_id)
            return run if pred(run) else None

        return until(f"run {run_id[-4:]}: {what}", within, probe)

    def gate_of(r):
        g = r.get("doc_gate") or {}
        return g.get("kind"), g.get("version"), g.get("revising")

    def at_gate(run_id, kind, version):
        def open_(r):
            return r["state"] == "awaiting_approval" and gate_of(r) == (kind, version, None)

        return wait_run(run_id, f"the {kind} gate at v{version}", open_, ORCH_WAIT)

    def sidebar_alerts():
        """The sidebar's Alerts box as one line: its rows' text inside the frame, joined
        with spaces, so a row wrapped to the sidebar's width reads whole."""
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

    def from_alert(alert):
        """`C-b a`, the gate's Alerts row (exact, read across its wrap), Enter (the action
        menu, its entry preselected) and Enter again: the gate's screen."""
        proc.send(b"\x02a")
        until(f"the alert {alert!r}", SCREEN_WAIT, lambda: alert in sidebar_alerts())
        proc.send(b"\r")
        proc.wait_for("j/k move", label="the alert's action menu")
        proc.send(b"\r")

    def closed(header):
        """An approve's reply closes its gate screen (`app/doc_gate_replies.rs`)."""
        gone = lambda: header not in proc.screen_text()  # noqa: E731
        until(f"the gate screen {header!r} closing", SCREEN_WAIT, gone)

    def confirm(page, run_id, left):
        """`a` once the shown document has loaded (final fix wave FW-54: before that the
        gate screen answers `a` with a note and opens no confirm page), then `y`."""
        loaded = lambda: "loading" not in proc.screen_text()  # noqa: E731
        until("the shown document loading", SCREEN_WAIT, loaded)
        proc.send(b"a")
        proc.wait_for(page, label=f"the confirm page {page!r}")
        proc.send(b"y")
        wait_run(run_id, f"past {page!r}", left, REQUEST_WAIT)

    try:
        os.makedirs(repo)
        os.makedirs(deciders)
        unconfined = "" if sys.platform == "darwin" else "unconfined_checks = true\n"
        _write(config, CONFIG.format(git=GIT_TIMEOUT_SECS, unconfined=unconfined))
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
        for label in LABELS:
            _write_script(repo, f"brainstormer-{label}-1", _brainstormer(label))
        _write_script(repo, "doc_reviewer-spec-r1-1", SPEC_REVIEWER)
        _write_script(repo, "doc_reviewer-plan-r1-1", PLAN_REVIEWER)
        _write_script(repo, "worker-t1-1", WORKER)
        _write_script(repo, "reviewer-t1-1", REVIEWER)

        # The daemon is this stage's own child, in its own session, as stage 11j's.
        daemon = subprocess.Popen(
            [bin_path, "daemon", "start", "--foreground"],
            cwd=root,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        bound = lambda: os.path.exists(socket) or daemon.poll() is not None  # noqa: E731
        until("the stage's daemon binding its socket", DAEMON_START_WAIT, bound)
        if daemon.poll() is not None:
            fail(f"stage 11k: its daemon exited at its start ({daemon.returncode})")

        # The profile a goal needs, stored as stage 11j does.
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

        # 1. `C-b g` on the project; the goal; the design row to `full`; Ctrl-S.
        proc = pty_proc([bin_path], env=env)
        proc.wait_for("agents", label="stage-11k attach banner")
        proc.wait_for(SHELL, label="the repository's shell in the sidebar")
        proc.send(b"\x02t")
        proc.wait_for(" TREE ", label="tree navigation mode")
        proc.send(f"/{SHELL}".encode())
        proc.wait_for(" FILTER ", label="tree filter mode")
        proc.send(b"\r")
        proc.wait_for(" TREE ", label="tree navigation after the filter")
        proc.send(b"k")
        proc.send(b"\x02g")
        proc.wait_for(f" start a goal in {project} ", label="the goal dialog on the repository")
        proc.send(GOAL.encode())
        proc.wait_for(GOAL, label="the typed goal")
        # Tab from the text: model, effort, orchestrator, delivery, design.
        proc.send(b"\t" * 5)
        proc.send(b"\x1b[C")

        def design_row():
            lines = proc.screen_text().splitlines()
            return any("design" in line and "‹ full ›" in line for line in lines)

        until("the goal dialog's design row reading `full`", SCREEN_WAIT, design_row)
        proc.send(b"\x13")

        def listed():
            runs = json_of(["run", "status", "--json"])["runs"]
            return [r["run_id"] for r in runs if r["goal"] == GOAL]

        run_id = until("the goal's run listed", GOAL_CMD_TIMEOUT, listed)[0]
        h4 = run_id[-4:]
        view = f" run · {GOAL} · {h4} "
        proc.wait_for(view, timeout=GOAL_CMD_TIMEOUT, label="the run view on the new run")
        run = run_of(run_id)
        if run.get("path") != "plan" or run.get("design") != "full":
            path, design = run.get("path"), run.get("design")
            fail(f"run {run_id} is not a design run: path {path!r}, design {design!r}")
        print(f"ok: the goal dialog started design run {run_id} with design: full (configured off)")

        # 2. The orchestrator's question: once it is asked and the window idles, Enter on
        # the run's root focuses the window and the user types `skip`.
        def asked():
            calls = _mcp_calls(mcp_log)
            return any(c.get("script") == ORCH and c.get("args") == {"wait_secs": 0} for c in calls)

        until("the orchestrator's question", ORCH_WAIT, asked)
        bound = lambda r: (r.get("orchestrator") or {}).get("window_id")  # noqa: E731
        window = bound(wait_run(run_id, "its orchestrator window", bound, ORCH_WAIT))

        def idle():
            windows = json_of(["ls", "--json"])
            return any(w["id"] == window and w["status"] in ("idle", "done") for w in windows)

        until("the orchestrator's window idle at its question", ORCH_WAIT, idle)
        proc.send(b"\r")
        proc.wait_for(f"{h4}/orchestrator · ", label="the focused orchestrator window")
        proc.send(f"{ANSWER}\r".encode())
        print("ok: the user typed skip into the orchestrator's window")

        # 3. The brainstorm v1, approved on its gate screen.
        at_gate(run_id, "brainstorm", 1)
        from_alert(f"brainstorm v1 ready for review · run {h4}")
        proc.wait_for(f"Brainstorm · run {h4} · v1 of ", label="the brainstorm gate screen")
        page = "Approve brainstorm v1? Spec starts next."
        confirm(page, run_id, lambda r: gate_of(r)[0] != "brainstorm")
        closed(f"Brainstorm · run {h4} · ")
        print("ok: the brainstorm v1 was approved with a, then y")

        # 4. The spec v1: `c` with a note; the orchestrator's v2 approved.
        at_gate(run_id, "spec", 1)
        from_alert(f"spec v1 ready for review · run {h4}")
        proc.wait_for(f"Spec · run {h4} · v1 of ", label="the spec gate screen")
        proc.send(b"c")
        proc.wait_for("ask for changes · spec v1", label="the changes editor")
        proc.send(NOTE.encode())
        proc.wait_for(NOTE, label="the typed note")
        proc.send(b"\x13")
        revised = at_gate(run_id, "spec", 2)
        versions = [d["version"] for d in revised.get("docs", []) if d.get("kind") == "spec"]
        if 1 not in versions or 2 not in versions:
            fail(f"run {run_id} lists spec versions {versions}, not v1 and v2")
        proc.wait_for(f"Spec · run {h4} · v2 of ", label="the gate screen following to v2")
        confirm("Approve spec v2? Planning starts next.", run_id, lambda r: gate_of(r)[0] != "spec")
        closed(f"Spec · run {h4} · ")
        print("ok: c asked for changes on the spec v1; the spec v2 was approved")

        # 5. The plan v1, approved on the plan review.
        at_gate(run_id, "plan", 1)
        from_alert(f"plan v1 ready for review · run {h4}")
        proc.wait_for(f"plan · {GOAL}", label="the plan review")
        page = "Approve plan v1? The run starts next."
        confirm(page, run_id, lambda r: r["state"] != "awaiting_approval")
        print("ok: the plan v1 was approved on the plan review")

        # 6. The run completes; `anthrex run accept`.
        run = wait_run(run_id, "complete", lambda r: r["state"] == "complete", DESIGN_RUN_WAIT)
        tasks = {t["id"]: t["state"] for t in run["tasks"]}
        if tasks != {"t1": "merged"}:
            fail(f"run {run_id} completed with {tasks}")
        # Esc leaves the plan review, if it is still open, before the detach; a bare Esc
        # sent with the prefix would read as Alt.
        if f"plan · {GOAL}" in proc.screen_text():
            proc.send(b"\x1b")
            closed(f"plan · {GOAL}")
        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=5.0)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"stage-11k detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None
        cmd(["run", "accept", run_id, "--yes", "--dir", repo], timeout=ACCEPT_CMD_TIMEOUT)
        print(f"ok: run {h4} completed with t1 merged and was accepted")

        # 7. The approved spec v2 is on main under `docs/anthrex/specs/`.
        listed = _git_bytes(["ls-tree", "--name-only", "main", "docs/anthrex/specs/"], repo, fail)
        specs = [p for p in listed.decode().splitlines() if p.endswith(".md")]
        if len(specs) != 1:
            fail(f"main's docs/anthrex/specs/ holds {specs}, not one spec")
        with open(os.path.join(data, "runs", run_id, "design", "spec-v2.md"), "rb") as f:
            stored = f.read()
        if _git_bytes(["cat-file", "blob", f"main:{specs[0]}"], repo, fail) != stored:
            fail(f"main's {specs[0]} is not the approved spec v2 byte for byte")
        if _git(["show", "main:a.txt"], repo, fail) != "a":
            fail("a.txt is not on main after the accept")
        print(f"ok: the approved spec v2 is on main as {specs[0]}, with a.txt")
    finally:
        if proc is not None:
            _reap(proc)
        _stop_daemon(run_cmd, env, socket, daemon, "11k")
        if not os.environ.get("ANTHREX_SMOKE_KEEP"):
            shutil.rmtree(root, ignore_errors=True)
