"""Milestone 9.12 task M9.12.13: a smoke stage's teardown of the daemon's task
temporary directories.

The daemon names each task's temporary directory `<root>/<16 hex digits>`, the root
`/private/tmp/ax-<uid>` on macOS (`/tmp/ax-<uid>` elsewhere) and the leaf hashed from
the checkout repository's path (`crates/daemon/src/run/git/tmp.rs`). It removes one
only with its checkout, so a stage that removes its data directory with runs still in
it would leave theirs behind. `remove_task_tmps_beneath(base)` mirrors the daemon's
`remove_task_tmps_beneath`: it removes the directory of every directory under `base`
(and of the worktree checkouts' repository directories, which are never made), under
each spelling of `base`, and nothing else.

`PINNED` is a path and its name. The Rust test `the_smokes_pinned_name_is_this_hash`
(`tmp.rs`) reads this file's `PINNED` line and asserts the name is Rust's own hash of
the path; `check_pinned`, at the smoke's start, asserts it is this module's hash. A
change to either hash that the other does not follow fails one of the two.

It only removes directories whose names are hashed from paths under `base`, which only
the stage's own daemon makes. It never signals anything.
"""
import os
import shutil
import stat
import sys

# `tmp.rs`'s WORKTREE_CHECKOUTS.
WORKTREE_CHECKOUTS = ("integration", ".full")
# Read by `tmp.rs`'s `the_smokes_pinned_name_is_this_hash`: keep it on one line.
PINNED = ("/x/runs/r/tasks/t", "ebb460a1810f194c")

_OFFSET = 0xCBF29CE484222325
_PRIME = 0x100000001B3
_MASK = (1 << 64) - 1


def tmp_root():
    base = "/private/tmp" if sys.platform == "darwin" else "/tmp"
    return f"{base}/ax-{os.getuid()}"


def _components(path):
    """Rust's `Path::components` of an absolute path: the root, then each part, with
    empty and `.` parts dropped and `..` kept."""
    if not path.startswith("/"):
        raise ValueError(f"{path} is not absolute")
    return ["/"] + [part for part in path.split("/") if part not in ("", ".")]


def task_name(repo_dir):
    """`task_tmp(repo_dir)`'s file name: FNV-1a over each component's bytes, each
    component closed with 0xff."""
    value = _OFFSET
    for component in _components(repo_dir):
        for byte in os.fsencode(component):
            value = ((value ^ byte) * _PRIME) & _MASK
        value = ((value ^ 0xFF) * _PRIME) & _MASK
    return f"{value:016x}"


def task_tmp(repo_dir):
    return os.path.join(tmp_root(), task_name(repo_dir))


def check_pinned():
    """The pinned name, or an AssertionError naming the drift."""
    path, name = PINNED
    got = task_name(path)
    assert got == name, f"smoke_task_tmp.py hashes {path} to {got}, not the pinned {name}"


def _owner_access(top):
    """Gives the owner access to every directory under `top`, following no link, as
    the daemon's `restore_owner_access` does before it removes one."""
    for dirpath, dirnames, _ in os.walk(top):
        for name in [dirpath] + [os.path.join(dirpath, d) for d in dirnames]:
            try:
                mode = os.lstat(name).st_mode
            except OSError:
                continue
            if stat.S_ISDIR(mode):
                os.chmod(name, stat.S_IMODE(mode) | stat.S_IRWXU)


def _remove(repo_dir):
    tmp = task_tmp(repo_dir)
    try:
        mode = os.lstat(tmp).st_mode
    except FileNotFoundError:
        return False
    if stat.S_ISDIR(mode):
        _owner_access(tmp)
        shutil.rmtree(tmp)
    else:
        os.unlink(tmp)
    return True


def remove_task_tmps_beneath(base):
    """Removes the task temporary directory of every directory under `base`; the
    number removed."""
    spellings = {base}
    if os.path.isdir(base):
        resolved = os.path.realpath(base)
        spellings.add(resolved)
        if resolved.startswith("/private/"):
            spellings.add(resolved[len("/private"):])
    removed = 0
    for spelling in sorted(spellings):
        for dirpath, _, _ in os.walk(spelling):
            worktrees = [os.path.join(dirpath, "tasks", n) for n in WORKTREE_CHECKOUTS]
            for repo_dir in [dirpath] + worktrees:
                if _remove(repo_dir):
                    removed += 1
    return removed
