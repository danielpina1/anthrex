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
With `AX_TMP_TRACE` set, `check` also names the creator of each leaked `ax-<uid>/<hash>`.
`check` waits until no matching process the snapshot did not hold remains, or
`--wait` seconds pass, then exits 1 when such a process or a new temporary entry
exists, printing each one; 0 when clean; 2 on a usage error or an unreadable snapshot.
"""
import json
import os
import subprocess
import sys

# Decision 19: a test daemon's stop is HANDSHAKE_TIMEOUT (5 s) + HUP_GRACE (1 s) +
# KILL_GRACE (3 s) + wait_released's cap (10 s) + the exit wait (5 s) = 24 s
# (docs/timing-budgets.md, the `daemon stop` row), plus slack.
PROCESS_WAIT = 30
POLL = 0.5
# Decision 20 with "Spec and brief defects" 7's ruling: `ax ` (with a space) is the
# prefix of `daemon/tests/it/run_git_worktrees.rs`'s `ax run ...` and `ax wt ...`.
PREFIXES = ("ax-", "ax ", "anthrex-")
# Shared roots, not a test's own directory: their children are compared, never the
# roots themselves. `anthrex-worktrees` is `ManagerConfig::new`'s default worktree
# root (`daemon/src/manager/config.rs`; since M9.14.11 `for_tests` names a sentinel
# that panics instead, so a test reaching it is a leak this gate still sees); `ax-<uid>` is the daemon's task-temporary root
# (`daemon/src/run/git/tmp.rs`); `anthrex-<uid>` is the default socket directory
# (`proto/src/paths.rs`), the live daemon's.
SHARED_ROOTS = ("anthrex-worktrees", f"ax-{os.getuid()}", f"anthrex-{os.getuid()}")
# The ax-leak-2 fix: a task temporary directory's name is a hash of its repository's
# path and says nothing about who made it. With `AX_TMP_TRACE` naming a file, the
# daemon's code appends one line to it per task temporary directory it creates
# (`crates/daemon/src/run/git/tmp_trace.rs`, whose test pins these field names), and
# `check` prints the creator of each leaked `ax-<uid>/<hash>`.
TRACE_ENV = "AX_TMP_TRACE"
TRACE_FIELDS = ("created", "hash", "repo", "exe", "test", "pid")


def usage(message):
    print(f"leak_gate: {message}", file=sys.stderr)
    print(__doc__.split("\n\n")[2], file=sys.stderr)
    return 2


def matching_processes():
    """Every live matching process as {pid: (ppid, args)}.

    A match is argv[0]'s basename `anthrex` with argv[1] `daemon`, or argv[0]'s
    basename `fake-agent`. Zombies are not counted: there is no process left to leak,
    and its parent is what would be. `ps axww` is the BSD form macOS and procps share.
    argv[0] may hold spaces (`split_program`).
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
        if matches(args):
            found[int(pid)] = (int(ppid), args)
    return found


def matches(args):
    """Whether `ps`'s args for one process are an anthrex daemon or a fake-agent."""
    program, rest = split_program(args)
    name = os.path.basename(program)
    return (name == "anthrex" and rest.split()[:1] == ["daemon"]) or name == "fake-agent"


def split_program(args):
    """`ps`'s space-joined args as (argv[0], the rest).

    `ps` joins argv with spaces, so argv[0] is not always its first word: a binary under
    a directory whose name has a space (`/Users/me/My Repos/target/debug/anthrex daemon`)
    spans several. argv[0] is the shortest leading run of words whose basename is one
    this gate matches and which is that bare name (no `/`) or an existing file;
    otherwise, the first word. The file check is what keeps a directory named after a
    match (`/Users/me/anthrex copy/target/debug/anthrex daemon`) from ending the run at
    `/Users/me/anthrex`.
    """
    words = args.split(" ")
    for k in range(1, len(words) + 1):
        program = " ".join(words[:k])
        if os.path.basename(program) not in ("anthrex", "fake-agent"):
            continue
        if "/" not in program or os.path.isfile(program):
            return program, " ".join(words[k:])
    first, _, rest = args.partition(" ")
    return first, rest


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


def trace_creators(path):
    """{hash: [record]} from the trace file at `path`, each record a dict of
    `TRACE_FIELDS`; {} when there is no file. A line of any other shape is skipped."""
    creators = {}
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                fields = line.rstrip("\n").split("\t")
                if len(fields) != len(TRACE_FIELDS) or fields[0] != "created":
                    continue
                record = dict(zip(TRACE_FIELDS, fields))
                creators.setdefault(record["hash"], []).append(record)
    except OSError:
        return {}
    return creators


def creator_lines(entry, creators):
    """The report lines naming who made `entry`, a leaked child of the task temporary
    root `ax-<uid>`; none for any other entry."""
    if os.path.basename(os.path.dirname(entry)) != f"ax-{os.getuid()}":
        return []
    records = creators.get(os.path.basename(entry), [])
    if not records:
        return ["  created by: not in the trace"]
    return [
        f"  created by: test {r['test']} ({r['exe']}, pid {r['pid']}), repository {r['repo']}"
        for r in records
    ]


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
    trace = os.environ.get(TRACE_ENV)
    creators = trace_creators(trace) if trace else {}
    for entry in entries:
        print(f"leaked temporary entry: {entry}")
        if trace:
            for line in creator_lines(entry, creators):
                print(line)
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
    failures.extend(selftest_trace())
    temp = system_temp()
    # A match present before the snapshot stands in for the user's live daemon.
    before_dir = os.path.join(temp, f"ax-leakgate-selftest-before-{os.getpid()}")
    os.makedirs(before_dir, exist_ok=True)
    before = subprocess.Popen(["fake-agent", "60"], executable="/bin/sleep")
    leaked_dir = os.path.join(temp, f"ax-leakgate-selftest-{os.getpid()}")
    # A stand-in under a directory whose name has a space: `ps` shows its argv[0] as
    # several words, and the gate must still match it.
    spaced_dir = os.path.join(temp, f"ax-leakgate selftest spaced-{os.getpid()}")
    os.makedirs(spaced_dir, exist_ok=True)
    spaced_program = os.path.join(spaced_dir, "fake-agent")
    with open(spaced_program, "w"):
        pass
    # A stand-in under a directory literally named `anthrex copy`, whose parent path has
    # no space: the word `.../anthrex` is a directory, not argv[0], and must not end the
    # match early.
    copy_bin = os.path.join(before_dir, "anthrex copy", "target", "debug")
    os.makedirs(copy_bin, exist_ok=True)
    copy_program = os.path.join(copy_bin, "fake-agent")
    copy_daemon = os.path.join(copy_bin, "anthrex")
    for path in (copy_program, copy_daemon):
        with open(path, "w"):
            pass
    # What `ps` shows, matched or not; an unrelated process is never matched.
    for args, want in (
        (f"{copy_program} 60", True),
        (f"{copy_daemon} daemon --foreground", True),
        (f"{spaced_program} 60", True),
        ("fake-agent 60", True),
        ("/tmp/x/anthrex daemon", True),
        (f"{copy_daemon} attach", False),
        ("vim /tmp/x/fake-agent", False),
        (f"vim {copy_program}", False),
        ("/bin/sh -c /tmp/anthrex daemon", False),
        (f"/bin/sh -c {copy_daemon} daemon", False),
        ("cargo run -p anthrex -- daemon", False),
    ):
        if matches(args) != want:
            failures.append(f"matches({args!r}) is {not want}, not {want}")
    child = None
    spaced = None
    copied = None
    fd, snap = tempfile.mkstemp(suffix=".json")
    os.close(fd)
    try:
        status, out = run_self("snapshot", snap)
        if status != 0:
            failures.append(f"snapshot exited {status}:\n{out}")
        os.makedirs(leaked_dir, exist_ok=True)
        child = subprocess.Popen(["fake-agent", "60"], executable="/bin/sleep")
        spaced = subprocess.Popen([spaced_program, "60"], executable="/bin/sleep")
        copied = subprocess.Popen([copy_program, "60"], executable="/bin/sleep")

        status, out = run_self("check", snap, "--wait", "1")
        print(out, end="")
        if status != 1:
            failures.append(f"check with a leak exited {status}, not 1")
        if f"leaked process: pid {child.pid} " not in out:
            failures.append(f"check did not name the leaked process {child.pid}")
        if f"leaked process: pid {spaced.pid} " not in out:
            failures.append(f"check did not name the leaked process {spaced.pid}, whose path has a space")
        if f"leaked process: pid {copied.pid} " not in out:
            failures.append(f"check did not name the leaked process {copied.pid}, under `anthrex copy`")
        # Whole report lines, not substrings: a leaked process's command line can hold a
        # directory's path (the `anthrex copy` stand-in lives under `before_dir`).
        lines = out.splitlines()
        if f"leaked temporary entry: {os.path.realpath(leaked_dir)}" not in lines:
            failures.append(f"check did not name the leaked directory {leaked_dir}")
        if f"pid {before.pid} " in out:
            failures.append(f"check named the pre-existing process {before.pid}")
        if f"leaked temporary entry: {os.path.realpath(before_dir)}" in lines:
            failures.append(f"check named the pre-existing directory {before_dir}")

        child.terminate()  # the only signal this script sends: its own child, by its pid
        child.wait()
        child = None
        spaced.terminate()  # its own child, by its pid
        spaced.wait()
        spaced = None
        copied.terminate()  # its own child, by its pid
        copied.wait()
        copied = None
        os.rmdir(leaked_dir)

        status, out = run_self("check", snap)
        print(out, end="")
        if status != 0:
            failures.append(f"check after the clean-up exited {status}, not 0")
    finally:
        if child is not None:
            child.terminate()  # the only signal: its own child, by its pid
            child.wait()
        if spaced is not None:
            spaced.terminate()  # its own child, by its pid
            spaced.wait()
        if copied is not None:
            copied.terminate()  # its own child, by its pid
            copied.wait()
        before.terminate()  # the only signal: its own child, by its pid
        before.wait()
        shutil.rmtree(leaked_dir, ignore_errors=True)
        shutil.rmtree(before_dir, ignore_errors=True)
        shutil.rmtree(spaced_dir, ignore_errors=True)
        os.unlink(snap)

    for failure in failures:
        print(f"leak gate selftest: {failure}")
    print("leak gate selftest: " + ("FAILED" if failures else "ok"))
    return 1 if failures else 0


def selftest_trace():
    """The creator lookup: a leaked task temporary directory is named with every line
    the trace holds for its hash, an unknown one says so, and nothing else is looked
    up. Only a trace file of its own is written; the shared root is never touched."""
    import tempfile

    failures = []
    root = os.path.join("/tmp", f"ax-{os.getuid()}")
    fd, trace = tempfile.mkstemp(suffix=".tsv")
    with os.fdopen(fd, "w") as f:
        f.write("created\t00000000000000aa\t/d/runs/r1/tasks/t1\tanthrex\tit::one\t7\n")
        f.write("a line of another shape\n")
        f.write("created\t00000000000000aa\t/d/runs/r1/tasks/t1\tit-1\tit::two\t8\n")
    try:
        creators = trace_creators(trace)
        got = creator_lines(os.path.join(root, "00000000000000aa"), creators)
        want = [
            "  created by: test it::one (anthrex, pid 7), repository /d/runs/r1/tasks/t1",
            "  created by: test it::two (it-1, pid 8), repository /d/runs/r1/tasks/t1",
        ]
        if got != want:
            failures.append(f"creator_lines of a traced hash is {got}, not {want}")
        got = creator_lines(os.path.join(root, "00000000000000bb"), creators)
        if got != ["  created by: not in the trace"]:
            failures.append(f"creator_lines of an untraced hash is {got}")
        if creator_lines("/tmp/ax-run123456", creators):
            failures.append("creator_lines looked up an entry outside ax-<uid>")
        if trace_creators(trace + ".missing") != {}:
            failures.append("a missing trace file is not an empty trace")
    finally:
        os.unlink(trace)
    return failures


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
