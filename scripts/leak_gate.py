#!/usr/bin/env python3
"""The leak gate (milestone 9.12, decisions 19 to 21).

It reports what a test run left behind: anthrex daemons, fake-agent stand-ins and
temporary files or directories that were not there before the run. It only inspects;
it never stops, ends or touches any process or file of anyone's. The one exception is
`selftest`, which ends the child it started itself, by that child's pid.

    python3 scripts/leak_gate.py snapshot <file>
    python3 scripts/leak_gate.py check <file> [--wait <seconds>]
    python3 scripts/leak_gate.py selftest

`snapshot` writes the matching processes (pids) and temporary entries present now.
`check` waits until no matching process the snapshot did not hold remains, or
`--wait` seconds pass, then exits 1 when such a process or a new temporary entry
exists, printing each one; 0 when clean; 2 on a usage error or an unreadable snapshot.
"""
import json
import os
import subprocess
import sys

# Decision 19: a test daemon's stop is HANDSHAKE_TIMEOUT (5 s) + HUP_GRACE (1 s) +
# KILL_GRACE (3 s) + wait_released's cap (10 s) = 19 s (docs/timing-budgets.md, the
# `daemon stop` row), plus slack.
PROCESS_WAIT = 30
POLL = 0.5
# Decision 20 with "Spec and brief defects" 7's ruling: `ax ` (with a space) is the
# prefix of `daemon/tests/it/run_git_worktrees.rs`'s `ax run ...` and `ax wt ...`.
PREFIXES = ("ax-", "ax ", "anthrex-")
# Shared roots, not a test's own directory: their children are compared, never the
# roots themselves. `anthrex-worktrees` is the test default worktree root
# (`daemon/src/manager/config.rs`); `ax-<uid>` is the daemon's task-temporary root
# (`daemon/src/run/git/tmp.rs`); `anthrex-<uid>` is the default socket directory
# (`proto/src/paths.rs`), the live daemon's.
SHARED_ROOTS = ("anthrex-worktrees", f"ax-{os.getuid()}", f"anthrex-{os.getuid()}")


def usage(message):
    print(f"leak_gate: {message}", file=sys.stderr)
    print(__doc__.split("\n\n")[2], file=sys.stderr)
    return 2


def matching_processes():
    """Every live matching process as {pid: (ppid, args)}.

    A match is argv[0]'s basename `anthrex` with argv[1] `daemon`, or argv[0]'s
    basename `fake-agent`. Zombies are not counted: there is no process left to leak,
    and its parent is what would be. `ps axww` is the BSD form macOS and procps share.
    """
    out = subprocess.run(
        ["ps", "axww", "-o", "pid=,ppid=,stat=,args="],
        stdout=subprocess.PIPE,
        text=True,
        check=True,
    ).stdout
    found = {}
    for line in out.splitlines():
        fields = line.split(None, 3)
        if len(fields) < 4:
            continue
        pid, ppid, stat, args = fields
        if "Z" in stat:
            continue
        argv = args.split()
        name = os.path.basename(argv[0])
        if (name == "anthrex" and len(argv) > 1 and argv[1] == "daemon") or name == "fake-agent":
            found[int(pid)] = (int(ppid), args)
    return found


def temp_dirs():
    """`/tmp` and the system temporary directory, each once, resolved."""
    dirs = []
    for d in ("/tmp", system_temp()):
        real = os.path.realpath(d)
        if real not in dirs and os.path.isdir(real):
            dirs.append(real)
    return dirs


def listdir(path):
    try:
        return sorted(os.listdir(path))
    except OSError:
        return []


def temporary_entries():
    """Every matching file or directory directly in a temporary directory; for a
    shared root, its children instead of the root itself."""
    entries = set()
    for d in temp_dirs():
        for name in listdir(d):
            if not name.startswith(PREFIXES):
                continue
            path = os.path.join(d, name)
            if name in SHARED_ROOTS:
                if os.path.isdir(path) and not os.path.islink(path):
                    entries.update(os.path.join(path, child) for child in listdir(path))
            else:
                entries.add(path)
    return entries


def snapshot(path):
    data = {"pids": sorted(matching_processes()), "entries": sorted(temporary_entries())}
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=1)
    return 0


def check(path, wait):
    import time

    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        known_pids = set(data["pids"])
        known_entries = set(data["entries"])
    except (OSError, ValueError, KeyError, TypeError) as err:
        print(f"leak_gate: cannot read the snapshot {path}: {err}", file=sys.stderr)
        return 2

    # A daemon a test stopped may still be on its way out; wait for it, never end it.
    deadline = time.monotonic() + wait
    while True:
        new = {p: v for p, v in matching_processes().items() if p not in known_pids}
        if not new or time.monotonic() >= deadline:
            break
        time.sleep(POLL)
    entries = sorted(temporary_entries() - known_entries)

    for pid in sorted(new):
        ppid, args = new[pid]
        print(f"leaked process: pid {pid} ppid {ppid}: {args}")
    for entry in entries:
        print(f"leaked temporary entry: {entry}")
    if new or entries:
        print(f"leak gate: {len(new)} leaked processes, {len(entries)} leaked temporary entries")
        return 1
    print("leak gate: clean")
    return 0


def run_self(*args):
    """Run this script as a child and return (exit status, combined output)."""
    done = subprocess.run(
        [sys.executable, os.path.abspath(__file__), *args],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=False,
    )
    return done.returncode, done.stdout


def system_temp():
    return os.environ.get("TMPDIR") or "/tmp"


def selftest():
    """Prove both checks bite and that what the snapshot held is never reported."""
    import shutil
    import tempfile

    failures = []
    temp = system_temp()
    # A match present before the snapshot stands in for the user's live daemon.
    before_dir = os.path.join(temp, f"ax-leakgate-selftest-before-{os.getpid()}")
    os.makedirs(before_dir, exist_ok=True)
    before = subprocess.Popen(["fake-agent", "60"], executable="/bin/sleep")
    leaked_dir = os.path.join(temp, f"ax-leakgate-selftest-{os.getpid()}")
    child = None
    fd, snap = tempfile.mkstemp(suffix=".json")
    os.close(fd)
    try:
        status, out = run_self("snapshot", snap)
        if status != 0:
            failures.append(f"snapshot exited {status}:\n{out}")
        os.makedirs(leaked_dir, exist_ok=True)
        child = subprocess.Popen(["fake-agent", "60"], executable="/bin/sleep")

        status, out = run_self("check", snap, "--wait", "1")
        print(out, end="")
        if status != 1:
            failures.append(f"check with a leak exited {status}, not 1")
        if f"leaked process: pid {child.pid} " not in out:
            failures.append(f"check did not name the leaked process {child.pid}")
        if os.path.realpath(leaked_dir) not in out:
            failures.append(f"check did not name the leaked directory {leaked_dir}")
        if f"pid {before.pid} " in out:
            failures.append(f"check named the pre-existing process {before.pid}")
        if os.path.realpath(before_dir) in out:
            failures.append(f"check named the pre-existing directory {before_dir}")

        child.terminate()  # the only signal this script sends: its own child, by its pid
        child.wait()
        child = None
        os.rmdir(leaked_dir)

        status, out = run_self("check", snap)
        print(out, end="")
        if status != 0:
            failures.append(f"check after the clean-up exited {status}, not 0")
    finally:
        if child is not None:
            child.terminate()  # the only signal: its own child, by its pid
            child.wait()
        before.terminate()  # the only signal: its own child, by its pid
        before.wait()
        shutil.rmtree(leaked_dir, ignore_errors=True)
        shutil.rmtree(before_dir, ignore_errors=True)
        os.unlink(snap)

    for failure in failures:
        print(f"leak gate selftest: {failure}")
    print("leak gate selftest: " + ("FAILED" if failures else "ok"))
    return 1 if failures else 0


def main(argv):
    if len(argv) < 2:
        return usage("a mode is required")
    mode, rest = argv[1], argv[2:]
    if mode == "snapshot" and len(rest) == 1:
        return snapshot(rest[0])
    if mode == "check" and len(rest) in (1, 3):
        wait = PROCESS_WAIT
        if len(rest) == 3:
            if rest[1] != "--wait":
                return usage(f"unknown option {rest[1]}")
            try:
                wait = float(rest[2])
            except ValueError:
                return usage(f"--wait needs a number of seconds, not {rest[2]}")
        return check(rest[0], wait)
    if mode == "selftest" and not rest:
        return selftest()
    return usage(f"bad arguments: {' '.join(argv[1:])}")


if __name__ == "__main__":
    sys.exit(main(sys.argv))
