//! M8a.8: run preflight (decision 17), project settings (decision 53) and protected
//! files (decision 56), each against a real temporary repository. The worktrees are in
//! `run_git_worktrees.rs`, the review worktree and its diff in `run_git_review.rs`,
//! `verify_done`, `count_commits` and `diff_so_far` in `run_git_done.rs`, and the
//! environment check alone in `run_git_env.rs`.

mod support;

use daemon::run::git::{preflight, project_settings, protected_files};
use daemon::run::globs::ProtectedMatcher;
use daemon::run::plan::BUILTIN_PROTECTED;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, write, wt_dir};

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

    // A bare repository.
    let bare = tempfile::tempdir().unwrap();
    let bare_root = bare.path().canonicalize().unwrap();
    out(&bare_root, &["init", "-q", "--bare"]);
    assert_eq!(
        preflight(real_git(), &bare_root, T).unwrap_err(),
        format!(
            "anthrex runs need a working tree; {} is a bare repository",
            bare_root.display()
        )
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
        // Case differs from the built-ins: still protected (fix round 1, finding 5).
        ("lib/agents.md", "lower\n"),
        ("notes/Claude.MD", "mixed\n"),
    ]);
    write(&repo.root, "CLAUDE.md", "untracked\n");
    let builtin: Vec<String> = BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect();
    let matcher = ProtectedMatcher::new(&builtin).unwrap();

    let files = protected_files(real_git(), &repo.root, &base, &matcher, T).unwrap();

    assert_eq!(
        files,
        vec![
            ".claude/settings.json".to_string(),
            "AGENTS.md".to_string(),
            "docs/AGENTS.md".to_string(),
            "lib/agents.md".to_string(),
            "notes/Claude.MD".to_string(),
        ]
    );
}

/// Final fix batch F1, finding D-13: a run started from inside another run's worktree
/// would take that run's branch as its base, merge into it at accept, and have its base
/// deleted by the other run's discard. `anthrex/` is reserved.
#[test]
fn preflight_refuses_a_base_branch_under_anthrex() {
    let repo = repo();
    out(
        &repo.root,
        &["checkout", "-q", "-b", "anthrex/r1/integration"],
    );
    assert_eq!(
        preflight(real_git(), &repo.root, T).unwrap_err(),
        format!(
            "{} is on anthrex/r1/integration, a branch reserved for runs; check out your own branch first",
            repo.root.display()
        )
    );
}

/// Final fix batch F1, fix round 1 (N2): a repository with per-worktree config is
/// refused, since that config would live where the engine cannot keep it inert.
#[test]
fn preflight_refuses_per_worktree_config() {
    let repo = repo();
    out(&repo.root, &["config", "extensions.worktreeConfig", "true"]);
    assert_eq!(
        preflight(real_git(), &repo.root, T).unwrap_err(),
        format!(
            "{} uses per-worktree config (extensions.worktreeConfig), which anthrex runs do not support",
            repo.root.display()
        )
    );
}
