//! What a sandboxed worker may write of the repository, proven under a real macOS
//! seatbelt profile (`/usr/bin/sandbox-exec`) that allows writes only to the worktree
//! and the roots the engine grants, as Claude Code's and Codex's sandboxes do.
//!
//! Final fix batch F1b: the worker's grant holds nothing of the git common directory.
//! Its objects go to a private directory (since F1c its checkout's own repository's
//! object directory, the common store its read-only alternate; no environment needed)
//! and it commits on a detached `HEAD`; the engine imports its work. Its ordinary git work (commits, an amend, a revert, a `reset --hard`, a
//! rebase, an interactive `rebase --exec`, a conflicted merge concluded with `merge
//! --continue`) succeeds; every write to the common directory is denied: the shared
//! config, a hook, any branch (its own task's included), `packed-refs`, a reflog, the
//! files of its git dir that choose its repository and config, `objects/info/
//! alternates`, and (the object-store finding) any object or pack: the loose object
//! behind the base branch's content, a pack, a new object, a symbolic link in place of
//! an object directory. The same harness with the whole common dir writable (the
//! grant before F1) lets every write through, which shows the profile is live. Skipped
//! where `sandbox-exec` does not exist.

mod support;

use daemon::run::git::{prepare_task_worktree, sync, worker_git_dirs};
use daemon::run::role_launch::{task_repo_dir, with_worker_git_config, worker_git_roots};
use std::path::{Path, PathBuf};
use std::process::Command;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wt_dir};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// A seatbelt profile: everything allowed except file writes, which are allowed only
/// below `writable` (and to `/dev`, for `/dev/null`).
fn profile(writable: &[PathBuf]) -> String {
    let mut rules = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");
    rules.push_str("(allow file-write* (subpath \"/dev\")");
    for path in writable {
        rules.push_str(&format!(" (subpath {:?})", path.display().to_string()));
    }
    rules.push_str(")\n");
    rules
}

struct Setup {
    run: String,
    repo: support::TempRepo,
    _wt: tempfile::TempDir,
    data: tempfile::TempDir,
    common: PathBuf,
    task: PathBuf,
    base: String,
    /// The blob behind `main:g.txt`, a loose object; `main:f.txt`'s is packed.
    loose: String,
}

impl Setup {
    /// The worker's environment: its git configuration (final fix batch F1c: no object
    /// directories; its checkout's repository names its own).
    fn env(&self) -> Vec<(String, String)> {
        with_worker_git_config(Vec::new())
    }

    /// Runs `script` with `sh -c` in the task worktree under `profile`, with the
    /// worker's environment; whether it exited 0.
    fn sandboxed(&self, profile: &str, script: &str) -> bool {
        let output = Command::new(SANDBOX_EXEC)
            .args(["-p", profile, "sh", "-c", script])
            .current_dir(&self.task)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .envs(self.env())
            .output()
            .unwrap();
        if !output.status.success() {
            eprintln!("{script}: {}", String::from_utf8_lossy(&output.stderr));
        }
        output.status.success()
    }

    /// The worker's writable paths: its worktree and its grant.
    fn writable(&self) -> Vec<PathBuf> {
        let roots = worker_git_roots(self.data.path(), "t1");
        let granted = worker_git_dirs(&self.common, &self.task, &roots).unwrap();
        let mut writable = vec![self.task.clone()];
        writable.extend(granted);
        writable
    }

    fn object_path(&self, id: &str) -> PathBuf {
        self.common.join("objects").join(&id[..2]).join(&id[2..])
    }
}

fn setup(run: &str) -> Setup {
    let repo = repo();
    commit_file(&repo.root, "f.txt", "base\n", "f");
    // Everything so far packed; `g.txt`'s blob then stays loose.
    out(&repo.root, &["repack", "-a", "-d", "-q"]);
    let base = commit_file(&repo.root, "g.txt", "loose\n", "g");
    let loose = out(&repo.root, &["rev-parse", "main:g.txt"]);
    let (wt, wt_path) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare_task_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        &task_repo_dir(data.path(), "t1"),
        T,
    )
    .unwrap();
    // Another run's branch, a sibling task's branch and the run branch, which this
    // run's worker must not move either.
    out(&repo.root, &["branch", "anthrex/other/integration", &base]);
    out(&repo.root, &["branch", &format!("anthrex/{run}/t2"), &base]);
    out(
        &repo.root,
        &["branch", &format!("anthrex/{run}/integration"), &base],
    );
    out(&repo.root, &["pack-refs", "--all"]);
    let common = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap();
    Setup {
        run: run.to_string(),
        repo,
        _wt: wt,
        data,
        common,
        task,
        base,
        loose,
    }
}

/// A worker's ordinary git work on its detached `HEAD`, every step of which must
/// succeed under the grant (fix round 2, R3; F1b): commits, an amend, a revert, a
/// `reset --hard`, a rebase and an interactive `rebase --exec`, and a conflicted merge
/// concluded with `merge --continue`. `git stash` is not among them: it writes
/// `refs/stash`, which is not granted.
fn work_script() -> String {
    [
        "set -e",
        "printf 'work\\n' > a.txt && git add a.txt && git commit -q -m work",
        "printf 'more\\n' > b.txt && git add b.txt && git commit -q -m more",
        "git commit -q --amend -m 'more, amended'",
        "git revert --no-edit HEAD >/dev/null",
        "git reset -q --hard HEAD~1",
        "git rebase -q HEAD~1 >/dev/null 2>&1",
        "GIT_SEQUENCE_EDITOR=true git rebase -q -i --exec true HEAD~1 >/dev/null 2>&1",
        "ours=$(git rev-parse HEAD)",
        "git checkout -q --detach HEAD~1",
        "printf 'theirs\\n' > a.txt && git add a.txt && git commit -q -m theirs",
        "theirs=$(git rev-parse HEAD)",
        "git checkout -q --detach \"$ours\"",
        "printf 'ours\\n' > a.txt && git add a.txt && git commit -q -m ours",
        "if git merge -q --no-edit \"$theirs\" >/dev/null 2>&1; then exit 3; fi",
        "printf 'resolved\\n' > a.txt && git add a.txt",
        "GIT_EDITOR=true git merge --continue >/dev/null",
        "test \"$(git rev-parse HEAD^2)\" = \"$theirs\"",
    ]
    .join("\n")
}

/// What each attempt is, in [`attempts`]'s order after the work script.
const DENIED: [&str; 20] = [
    "the shared git config",
    "a hook",
    "the base branch",
    "another run's branch",
    "a sibling task's branch",
    "the run branch",
    "its own task's branch",
    "packed-refs",
    "its git dir's commondir",
    "its git dir's gitdir",
    "a config.worktree",
    "objects/info/alternates",
    "a link at its worktree's HEAD reflog",
    "its worktree's HEAD reflog",
    "its branch's reflog",
    "the loose object behind main:g.txt",
    "a pack of the common store",
    "a new object in the common store",
    "a link in place of an object directory",
    "a new object directory",
];

/// The work script, then every write the worker must not make.
fn attempts(s: &Setup, profile: &str) -> Vec<bool> {
    let hook = s.common.join("hooks/post-merge");
    let admin = PathBuf::from(out(&s.task, &["rev-parse", "--absolute-git-dir"]));
    let run = s.run.as_str();
    let write =
        |path: PathBuf| s.sandboxed(profile, &format!("printf 'x\\n' > '{}'", path.display()));
    let loose = s.object_path(&s.loose);
    let pack = std::fs::read_dir(s.common.join("objects/pack"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "pack"))
        .expect("a pack");
    let evil = s.data.path().join("evil");
    std::fs::create_dir_all(&evil).unwrap();
    // Another blob's object file, made outside the sandbox, to copy over the base's.
    let planted = Command::new("sh")
        .args(["-c", "printf 'evil\\n' | git hash-object -w --stdin"])
        .current_dir(&s.repo.root)
        .env("GIT_OBJECT_DIRECTORY", &evil)
        .output()
        .unwrap();
    let planted = String::from_utf8(planted.stdout).unwrap();
    let planted = evil.join(&planted[..2]).join(planted[2..].trim());
    assert!(planted.is_file(), "{}", planted.display());
    let dir = &s.loose[..2];
    vec![
        s.sandboxed(profile, &work_script()),
        s.sandboxed(profile, "git config core.fsmonitor 'touch /tmp/x'"),
        write(hook),
        s.sandboxed(profile, "git update-ref refs/heads/main HEAD"),
        s.sandboxed(
            profile,
            "git update-ref refs/heads/anthrex/other/integration HEAD",
        ),
        s.sandboxed(
            profile,
            &format!("git update-ref refs/heads/anthrex/{run}/t2 HEAD"),
        ),
        s.sandboxed(
            profile,
            &format!("git update-ref refs/heads/anthrex/{run}/integration HEAD"),
        ),
        s.sandboxed(
            profile,
            &format!("git update-ref refs/heads/anthrex/{run}/t1 HEAD"),
        ),
        write(s.common.join("packed-refs")),
        write(admin.join("commondir")),
        write(admin.join("gitdir")),
        write(admin.join("config.worktree")),
        write(s.common.join("objects/info/alternates")),
        s.sandboxed(
            profile,
            &format!(
                "mkdir -p '{logs}' && ln -s /dev/null '{logs}/HEAD'",
                logs = admin.join("logs").display()
            ),
        ),
        write(admin.join("logs/HEAD")),
        s.sandboxed(
            profile,
            &format!(
                "mkdir -p '{dir}' && printf 'x\\n' > '{dir}/t1'",
                dir = s.common.join("logs/refs/heads/anthrex").join(run).display()
            ),
        ),
        // (a) The base's content replaced with no ref moving.
        s.sandboxed(
            profile,
            &format!(
                "chmod u+w '{loose}' 2>/dev/null; cp '{planted}' '{loose}'",
                loose = loose.display(),
                planted = planted.display()
            ),
        ),
        s.sandboxed(
            profile,
            &format!(
                "chmod u+w '{p}' 2>/dev/null; printf 'x' >> '{p}'",
                p = pack.display()
            ),
        ),
        // (c) An object planted at an id the engine would later write.
        write(
            s.common
                .join("objects")
                .join(dir)
                .join("0123456789abcdef0123456789abcdef012345"),
        ),
        // (b) The engine's object writes steered outside the repository.
        s.sandboxed(
            profile,
            &format!(
                "rm -rf '{d}' && ln -s '{evil}' '{d}'",
                d = s.common.join("objects").join(dir).display(),
                evil = evil.display()
            ),
        ),
        s.sandboxed(
            profile,
            &format!("mkdir '{}'", s.common.join("objects/zz").display()),
        ),
    ]
}

#[test]
fn a_sandboxed_worker_commits_but_writes_nothing_of_the_common_dir() {
    if !Path::new(SANDBOX_EXEC).exists() {
        eprintln!("skipped: no {SANDBOX_EXEC}");
        return;
    }
    let s = setup("sb01");
    let writable = s.writable();
    assert!(
        !writable[1..].iter().any(|p| p.starts_with(&s.common)),
        "the grant names the common dir: {writable:?}"
    );
    let objects_before: Vec<_> = walk(&s.common.join("objects"));

    let results = attempts(&s, &profile(&writable));
    assert!(
        results[0],
        "the worker's ordinary git work failed in its worktree"
    );
    for (wrote, what) in results[1..].iter().zip(DENIED) {
        assert!(!wrote, "the worker wrote {what}");
    }
    assert_eq!(results.len(), DENIED.len() + 1);
    // Nothing of the common object store changed, and the base reads as it did.
    assert_eq!(walk(&s.common.join("objects")), objects_before);
    assert_eq!(out(&s.repo.root, &["rev-parse", "main"]), s.base);
    assert_eq!(out(&s.repo.root, &["show", "main:g.txt"]), "loose");
    assert_eq!(out(&s.repo.root, &["show", "main:f.txt"]), "base");
    assert!(
        !try_git(&s.repo.root, &["config", "--get", "core.fsmonitor"])
            .status
            .success()
    );
    assert_eq!(
        out(
            &s.repo.root,
            &["rev-parse", &format!("anthrex/{}/t1", s.run)]
        ),
        s.base,
        "the worker moved its own branch"
    );

    // The worker's commits are the engine's to import: the task's branch moves to them,
    // and the repository then has every object they reach.
    let tip = head(&s.task);
    assert!(
        !try_git(&s.repo.root, &["cat-file", "-e", &tip])
            .status
            .success(),
        "the worker's commit reached the common store"
    );
    assert_eq!(sync(real_git(), &s.task, T).unwrap(), tip);
    assert_eq!(
        out(
            &s.repo.root,
            &["rev-parse", &format!("anthrex/{}/t1", s.run)]
        ),
        tip
    );
    out(
        &s.repo.root,
        &["fsck", "--no-dangling", "--connectivity-only"],
    );
    assert_eq!(
        out(&s.repo.root, &["show", &format!("{tip}:a.txt")]),
        "resolved"
    );
}

/// Every file under `dir`, with its length, sorted.
fn walk(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap().flatten() {
            let meta = std::fs::symlink_metadata(entry.path()).unwrap();
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                found.push((entry.path(), meta.len()));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn the_whole_common_dir_writable_lets_every_write_through() {
    if !Path::new(SANDBOX_EXEC).exists() {
        eprintln!("skipped: no {SANDBOX_EXEC}");
        return;
    }
    let s = setup("sb02");
    let mut writable = s.writable();
    writable.push(s.common.clone());
    // F1c: the checkout's git directory is no longer inside the common dir.
    writable.push(PathBuf::from(out(
        &s.task,
        &["rev-parse", "--absolute-git-dir"],
    )));
    assert_eq!(attempts(&s, &profile(&writable)), [true; 21]);
}

/// The object-store finding, reproduced against the grant F1b replaced (the shared
/// object directories `objects/00`..`objects/ff` and `objects/pack`, and the task's own
/// branch and its lock): the worker (a) replaces the loose object behind the base
/// branch's content, and `main:g.txt` then reads as its content with no ref moving, and
/// (b) puts a link in place of an object directory, so the engine's unsandboxed object
/// writes land outside the repository. Both are harmless payloads inside the test's
/// temporary directories. [`a_sandboxed_worker_commits_but_writes_nothing_of_the_common_dir`]
/// is the same attempts denied under the grant that replaced it.
#[test]
fn the_grant_before_f1b_let_a_worker_rewrite_the_bases_content() {
    if !Path::new(SANDBOX_EXEC).exists() {
        eprintln!("skipped: no {SANDBOX_EXEC}");
        return;
    }
    let s = setup("sb04");
    let objects = s.common.join("objects");
    let branch = s.common.join("refs/heads/anthrex/sb04/t1");
    let mut old: Vec<PathBuf> = (0..=255u8)
        .map(|byte| objects.join(format!("{byte:02x}")))
        .collect();
    old.extend([
        objects.join("pack"),
        branch.clone(),
        branch.with_file_name("t1.lock"),
    ]);
    for dir in &old[..257] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut writable = s.writable();
    writable.extend(old);
    let profile = profile(&writable);

    // (a) Another blob's object file, copied over the base's.
    let evil = s.data.path().join("evil");
    std::fs::create_dir_all(&evil).unwrap();
    let planted = Command::new("sh")
        .args(["-c", "printf 'evil\\n' | git hash-object -w --stdin"])
        .current_dir(&s.repo.root)
        .env("GIT_OBJECT_DIRECTORY", &evil)
        .output()
        .unwrap();
    let planted = String::from_utf8(planted.stdout).unwrap();
    let planted = evil.join(&planted[..2]).join(planted[2..].trim());
    let loose = s.object_path(&s.loose);
    assert!(s.sandboxed(
        &profile,
        &format!(
            "chmod u+w '{loose}' && cp '{planted}' '{loose}'",
            loose = loose.display(),
            planted = planted.display()
        ),
    ));
    assert_eq!(out(&s.repo.root, &["rev-parse", "main"]), s.base);
    assert_eq!(out(&s.repo.root, &["show", "main:g.txt"]), "evil");

    // (b) The directory of the next object the engine writes, made a link.
    let content = "the next engine object\n";
    let id = Command::new("sh")
        .args([
            "-c",
            &format!("printf '{content}' | git hash-object --stdin"),
        ])
        .current_dir(&s.repo.root)
        .output()
        .unwrap();
    let id = String::from_utf8(id.stdout).unwrap().trim().to_string();
    let outside = s.data.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    assert!(s.sandboxed(
        &profile,
        &format!(
            "rm -rf '{d}' && ln -s '{outside}' '{d}'",
            d = objects.join(&id[..2]).display(),
            outside = outside.display()
        ),
    ));
    let written = Command::new("sh")
        .args([
            "-c",
            &format!("printf '{content}' | git hash-object -w --stdin"),
        ])
        .current_dir(&s.repo.root)
        .output()
        .unwrap();
    assert!(written.status.success());
    assert!(
        outside.join(&id[2..]).is_file(),
        "the engine's object write stayed in the repository"
    );
}

/// The grant is found from the repository's side: a `.git` file the worker pointed at
/// another worktree's git dir changes nothing. A root inside the common dir is refused.
#[test]
fn the_grant_never_follows_the_worktrees_git_file() {
    let s = setup("sb03");
    let roots = worker_git_roots(s.data.path(), "t1");
    let own = PathBuf::from(out(&s.task, &["rev-parse", "--absolute-git-dir"]));
    let other = s.task.with_file_name("users-own");
    out(
        &s.repo.root,
        &["worktree", "add", "-q", "--detach", other.to_str().unwrap()],
    );
    let other_admin = out(&other, &["rev-parse", "--absolute-git-dir"]);
    std::fs::write(s.task.join(".git"), format!("gitdir: {other_admin}\n")).unwrap();
    let granted = worker_git_dirs(&s.common, &s.task, &roots).unwrap();
    assert!(granted.contains(&own.join("index")), "{granted:?}");
    assert!(
        !granted.iter().any(|p| p.starts_with(&other_admin)),
        "{granted:?}"
    );

    // A directory no worktree of the repository names is refused.
    let stray = tempfile::tempdir().unwrap();
    let err = worker_git_dirs(&s.common, stray.path(), &roots).unwrap_err();
    assert!(err.contains("is not a linked worktree of"), "{err}");

    // F1b: nothing of the common directory is ever granted.
    let err = worker_git_dirs(&s.common, &s.task, &[s.common.join("objects")]).unwrap_err();
    assert!(err.contains("overlaps the git common directory"), "{err}");
}
