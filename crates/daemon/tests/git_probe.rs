use daemon::git::probe::{PROBE_TIMEOUT, probe};
use proto::{GitOperation, Head};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};

fn git(dir: &Path, args: &[&OsStr]) {
    let status = git_command(dir, args).status().unwrap();
    assert!(status.success(), "git {args:?} failed with {status}");
}

/// Runs a git command without asserting success, for commands this suite expects to
/// exit non-zero — a conflicting merge or rebase.
fn git_allow_failure(dir: &Path, args: &[&OsStr]) {
    git_command(dir, args).status().unwrap();
}

fn git_command(dir: &Path, args: &[&OsStr]) -> Command {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

fn init_repo() -> TempDir {
    let dir = tempdir().unwrap();
    git(dir.path(), &[OsStr::new("init")]);
    git(
        dir.path(),
        &[
            OsStr::new("commit"),
            OsStr::new("--allow-empty"),
            OsStr::new("-m"),
            OsStr::new("init"),
        ],
    );
    dir
}

fn set_executable(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let script = dir.join(name);
    fs::write(&script, body).unwrap();
    set_executable(&script);
    script
}

/// A repository with one conflicting file on `main` and `feature`, mid-`git merge
/// feature` on `main`: `.git/MERGE_HEAD` is present and `conflict.txt` is unmerged.
fn merge_conflict_repo() -> TempDir {
    let repo = init_repo();
    let conflict = repo.path().join("conflict.txt");
    fs::write(&conflict, "base\n").unwrap();
    git(
        repo.path(),
        &[OsStr::new("add"), OsStr::new("conflict.txt")],
    );
    git(
        repo.path(),
        &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("base")],
    );
    git(
        repo.path(),
        &[
            OsStr::new("checkout"),
            OsStr::new("-b"),
            OsStr::new("feature"),
        ],
    );
    fs::write(&conflict, "feature change\n").unwrap();
    git(
        repo.path(),
        &[
            OsStr::new("commit"),
            OsStr::new("-am"),
            OsStr::new("feature change"),
        ],
    );
    git(repo.path(), &[OsStr::new("checkout"), OsStr::new("main")]);
    fs::write(&conflict, "main change\n").unwrap();
    git(
        repo.path(),
        &[
            OsStr::new("commit"),
            OsStr::new("-am"),
            OsStr::new("main change"),
        ],
    );
    git_allow_failure(repo.path(), &[OsStr::new("merge"), OsStr::new("feature")]);
    repo
}

/// A repository with one conflicting file on `main` and `feature`, mid-`git rebase
/// main` on `feature`.
fn rebase_conflict_repo() -> TempDir {
    let repo = init_repo();
    let conflict = repo.path().join("conflict.txt");
    fs::write(&conflict, "base\n").unwrap();
    git(
        repo.path(),
        &[OsStr::new("add"), OsStr::new("conflict.txt")],
    );
    git(
        repo.path(),
        &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("base")],
    );
    git(
        repo.path(),
        &[
            OsStr::new("checkout"),
            OsStr::new("-b"),
            OsStr::new("feature"),
        ],
    );
    fs::write(&conflict, "feature change\n").unwrap();
    git(
        repo.path(),
        &[
            OsStr::new("commit"),
            OsStr::new("-am"),
            OsStr::new("feature change"),
        ],
    );
    git(repo.path(), &[OsStr::new("checkout"), OsStr::new("main")]);
    fs::write(&conflict, "main change\n").unwrap();
    git(
        repo.path(),
        &[
            OsStr::new("commit"),
            OsStr::new("-am"),
            OsStr::new("main change"),
        ],
    );
    git(
        repo.path(),
        &[OsStr::new("checkout"), OsStr::new("feature")],
    );
    git_allow_failure(repo.path(), &[OsStr::new("rebase"), OsStr::new("main")]);
    repo
}

#[test]
fn probes_a_clean_repository() {
    let repo = init_repo();
    let file = repo.path().join("a.txt");
    fs::write(&file, "hello\n").unwrap();
    git(repo.path(), &[OsStr::new("add"), OsStr::new("a.txt")]);
    git(
        repo.path(),
        &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("add a")],
    );

    let state = probe(OsStr::new("git"), repo.path(), PROBE_TIMEOUT).expect("a clean repo probes");

    assert_eq!(state.head, Head::Branch("main".into()));
    assert_eq!(state.upstream, None);
    assert_eq!(state.dirty, 0);
    assert_eq!(state.untracked, 0);
    assert_eq!(state.conflicts, 0);
    assert_eq!(state.operation, None);
    assert!(!state.stale);
}

#[test]
fn counts_a_modified_and_an_untracked_file() {
    let repo = init_repo();
    let tracked = repo.path().join("tracked.txt");
    fs::write(&tracked, "hello\n").unwrap();
    git(repo.path(), &[OsStr::new("add"), OsStr::new("tracked.txt")]);
    git(
        repo.path(),
        &[
            OsStr::new("commit"),
            OsStr::new("-m"),
            OsStr::new("add tracked"),
        ],
    );
    fs::write(&tracked, "hello again\n").unwrap();
    fs::write(repo.path().join("untracked.txt"), "new\n").unwrap();

    let state = probe(OsStr::new("git"), repo.path(), PROBE_TIMEOUT).expect("probe succeeds");

    assert_eq!(state.dirty, 1);
    assert_eq!(state.untracked, 1);
    assert!(!state.stale);
}

#[test]
fn reports_a_merge_in_progress() {
    let repo = merge_conflict_repo();

    let state = probe(OsStr::new("git"), repo.path(), PROBE_TIMEOUT)
        .expect("probe succeeds despite the conflict");

    assert_eq!(state.operation, Some(GitOperation::Merge));
    assert!(state.conflicts >= 1);
}

#[test]
fn reports_a_rebase_in_progress() {
    let repo = rebase_conflict_repo();

    let state = probe(OsStr::new("git"), repo.path(), PROBE_TIMEOUT)
        .expect("probe succeeds despite the conflict");

    assert_eq!(state.operation, Some(GitOperation::Rebase));
}

#[test]
fn detects_the_operation_in_a_linked_worktree() {
    let repo = merge_conflict_repo();
    let worktree_parent = tempdir().unwrap();
    let worktree = worktree_parent.path().join("wt");
    git(
        repo.path(),
        &[
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new("other"),
            worktree.as_os_str(),
            OsStr::new("main"),
        ],
    );

    let main_state =
        probe(OsStr::new("git"), repo.path(), PROBE_TIMEOUT).expect("the main checkout probes");
    let linked_state =
        probe(OsStr::new("git"), &worktree, PROBE_TIMEOUT).expect("the linked worktree probes");

    assert_eq!(main_state.operation, Some(GitOperation::Merge));
    assert_eq!(
        linked_state.operation, None,
        "a merge in the main checkout must not show up in a sibling worktree"
    );
}

#[test]
fn a_directory_outside_a_repository_probes_to_none() {
    let dir = tempdir().unwrap();

    assert_eq!(probe(OsStr::new("git"), dir.path(), PROBE_TIMEOUT), None);
}

#[test]
fn a_missing_git_binary_probes_to_none() {
    let repo = init_repo();

    assert_eq!(
        probe(
            OsStr::new("/definitely/missing/git"),
            repo.path(),
            PROBE_TIMEOUT
        ),
        None
    );
}

#[test]
fn a_timeout_marks_the_state_stale() {
    let repo = init_repo();
    let scripts = tempdir().unwrap();
    let pid_file = scripts.path().join("child.pid");
    // Prints a valid, minimal branch header immediately — before forking anything —
    // so the parser always has something to work with regardless of how the
    // grandchild-forking race below lands: this test is about the timeout and the
    // cleanup, not about the parser's empty-input corner case (covered separately by
    // `parse::tests::empty_output_is_none`).
    let script = write_script(
        scripts.path(),
        "hanging-git",
        &format!(
            "#!/bin/sh\n\
             printf '# branch.oid abcdef1234567890'\n\
             printf '\\0'\n\
             printf '# branch.head main'\n\
             printf '\\0'\n\
             sleep 100 &\n\
             printf '%s\\n' \"$!\" > '{}'\n\
             wait\n",
            pid_file.display()
        ),
    );
    // Generous enough that forking `sh` and its backgrounded `sleep` reliably gets
    // scheduled before the deadline even when the rest of this suite is hammering the
    // machine with concurrent git spawns (300ms and 1s both measurably flaked here
    // under parallel test load — this is scheduler latency under contention, not the
    // wrapper being slow to fork).
    let timeout = Duration::from_secs(2);
    let started = Instant::now();

    let state = probe(script.as_os_str(), repo.path(), timeout);
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(4),
        "probe did not return promptly after its timeout: {elapsed:?}"
    );
    let state = state.expect("a timeout still returns the state parsed so far");
    assert!(state.stale);
    assert_eq!(state.head, Head::Branch("main".into()));

    // The wrapper recorded its backgrounded grandchild's pid before blocking on it.
    // `probe` must terminate and reap not just the wrapper but its whole process group
    // — a probe that leaves a grandchild running (or merely unreaped) will eventually
    // exhaust the daemon.
    let pid_deadline = Instant::now() + Duration::from_secs(2);
    let mut child_pid = None;
    while Instant::now() < pid_deadline {
        if let Ok(text) = fs::read_to_string(&pid_file)
            && let Ok(pid) = text.trim().parse::<libc::pid_t>()
        {
            child_pid = Some(pid);
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let child_pid = child_pid.expect("the wrapper never recorded its backgrounded child's pid");

    let gone_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let alive = unsafe { libc::kill(child_pid, 0) } == 0;
        if !alive || Instant::now() >= gone_deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        unsafe { libc::kill(child_pid, 0) },
        -1,
        "the backgrounded grandchild survived the probe's timeout cleanup"
    );
    assert_eq!(
        io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH),
        "expected the grandchild's pid to be gone entirely, not merely unsignalable"
    );
}

#[test]
fn a_timeout_before_any_output_is_none() {
    // Distinct from `a_timeout_marks_the_state_stale`: that fake git prints a valid
    // branch header before hanging, so the probe has a partial parse to mark stale.
    // This one never prints anything before the deadline hits — no `# branch.*`
    // header, no state, nothing for `GitState` to represent (`Head` has no "unknown"
    // variant) — so `probe` returns `None`, exactly as it does for a directory outside
    // a repository or a missing git binary. This boundary is the one the doc comment
    // on `probe` calls out explicitly: "any timeout" is not the rule, "a timeout (or a
    // cap) before even one header arrived" is.
    let repo = init_repo();
    let scripts = tempdir().unwrap();
    let script = write_script(
        scripts.path(),
        "silent-hanging-git",
        "#!/bin/sh\nsleep 100\n",
    );

    let state = probe(script.as_os_str(), repo.path(), Duration::from_millis(100));

    assert_eq!(state, None);
}

#[test]
fn output_over_the_cap_is_truncated_and_stale() {
    let repo = init_repo();
    let scripts = tempdir().unwrap();
    let script = write_script(
        scripts.path(),
        "huge-output-git",
        "#!/bin/sh\n\
         printf '# branch.oid abcdef1234567890'\n\
         printf '\\0'\n\
         printf '# branch.head main'\n\
         printf '\\0'\n\
         yes '? file.txt' | head -n 100000 | tr '\\n' '\\0'\n",
    );

    let state = probe(script.as_os_str(), repo.path(), PROBE_TIMEOUT)
        .expect("complete records before the cap still parse");

    assert!(state.stale);
    assert!(state.untracked > 0);
}

#[test]
fn a_stale_probe_still_reports_the_operation() {
    // The operation comes from `stat` on the worktree's own git dir (design decision
    // 7), not from `status` output, so it does not depend on how much of that output
    // the probe managed to read. Skipping it on the stale path dropped the red
    // `rebase` marker on exactly the repositories large enough to hit the cap or the
    // timeout — the moment it matters most. The repository here really is mid-rebase;
    // only the `status` half of the probe is faked, and it is faked into going stale.
    let repo = rebase_conflict_repo();
    let scripts = tempdir().unwrap();
    let script = write_script(
        scripts.path(),
        "huge-output-git",
        "#!/bin/sh\n\
         printf '# branch.oid abcdef1234567890'\n\
         printf '\\0'\n\
         printf '# branch.head main'\n\
         printf '\\0'\n\
         yes '? file.txt' | head -n 100000 | tr '\\n' '\\0'\n",
    );

    let state = probe(script.as_os_str(), repo.path(), PROBE_TIMEOUT)
        .expect("complete records before the cap still parse");

    assert!(
        state.stale,
        "the over-cap read must still mark the state stale"
    );
    assert_eq!(
        state.operation,
        Some(GitOperation::Rebase),
        "a degraded probe must not lose the in-progress operation"
    );
}

#[test]
fn the_probe_passes_no_optional_locks() {
    let repo = init_repo();
    let scripts = tempdir().unwrap();
    let argv_log = scripts.path().join("argv.log");
    let script = write_script(
        scripts.path(),
        "argv-recording-git",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf '# branch.oid abcdef1234567890\\0# branch.head main\\0'\n",
            argv_log.display()
        ),
    );

    let state = probe(script.as_os_str(), repo.path(), PROBE_TIMEOUT);
    assert!(state.is_some());

    let recorded = fs::read_to_string(&argv_log).unwrap();
    let status_line = recorded
        .lines()
        .find(|line| line.contains("--porcelain=v2"))
        .expect("the status invocation was recorded");
    assert!(
        status_line.contains("--no-optional-locks"),
        "missing --no-optional-locks in {status_line:?}"
    );
    assert!(status_line.contains("-z"), "missing -z in {status_line:?}");
}
