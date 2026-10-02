"""The TUI end-to-end stage for the PTY smoke test (milestone 9.0.6, task 16): stage 11t.

`scripts/pty-smoke.py` imports `tui_stage` and calls it with its own `PtyProc`, `BIN`,
`run_cmd` and `fail`, right after stage 11g (`tiers_stage`), so every `anthrex` here runs
with that script's isolated `ENV`: `ANTHREX_SOCKET`, `ANTHREX_DATA_DIR` and
`ANTHREX_CONFIG` under `/tmp` (the config under the pid-named directory
`/tmp/anthrex-smoke-data-<pid>/`), and `fake-agent` as both runtimes and as the decider.
Nothing here starts or stops a daemon of its own. No client is attached when the stage
starts, so it starts its own and detaches it.

The stage saves Settings through the daemon, which writes `ANTHREX_CONFIG`. Every other
stage but 12b needs that path absent (`ensure_config_path_absent` in `pty-smoke.py`), so
this stage removes the file's directory on every exit, as stage 12b does. The daemon
keeps the saved settings live until stage 12 stops it; no stage in between starts a run.
Like `scripts/pty_smoke_run.py`, whose helpers it reuses, this module has no `__main__`.
"""

import json
import os
import shutil
import sys
import time

from pty_smoke_run import POLL, RUN_CMD_TIMEOUT, RUN_WAIT, _git, _run_state, _write_script

# A key's own frame: the client redraws on its 100 ms tick (`crates/tui/src/lib.rs`), and
# nothing drawn here waits on the daemon: the menu is built from the snapshot the client
# already holds, and a screen's badge is drawn the moment it opens. `PtyProc.wait_for`'s
# default, which stage 11f's screens use too; about 100 ticks.
SCREEN_WAIT = 10.0

# The Settings screen's rows when it opens: from the client's settings cache, filled by
# the `Settings(Get)` it sends on connecting (answered from the daemon's live handle,
# no I/O), or else one `Get` the screen sends itself. The client gives up on a reply at
# `REPLY_TIMEOUT` (30 s, `crates/tui/src/app/replies.rs`) and shows `no reply from
# daemon`; 30 s + 15 s of margin.
SETTINGS_LOAD_WAIT = 45.0

# The saved line after `w`: the daemon takes its settings write lock within
# `SETTINGS_WRITE_TIMEOUT` (5 s, `crates/daemon/src/live_config.rs`), then writes and
# renames `config.toml` within the same 5 s, 10 s in all; the client waits
# `REPLY_TIMEOUT` (30 s) for the reply before it shows `no reply from daemon`. 30 s + 15
# s of margin.
SETTINGS_SAVE_WAIT = 45.0

# The client's exit after `C-b d`: local, as in every other stage.
DETACH_WAIT = 5.0

# The start of the Settings screen's confirmation of a save (`SAVED`,
# `crates/tui/src/app/settings_screen.rs`).
SAVED = "saved · new runs use these settings"

PLAN = """goal = "Menu t"

[profile]
check = "true"

[[task]]
id = "t1"
title = "add a"
size = "S"
test_mode = "check"
test_mode_reason = "smoke"
owns = ["a.txt"]
brief = "Do t1"
acceptance = ["t1 is done"]
"""


def _wait_gone(proc, text, label, fail, timeout=SCREEN_WAIT):
    """Waits, within `timeout`, until `text` is no longer on the rendered screen."""
    deadline = time.monotonic() + timeout
    while text in proc.screen_text():
        if time.monotonic() >= deadline:
            fail(f"{label} still showed {text!r} after {timeout}s\n--- rendered screen ---\n{proc.screen_text()}")
        proc.read_available(timeout=0.2)


def _repo(repo, fail):
    os.makedirs(repo)
    _git(["init", "-q", "-b", "main"], repo, fail)
    _git(["config", "user.name", "Smoke Test"], repo, fail)
    _git(["config", "user.email", "smoke@example.com"], repo, fail)
    _git(["config", "commit.gpgsign", "false"], repo, fail)
    with open(os.path.join(repo, "README"), "w") as f:
        f.write("readme\n")
    _git(["add", "-A"], repo, fail)
    _git(["commit", "-q", "-m", "initial"], repo, fail)


def tui_stage(pty_proc, bin_path, run_cmd, fail, config_path):
    print("== stage 11t: the action menu, Settings and Profile from the TUI ==")
    repo = f"/tmp/anthrex-smoke-tui-{os.getpid()}"
    plan = f"/tmp/anthrex-smoke-tui-plan-{os.getpid()}.toml"
    proc = None
    try:
        _repo(repo, fail)
        # The worker never runs: the run stays at its plan gate and is rejected there.
        _write_script(repo, "worker-t1-1", [{"wait_ms": 600000}])
        with open(plan, "w") as f:
            f.write(PLAN)
        start = ["run", "start", "--plan", plan, "--dir", repo]
        if sys.platform != "darwin":
            # Checks can be confined only on macOS (stage 11c).
            start.append("--unconfined-checks")
        started = run_cmd(start, timeout=RUN_CMD_TIMEOUT)
        run_id = started.stdout.strip()
        if not run_id or "\n" in run_id:
            fail(f"`anthrex run start` printed no single run id:\n{started.stdout}")
        h4 = run_id[-4:]

        proc = pty_proc([bin_path])
        proc.wait_for("agents", timeout=SCREEN_WAIT, label="stage-11t attach banner")
        # The run's node in the sidebar tree: filtered to the run's short id, the
        # project's node is selected, and `j` moves to the run.
        proc.send(b"\x02t")
        proc.wait_for(" TREE ", timeout=SCREEN_WAIT, label="the tree's navigation")
        proc.send(b"/")
        proc.wait_for(" FILTER ", timeout=SCREEN_WAIT, label="the tree's filter")
        proc.send(h4.encode())
        proc.send(b"\r")
        proc.wait_for(" TREE ", timeout=SCREEN_WAIT, label="the tree after filtering")
        proc.wait_for("Menu t", timeout=SCREEN_WAIT, label="the run's node in the tree")
        proc.send(b"j")
        proc.send(b".")
        proc.wait_for(" MENU ", timeout=SCREEN_WAIT, label="the action menu's badge")
        proc.wait_for(f"┌ Menu t · {h4} ", timeout=SCREEN_WAIT, label="the menu's title")
        proc.wait_for("approve", timeout=SCREEN_WAIT, label="the menu's approve entry")
        print(f"ok: `.` on run {run_id} opened its action menu with approve")
        proc.send(b"\x1b")
        _wait_gone(proc, " MENU ", "the action menu after its Esc", fail)

        proc.send(b"\x02S")
        proc.wait_for(" SETTINGS ", timeout=SCREEN_WAIT, label="the Settings screen's badge")
        proc.wait_for("claude-opus-5-5", timeout=SETTINGS_LOAD_WAIT, label="the Claude models")
        # Tab to the Codex table; `gpt-6-sol` is its second row
        # (`config::settings::SHIPPED_CODEX`), off in the built-in roster.
        proc.send(b"\t")
        proc.wait_for("[ ] gpt-6-sol", timeout=SCREEN_WAIT, label="the Codex table")
        proc.send(b"j")
        proc.send(b" ")
        proc.wait_for("[x] gpt-6-sol", timeout=SCREEN_WAIT, label="gpt-6-sol enabled")
        proc.send(b"w")
        proc.wait_for(SAVED, timeout=SETTINGS_SAVE_WAIT, label="the saved line")
        # The daemon answers `Saved` only after its rename, so the file is there now.
        try:
            with open(config_path, encoding="utf-8") as f:
                written = f.read()
        except FileNotFoundError:
            fail(f"the Settings save showed {SAVED!r} but {config_path} does not exist")
        if "builtin_models = false" not in written or '"gpt-6-sol"' not in written:
            fail(f"the saved {config_path} lacks the roster:\n{written}")
        print(f"ok: Settings saved gpt-6-sol through the daemon into {config_path}")
        proc.send(b"\x1b")
        _wait_gone(proc, " SETTINGS ", "the Settings screen after its Esc", fail)

        # The selected run's project is the screen's project.
        proc.send(b"\x02P")
        proc.wait_for(" PROFILE ", timeout=SCREEN_WAIT, label="the Profile screen's badge")
        proc.wait_for(
            f"profile · {os.path.basename(repo)}",
            timeout=SCREEN_WAIT,
            label="the Profile screen's project",
        )
        print("ok: C-b P opened the run's project's profile")
        proc.send(b"\x1b")
        _wait_gone(proc, " PROFILE ", "the Profile screen after its Esc", fail)

        # Out of the tree; the client must read this Esc on its own before the prefix
        # follows it (an Esc and a C-b in one read decode as Alt+C-b, stage 11e).
        proc.send(b"\x1b")
        _wait_gone(proc, " TREE ", "the tree after its Esc", fail)
        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=DETACH_WAIT)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"stage-11t detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None

        run_cmd(["run", "reject", run_id, "--confirm", run_id], timeout=RUN_CMD_TIMEOUT)
        # A rejected plan discards the run; its clean-up is one task path's git calls
        # at most (`RUN_WAIT`).
        deadline = time.monotonic() + RUN_WAIT
        while True:
            run = _run_state(run_cmd, run_id, fail)
            if run["state"] == "discarded":
                break
            if time.monotonic() >= deadline:
                fail(f"rejected run {run_id} was not discarded within {RUN_WAIT}s:\n{json.dumps(run, indent=2)}")
            time.sleep(POLL)
        print(f"ok: run {run_id} was rejected at its gate; the client detached cleanly")
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
        try:
            os.remove(plan)
        except FileNotFoundError:
            pass
        # The Settings save created `ANTHREX_CONFIG`; every later stage but 12b (which
        # writes and removes its own) needs it absent.
        shutil.rmtree(os.path.dirname(config_path), ignore_errors=True)
