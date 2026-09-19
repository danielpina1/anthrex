"""Project-tree stage for the PTY smoke test."""

import json
import os
import shutil
import subprocess
import tempfile


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
