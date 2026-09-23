//! M8a.8: run preflight (decision 17), project settings (decision 53), protected files
//! (decision 56), and the run, task and review worktrees (decisions 16, 18 and 19),
//! each against a real temporary repository. `verify_done`, `count_commits` and
//! `diff_so_far` are in `run_git_done.rs`; the environment check is alone in
//! `run_git_env.rs`.

mod support;

use daemon::run::contract::REVIEW_DIFF_MAX;
use daemon::run::git::{
    create_run_branch, preflight, prepare_review, prepare_worktree, project_settings,
    protected_files, verify_done,
};
use daemon::run::globs::OwnsMatcher;
use daemon::run::plan::BUILTIN_PROTECTED;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{
    T, commit_file, head, out, real_git, repo, try_git, worktree_block, write, wt_dir,
};

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn preflight_reports_branch_sha_and_roots() {
    let repo = repo();
    commit_file(&repo.root, "sub/deep/file.txt", "x\n", "add sub");
    out(&repo.root, &["checkout", "-q", "-b", "feature-x"]);
    let sha = commit_file(&repo.root, "sub/two.txt", "two\n", "second");

    let pre = preflight(real_git(), &repo.root.join("sub/deep"), T).unwrap();

    assert_eq!(pre.root, repo.root);
    assert_eq!(pre.project, repo.root);
    assert_eq!(pre.git_common_dir, repo.root.join(".git"));
    assert_eq!(pre.base_branch, "feature-x");
    assert_eq!(pre.base_sha, sha);
    assert!(pre.protected_files.is_empty(), "{:?}", pre.protected_files);
}

#[test]
fn preflight_from_a_linked_worktree_keeps_root_and_project_apart() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let linked = wt.join("linked checkout");
    out(
        &repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            linked.to_str().unwrap(),
        ],
    );
    let side = commit_file(&linked, "side.txt", "side\n", "side work");
    assert_ne!(side, base);

    let pre = preflight(real_git(), &linked, T).unwrap();

    assert_eq!(pre.root, linked);
    assert_eq!(pre.project, repo.root);
    assert_eq!(pre.git_common_dir, repo.root.join(".git"));
    assert_ne!(pre.git_common_dir, linked.join(".git"));
    assert_eq!(pre.base_branch, "side");
    assert_eq!(pre.base_sha, side);
}

#[test]
fn preflight_refusals() {
    // Not a repository.
    let plain = tempfile::tempdir().unwrap();
    let dir = plain.path().canonicalize().unwrap();
    assert_eq!(
        preflight(real_git(), &dir, T).unwrap_err(),
        format!("not a git repository: {}", dir.display())
    );

    // A git older than 2.38.
    let scripts = tempfile::tempdir().unwrap();
    let old_git = script(
        scripts.path(),
        "old-git",
        "for a in \"$@\"; do if [ \"$a\" = version ]; then echo 'git version 2.37.4'; exit 0; fi; done\nexec git \"$@\"",
    );
    let repo_ok = repo();
    assert_eq!(
        preflight(old_git.as_os_str(), &repo_ok.root, T).unwrap_err(),
        "anthrex runs need git 2.38 or newer for merge-tree --write-tree (found 2.37.4)"
    );

    // Detached HEAD.
    let detached = repo();
    out(&detached.root, &["checkout", "-q", "--detach"]);
    assert_eq!(
        preflight(real_git(), &detached.root, T).unwrap_err(),
        format!(
            "{} is on a detached HEAD; check out a branch first",
            detached.root.display()
        )
    );

    // No commits.
    let empty = tempfile::tempdir().unwrap();
    let empty_root = empty.path().canonicalize().unwrap();
    out(&empty_root, &["init", "-q"]);
    assert_eq!(
        preflight(real_git(), &empty_root, T).unwrap_err(),
        "the repository has no commits yet"
    );

    // No identity: no repository identity, and a git that sees no global or system
    // config either. The test process's own environment is never changed.
    let anonymous = TempRepo::new();
    let isolated_git = script(
        scripts.path(),
        "isolated-git",
        "export GIT_CONFIG_GLOBAL=/dev/null\nexport GIT_CONFIG_NOSYSTEM=1\nexec git \"$@\"",
    );
    assert_eq!(
        preflight(isolated_git.as_os_str(), &anonymous.root, T).unwrap_err(),
        format!(
            "git has no user.name/user.email configured for {}",
            anonymous.root.display()
        )
    );

    // A dirty tracked tree.
    let dirty = repo();
    write(&dirty.root, "README", "changed\n");
    assert_eq!(
        preflight(real_git(), &dirty.root, T).unwrap_err(),
        format!(
            "the working tree at {} has uncommitted changes; commit or stash them first",
            dirty.root.display()
        )
    );
}

#[test]
fn preflight_ignores_untracked_files() {
    let repo = repo();
    write(&repo.root, "scratch/notes.txt", "untracked\n");

    let pre = preflight(real_git(), &repo.root, T).unwrap();

    assert_eq!(pre.base_sha, head(&repo.root));
}

#[test]
fn run_branch_and_integration_worktree_are_created_and_locked() {
    let repo = repo();
    let base = head(&repo.root);
    commit_file(&repo.root, "later.txt", "later\n", "after base");
    let (_keep, wt) = wt_dir();
    let path = wt.join("runs/fix-r1ab/integration");

    let created = create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/fix-r1ab/integration",
        &base,
        &path,
        T,
    )
    .unwrap();

    assert_eq!(created, base);
    assert_eq!(head(&path), base);
    let block = worktree_block(&repo.root, &path).expect("git lists the worktree");
    assert!(
        block
            .lines()
            .any(|l| l == "branch refs/heads/anthrex/fix-r1ab/integration"),
        "{block}"
    );
    assert!(
        block.lines().any(|l| l == "locked anthrex run fix-r1ab"),
        "{block}"
    );
}

#[test]
fn task_branch_starts_at_the_given_run_head_and_is_reused() {
    let repo = repo();
    let first = commit_file(&repo.root, "a.txt", "a\n", "a");
    let second = commit_file(&repo.root, "b.txt", "b\n", "b");
    let (_keep, wt) = wt_dir();
    let path = wt.join("runs/r-9f0e/t1");
    let branch = "anthrex/r-9f0e/t1";

    let h = prepare_worktree(real_git(), &repo.root, branch, &first, &path, T).unwrap();
    assert_eq!(h, first);
    assert_eq!(out(&repo.root, &["rev-parse", branch]), first);
    assert_eq!(out(&path, &["symbolic-ref", "--short", "HEAD"]), branch);
    assert!(
        worktree_block(&repo.root, &path)
            .unwrap()
            .lines()
            .any(|l| l.starts_with("locked")),
        "a task worktree is locked right after creation"
    );

    // A second call with the same arguments reuses everything.
    let refs_before = out(&repo.root, &["for-each-ref"]);
    let list_before = out(&repo.root, &["worktree", "list", "--porcelain"]);
    let h = prepare_worktree(real_git(), &repo.root, branch, &first, &path, T).unwrap();
    assert_eq!(h, first);
    assert_eq!(out(&repo.root, &["for-each-ref"]), refs_before);
    assert_eq!(
        out(&repo.root, &["worktree", "list", "--porcelain"]),
        list_before
    );

    // The worktree goes (two `--force`s: it is locked); the branch is re-added.
    out(
        &repo.root,
        &[
            "worktree",
            "remove",
            "--force",
            "--force",
            path.to_str().unwrap(),
        ],
    );
    assert!(!path.exists());
    let h = prepare_worktree(real_git(), &repo.root, branch, &first, &path, T).unwrap();
    assert_eq!(h, first);
    assert_eq!(out(&path, &["symbolic-ref", "--short", "HEAD"]), branch);

    // A newer `from` while the branch has no commit of its own re-points it.
    let h = prepare_worktree(real_git(), &repo.root, branch, &second, &path, T).unwrap();
    assert_eq!(h, second);
    assert_eq!(out(&repo.root, &["rev-parse", branch]), second);
    assert_eq!(head(&path), second);
    assert_eq!(out(&path, &["symbolic-ref", "--short", "HEAD"]), branch);

    // With a commit of its own, a newer `from` leaves it alone.
    let own = commit_file(&path, "own.txt", "own\n", "task work");
    let third = commit_file(&repo.root, "c.txt", "c\n", "c");
    let h = prepare_worktree(real_git(), &repo.root, branch, &third, &path, T).unwrap();
    assert_eq!(h, own);
    assert_eq!(out(&repo.root, &["rev-parse", branch]), own);
}

#[test]
fn a_task_branch_can_live_under_the_run_branch_name() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();

    create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/r2cd/integration",
        &base,
        &wt.join("runs/r2cd/integration"),
        T,
    )
    .unwrap();
    prepare_worktree(
        real_git(),
        &repo.root,
        "anthrex/r2cd/t1",
        &base,
        &wt.join("runs/r2cd/t1"),
        T,
    )
    .unwrap();

    assert!(repo.branch_exists("anthrex/r2cd/integration"));
    assert!(repo.branch_exists("anthrex/r2cd/t1"));
    assert!(
        !try_git(&repo.root, &["branch", "anthrex/r2cd"])
            .status
            .success()
    );
}

fn tracked(files: &[(&str, &str)]) -> (TempRepo, String) {
    let repo = repo();
    for (path, content) in files {
        write(&repo.root, path, content);
        // `-f`: a global excludes file may ignore `.claude/settings.local.json`.
        out(&repo.root, &["add", "-f", "--", path]);
    }
    out(&repo.root, &["commit", "-q", "-m", "settings"]);
    let base = head(&repo.root);
    (repo, base)
}

const HOOKS: &str = r#"{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "touch /tmp/x"}]}]}}"#;
const CODEX: Option<&[&str]> = Some(&[".codex/config.toml", ".codex/hooks.json"]);

#[test]
fn project_settings_are_found_in_the_base_tree() {
    let settings = |repo: &TempRepo, base: &str, claude, codex| {
        project_settings(real_git(), &repo.root, base, claude, codex, T).unwrap()
    };

    // Hooks in the base tree are listed, even after the working tree (and a later
    // commit) drop them: the base commit is what every worktree is cut from.
    let (hooked, base) = tracked(&[(".claude/settings.json", HOOKS)]);
    commit_file(&hooked.root, ".claude/settings.json", "{}", "drop hooks");
    assert_eq!(
        settings(&hooked, &base, true, None),
        vec![".claude/settings.json".to_string()]
    );

    let (no_hooks, base) = tracked(&[
        (".claude/settings.json", r#"{"permissions": {"allow": []}}"#),
        (".claude/settings.local.json", r#"{"hooks": {}}"#),
    ]);
    assert!(settings(&no_hooks, &base, true, None).is_empty());

    let (mcp, base) = tracked(&[(".mcp.json", r#"{"mcpServers": {}}"#)]);
    assert_eq!(
        settings(&mcp, &base, true, None),
        vec![".mcp.json".to_string()]
    );

    let untracked = repo();
    write(&untracked.root, ".mcp.json", r#"{"mcpServers": {}}"#);
    let base = head(&untracked.root);
    assert!(settings(&untracked, &base, true, None).is_empty());

    let (invalid, base) = tracked(&[(".claude/settings.local.json", "{ not json")]);
    assert_eq!(
        settings(&invalid, &base, true, None),
        vec![".claude/settings.local.json".to_string()]
    );

    let (codex, base) = tracked(&[(".codex/config.toml", "model = \"o\"\n")]);
    assert_eq!(
        settings(&codex, &base, true, CODEX),
        vec![".codex/config.toml".to_string()]
    );
    assert!(settings(&codex, &base, true, None).is_empty());

    let (both, base) = tracked(&[
        (".claude/settings.json", HOOKS),
        (".mcp.json", "{}"),
        (".codex/hooks.json", "{}"),
    ]);
    assert_eq!(
        settings(&both, &base, false, CODEX),
        vec![".codex/hooks.json".to_string()]
    );
    assert_eq!(
        settings(&both, &base, true, CODEX),
        vec![
            ".claude/settings.json".to_string(),
            ".codex/hooks.json".to_string(),
            ".mcp.json".to_string(),
        ]
    );
}

#[test]
fn protected_files_lists_tracked_matches_only() {
    let (repo, base) = tracked(&[
        ("AGENTS.md", "agents\n"),
        ("docs/AGENTS.md", "nested\n"),
        (".claude/settings.json", "{}"),
        ("src/main.rs", "fn main() {}\n"),
    ]);
    write(&repo.root, "CLAUDE.md", "untracked\n");
    let builtin: Vec<String> = BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect();
    let matcher = OwnsMatcher::new(&builtin).unwrap();

    let files = protected_files(real_git(), &repo.root, &base, &matcher, T).unwrap();

    assert_eq!(
        files,
        vec![
            ".claude/settings.json".to_string(),
            "AGENTS.md".to_string(),
            "docs/AGENTS.md".to_string(),
        ]
    );
}

#[test]
fn review_worktree_is_detached_at_the_task_head_and_replaced() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/r7/t1");
    let branch = "anthrex/r7/t1";
    prepare_worktree(real_git(), &repo.root, branch, &base, &task, T).unwrap();
    let small = commit_file(&task, "src/lib.rs", "pub fn f() {}\n", "small");
    let review = wt.join("runs/r7/t1.review");

    let (b, h, patch) = prepare_review(real_git(), &repo.root, branch, &base, &review, T).unwrap();

    assert_eq!((b.as_str(), h.as_str()), (base.as_str(), small.as_str()));
    let expected =
        String::from_utf8(try_git(&repo.root, &["diff", &format!("{base}..{small}")]).stdout)
            .unwrap();
    assert_eq!(patch, expected);
    assert_eq!(head(&review), small);
    let block = worktree_block(&repo.root, &review).unwrap();
    assert!(block.lines().any(|l| l == "detached"), "{block}");

    // The next round replaces the worktree, and a diff over the limit is clamped.
    write(&review, "leftover.txt", "from round one\n");
    let big_body: String = "世世世世\n".repeat(6000);
    let big = commit_file(&task, "src/big.txt", &big_body, "big");

    let (b, h, patch) = prepare_review(real_git(), &repo.root, branch, &base, &review, T).unwrap();

    assert_eq!((b.as_str(), h.as_str()), (base.as_str(), big.as_str()));
    assert_eq!(head(&review), big);
    assert!(
        !review.join("leftover.txt").exists(),
        "the old round's worktree is gone"
    );
    assert!(
        worktree_block(&repo.root, &review)
            .unwrap()
            .lines()
            .any(|l| l == "detached")
    );
    assert!(patch.len() <= REVIEW_DIFF_MAX, "{}", patch.len());
    assert!(patch.len() >= REVIEW_DIFF_MAX - 3, "{}", patch.len());
    // `src/big.txt` sorts before `src/lib.rs`: the head is the big file's, the tail the
    // small change's.
    assert!(
        patch.starts_with("diff --git a/src/big.txt"),
        "the head of the diff is kept"
    );
    assert!(
        patch.ends_with("+pub fn f() {}\n"),
        "the tail of the diff is kept"
    );
}

#[test]
fn engine_writes_ignore_hooks_and_signing() {
    let repo = repo();
    out(&repo.root, &["config", "commit.gpgsign", "true"]);
    out(&repo.root, &["config", "gpg.program", "/bin/false"]);
    repo.failing_post_checkout();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();

    create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/hk12/integration",
        &base,
        &wt.join("runs/hk12/integration"),
        T,
    )
    .unwrap();
    let task = wt.join("runs/hk12/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/hk12/t1", &base, &task, T).unwrap();
    // The test's own commit passes `-c commit.gpgsign=false` (support::git_output).
    let newer = commit_file(&repo.root, "n.txt", "n\n", "newer");
    let repointed =
        prepare_worktree(real_git(), &repo.root, "anthrex/hk12/t1", &newer, &task, T).unwrap();
    assert_eq!(repointed, newer);
    prepare_review(
        real_git(),
        &repo.root,
        "anthrex/hk12/t1",
        &base,
        &wt.join("runs/hk12/t1.review"),
        T,
    )
    .unwrap();

    assert!(!repo.hook_marker().exists(), "no hook ran");
}

#[test]
fn engine_paths_with_spaces_and_unicode_work() {
    let repo = support::run_git::identified(TempRepo::with_prefix("ax run ü "));
    let pre = preflight(real_git(), &repo.root, T).unwrap();
    assert_eq!(pre.root, repo.root);
    let wt_keep = tempfile::Builder::new()
        .prefix("ax wt ü ")
        .tempdir_in("/tmp")
        .unwrap();
    let wt = wt_keep.path().canonicalize().unwrap();

    create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/sp01/integration",
        &pre.base_sha,
        &wt.join("runs/sp01/integration"),
        T,
    )
    .unwrap();
    let task = wt.join("runs/sp01/t1");
    prepare_worktree(
        real_git(),
        &repo.root,
        "anthrex/sp01/t1",
        &pre.base_sha,
        &task,
        T,
    )
    .unwrap();
    let h = commit_file(&task, "src/ü file.rs", "fn ü() {}\n", "unicode");
    write(&task, "lib/ü new.rs", "untracked\n");
    let none = OwnsMatcher::new(&[]).unwrap();
    let done = verify_done(
        real_git(),
        &task,
        &pre.base_sha,
        &pre.base_sha,
        &["lib/**".to_string()],
        &none,
        &none,
        None,
        T,
    )
    .unwrap();
    assert_eq!(done.outside_owns, vec!["src/ü file.rs".to_string()]);
    assert_eq!(done.untracked_in_owns, vec!["lib/ü new.rs".to_string()]);
    let (_, head_sha, patch) = prepare_review(
        real_git(),
        &repo.root,
        "anthrex/sp01/t1",
        &pre.base_sha,
        &wt.join("runs/sp01/t1.review"),
        T,
    )
    .unwrap();
    assert_eq!(head_sha, h);
    assert!(patch.contains("fn ü() {}"), "{patch}");
}
