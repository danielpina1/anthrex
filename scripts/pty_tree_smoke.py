"""Project-tree stage for the PTY smoke test."""

import json
import os
import re
import shutil
import subprocess
import tempfile
import time

# This module's own stage functions compare rendered screens, so a git segment in the
# bottom bar (driven by the state of whatever working tree the daemon happens to run
# in) would make those comparisons depend on the runner's git state rather than the
# fixture git repositories the stages build for themselves. `pty-smoke.py` sets this
# for the daemon it starts and passes it through its own `ENV`; this covers the case
# where a stage function in this module is ever driven by a caller that instead
# inherits `os.environ` for the daemon it starts.
os.environ["ANTHREX_GIT"] = "off"


GIT_CONFIG = [
    "-c",
    "user.name=t",
    "-c",
    "user.email=t@t",
    "-c",
    "commit.gpgsign=false",
    "-c",
    "init.defaultBranch=main",
    "-c",
    "core.hooksPath=/dev/null",
]


def _git(args, cwd, fail):
    env = dict(os.environ)
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    env["GIT_TERMINAL_PROMPT"] = "0"
    command = ["git", *GIT_CONFIG, *args]
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=15,
        )
    except subprocess.TimeoutExpired as error:
        fail(
            f"git command timed out after 15s: {command!r}\n"
            f"stdout: {error.stdout!r}\nstderr: {error.stderr!r}"
        )
    if result.returncode != 0:
        fail(
            f"git command failed: {command!r}\nstatus: {result.returncode}\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )


def _window_id(created, name, fail):
    try:
        return int(created.stdout.strip())
    except ValueError:
        fail(f"`anthrex new` did not print a window id for {name}: {created.stdout!r}")


def _guide_stem(screen, subagent_label):
    """The two characters right before the sidebar's connector on the row whose label
    is `explore: conn-<subagent_label>` — the ancestor stem for the window level: "  "
    when the window is the last of its project, "│ " otherwise. `None` if that row
    is not (yet) on screen in the expected shape (guides, connector, one-cell status
    glyph, a space, then the label), which the caller treats as "not settled yet"
    rather than a distinct failure.
    """
    match = re.search(rf"(..)(?:├─|└─). explore: conn-{subagent_label}\b", screen)
    return match.group(1) if match else None


def _tree_json(run_cmd, fail):
    result = run_cmd(["tree", "--json"])
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        fail(f"`anthrex tree --json` returned invalid JSON ({error}):\n{result.stdout}")


def run_project_tree_stage(repo_root, pty_proc, run_cmd, fail):
    """Exercise linked-worktree grouping and the tree UI, then restore the baseline."""
    print("== stage 8c: project tree ==")
    fixture = tempfile.mkdtemp(prefix="anthrex-tree-", dir="/tmp")
    repo = os.path.join(fixture, "repo")
    worktree = os.path.join(fixture, "wt")
    repo_sub = os.path.join(repo, "sub")
    pending_removal = []
    proc = None
    passed = False
    try:
        _git(["init", repo], repo_root, fail)
        _git(["-C", repo, "commit", "--allow-empty", "-m", "init"], repo_root, fail)
        os.mkdir(repo_sub)
        _git(["-C", repo, "worktree", "add", "-b", "feature", worktree], repo_root, fail)
        canonical_repo = os.path.realpath(repo)
        canonical_worktree = os.path.realpath(worktree)

        created_a = run_cmd(
            ["new", "--runtime", "shell", "--name", "tree-a", "--dir", repo_sub]
        )
        pending_removal.append("tree-a")
        _window_id(created_a, "tree-a", fail)
        created_b = run_cmd(
            ["new", "--runtime", "shell", "--name", "tree-b", "--dir", canonical_worktree]
        )
        pending_removal.append("tree-b")
        _window_id(created_b, "tree-b", fail)

        projects = _tree_json(run_cmd, fail).get("projects")
        if not isinstance(projects, list):
            fail(f"`anthrex tree --json` omitted the projects array: {projects!r}")
        matches = [project for project in projects if project.get("root") == canonical_repo]
        if len(matches) != 1:
            fail(f"expected exactly one project rooted at {canonical_repo!r}: {projects!r}")
        names = {window.get("name") for window in matches[0].get("windows", [])}
        if not {"tree-a", "tree-b"}.issubset(names):
            fail(f"linked worktree windows were not grouped under {canonical_repo!r}: {matches[0]!r}")

        proc = pty_proc([os.path.join(repo_root, "target/debug/anthrex")])
        proc.wait_for("agents", timeout=10.0, label="project-tree attach banner")
        proc.send(b"\x02t")
        proc.wait_for(" TREE ", timeout=10.0, label="tree navigation mode")
        proc.send(b"/tree-b")
        proc.wait_for(" FILTER ", timeout=10.0, label="tree filter mode")
        proc.wait_for("tree-b", timeout=10.0, label="tree-b filter result")
        proc.send(b"\r")
        proc.wait_for(" TREE ", timeout=10.0, label="tree navigation after retaining filter")
        proc.send(b"\r")
        proc.wait_for("tree-b · shell", timeout=10.0, label="tree-b focused title")
        proc.send(b"\x02T")
        proc.wait_for(" tree overview ", timeout=10.0, label="tree overview")
        proc.wait_for(canonical_repo, timeout=10.0, label="canonical repository in overview")
        proc.send(b"\x1b")
        proc.wait_for("tree-b · shell", timeout=10.0, label="overview dismissed")
        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=5.0)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"project-tree detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None

        for name in list(pending_removal):
            run_cmd(["rm", name])
            pending_removal.remove(name)
        remaining = json.loads(run_cmd(["ls", "--json"]).stdout)
        remaining_names = {window["name"] for window in remaining}
        expected = {"shell-1", "shell-2", "shell-3", "shell-4"}
        if remaining_names != expected:
            fail(f"unexpected windows after project-tree cleanup: {sorted(remaining_names)}")
        passed = True
    finally:
        try:
            for name in pending_removal:
                try:
                    run_cmd(["rm", name], expect_ok=False)
                except Exception:
                    pass
        finally:
            if proc is not None:
                proc.close()
            shutil.rmtree(fixture, ignore_errors=True)

    if passed:
        print("ok: project tree groups a linked worktree, filters and focuses it, and opens the overview")


def _write_subagent_script(path):
    """Overwrites the fake-agent script with one that spawns and finishes two
    root sub-agents, then finishes its own turn.

    Every `--runtime claude` window created after this call replays these steps, so
    this is written just before creating the one window this stage needs it for. Both
    sub-agents and the window itself are left in a finished, non-spinning state before
    the trailing long wait: a status glyph that keeps animating (the working spinner)
    keeps every row it shares a frame with redrawing too, which is needless noise for a
    stage that only cares about the guides, not the status.
    """
    steps = [
        {"hook": "SessionStart", "payload": {}},
        {"hook": "UserPromptSubmit", "payload": {}},
        {
            "hook": "PreToolUse",
            "payload": {
                "tool_name": "Agent",
                "tool_input": {"subagent_type": "explore", "name": "conn-alpha"},
            },
        },
        {"hook": "SubagentStart", "payload": {"agent_id": "conn-sub-alpha", "agent_type": "explore"}},
        {
            "hook": "PreToolUse",
            "payload": {
                "tool_name": "Agent",
                "tool_input": {"subagent_type": "explore", "name": "conn-beta"},
            },
        },
        {"hook": "SubagentStart", "payload": {"agent_id": "conn-sub-beta", "agent_type": "explore"}},
        {"hook": "SubagentStop", "payload": {"agent_id": "conn-sub-alpha"}},
        {"hook": "SubagentStop", "payload": {"agent_id": "conn-sub-beta"}},
        {"hook": "Stop", "payload": {}},
        {"wait_ms": 60000},
    ]
    with open(path, "w", encoding="utf-8") as script:
        for step in steps:
            script.write(json.dumps(step) + "\n")


def run_tree_connectors_stage(repo_root, pty_proc, run_cmd, fail, fake_agent_script):
    """Two projects, the last window of one with two sub-agents, guides checked in the overview."""
    print("== stage 9: tree connectors ==")
    fixture = tempfile.mkdtemp(prefix="anthrex-connectors-", dir="/tmp")
    project_a = os.path.join(fixture, "a")
    project_b = os.path.join(fixture, "b")
    pending_removal = []
    proc = None
    passed = False
    try:
        _git(["init", project_a], repo_root, fail)
        _git(["-C", project_a, "commit", "--allow-empty", "-m", "init"], repo_root, fail)
        _git(["init", project_b], repo_root, fail)
        _git(["-C", project_b, "commit", "--allow-empty", "-m", "init"], repo_root, fail)

        # First window of project A: not the last, so its row (and any sub-agents it
        # had) would begin "├─" / "│ ". It has no sub-agents; it exists only to make
        # conn-a2 provably *not* the project's only window.
        created = run_cmd(["new", "--runtime", "shell", "--name", "conn-a1", "--dir", project_a])
        pending_removal.append("conn-a1")
        _window_id(created, "conn-a1", fail)

        # Last window of project A: its sub-agent rows must begin with two spaces, not
        # "│ ", because it has no later sibling.
        _write_subagent_script(fake_agent_script)
        created = run_cmd(["new", "--runtime", "claude", "--name", "conn-a2", "--dir", project_a])
        pending_removal.append("conn-a2")
        _window_id(created, "conn-a2", fail)

        # A second project, so the tree has more than one root.
        created = run_cmd(["new", "--runtime", "shell", "--name", "conn-b1", "--dir", project_b])
        pending_removal.append("conn-b1")
        _window_id(created, "conn-b1", fail)

        proc = pty_proc([os.path.join(repo_root, "target/debug/anthrex")])
        proc.wait_for("agents", timeout=10.0, label="tree-connectors attach banner")
        proc.wait_for("conn-alpha", timeout=10.0, label="conn-alpha sub-agent row")
        proc.wait_for("conn-beta", timeout=10.0, label="conn-beta sub-agent row")
        proc.send(b"\x02T")
        proc.wait_for(" tree overview ", timeout=10.0, label="tree overview for connectors")

        # The sidebar keeps redrawing (spinners, the newly-created windows settling
        # into their final rows) for a moment after "conn-alpha"/"conn-beta" first
        # appear, and ratatui only repaints the cells that changed between frames — so
        # a screen read caught mid-redraw can show a stale character at a cell a later
        # frame never revisits. Poll until the exact shape holds rather than reading
        # once, the same way `wait_for` above tolerates the same kind of settling.
        deadline = time.monotonic() + 5.0
        screen = proc.screen_text()
        while True:
            alpha_stem = _guide_stem(screen, "alpha")
            beta_stem = _guide_stem(screen, "beta")
            settled = "├─" in screen and "└─" in screen and alpha_stem == "  " and beta_stem == "  "
            if settled:
                break
            if time.monotonic() >= deadline:
                fail(
                    "tree overview never settled with ├─/└─ connectors present and "
                    "conn-alpha/conn-beta preceded by two spaces, not │ "
                    f"(alpha stem={alpha_stem!r}, beta stem={beta_stem!r}):\n{screen}"
                )
            proc.read_available(timeout=0.2)
            screen = proc.screen_text()

        # The prefix key is handled ahead of tree/overview input routing (see
        # `Keymap::handle` in `crates/tui/src/keymap.rs`), so detach works directly
        # from the overview without first pressing Escape to leave it.
        proc.send(b"\x02d")
        status = proc.wait_exit(timeout=5.0)
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            fail(f"tree-connectors detach did not exit cleanly with status 0 (raw status {status})")
        proc.close()
        proc = None

        for name in list(pending_removal):
            run_cmd(["rm", name])
            pending_removal.remove(name)
        remaining = json.loads(run_cmd(["ls", "--json"]).stdout)
        remaining_names = {window["name"] for window in remaining}
        expected = {"shell-1", "shell-2", "shell-3", "shell-4"}
        if remaining_names != expected:
            fail(f"unexpected windows after tree-connectors cleanup: {sorted(remaining_names)}")
        passed = True
    finally:
        try:
            for name in pending_removal:
                try:
                    run_cmd(["rm", name], expect_ok=False)
                except Exception:
                    pass
        finally:
            if proc is not None:
                proc.close()
            shutil.rmtree(fixture, ignore_errors=True)

    if passed:
        print(
            "ok: tree overview draws ├─/└─ connectors and stems the last window's "
            "sub-agents with two spaces, not │"
        )
