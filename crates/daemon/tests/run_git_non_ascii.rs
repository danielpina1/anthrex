//! Milestone 9 decision 23a, as the M9.4 review fixes scope it: a path with a non-ASCII
//! character is protected at the done gate only. The run's protected files, the fast
//! path's `owned_protected` and the plan's protected notes behave as M8a and M8b left
//! them, so a tracked `docs/café.md` is an ordinary file there.

mod support;

use daemon::run::git::{
    create_run_branch, preflight, prepare_worktree, protected_files, verify_done,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher, escape_path};
use daemon::run::plan::BUILTIN_PROTECTED;
use daemon::run::triage::owned_protected;
use daemon::run::validate::protected_notes;
use support::run_git::{T, commit_file, out, real_git, repo, write, wt_dir};

fn builtin() -> Vec<String> {
    BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect()
}

#[test]
fn non_ascii_paths_are_protected_at_the_done_gate_only() {
    let repo = repo();
    for (path, content) in [
        ("docs/café.md", "café\n"),
        ("locales/日本語.json", "{}\n"),
        ("AGENTS.md", "agents\n"),
    ] {
        write(&repo.root, path, content);
        out(&repo.root, &["add", "--", path]);
    }
    out(&repo.root, &["commit", "-q", "-m", "tracked"]);
    let pre = preflight(real_git(), &repo.root, T).unwrap();
    let protected = builtin();
    let matcher = ProtectedMatcher::new(&protected).unwrap();

    // The run's protected files: the built-ins' match only.
    let files = protected_files(real_git(), &repo.root, &pre.base_sha, &matcher, T).unwrap();
    assert_eq!(files, vec!["AGENTS.md".to_string()]);
    // The fast path: `docs/**` owns nothing protected.
    let docs = vec!["docs/**".to_string()];
    assert_eq!(owned_protected(&docs, &protected, &files), None);
    // The plan: no protected note on a task owning `docs/**` or `locales/**`.
    let owns = vec!["docs/**".to_string(), "locales/**".to_string()];
    assert!(protected_notes(&owns, &files).is_empty());

    // The done gate: a changed non-ASCII path is protected unless `owns` names it.
    let (_keep, wt) = wt_dir();
    create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/na01/integration",
        &pre.base_sha,
        &wt.join("runs/na01/integration"),
        T,
    )
    .unwrap();
    let task = wt.join("runs/na01/t1");
    prepare_worktree(
        real_git(),
        &repo.root,
        "anthrex/na01/t1",
        &pre.base_sha,
        &task,
        T,
    )
    .unwrap();
    commit_file(&task, "src/ü file.rs", "fn ü() {}\n", "unicode");
    let none = OwnsMatcher::new(&[]).unwrap();
    let done = |owns: &[&str]| {
        let owns: Vec<String> = owns.iter().map(|s| s.to_string()).collect();
        verify_done(
            real_git(),
            &task,
            &pre.base_sha,
            &pre.base_sha,
            &owns,
            &none,
            &matcher,
            None,
            T,
        )
        .unwrap()
    };
    let covered = done(&["src/**"]);
    assert_eq!(covered.protected_changed, vec!["src/ü file.rs".to_string()]);
    assert!(
        covered.outside_owns.is_empty(),
        "{:?}",
        covered.outside_owns
    );
    let named = done(&["src/ü file.rs"]);
    assert!(
        named.protected_changed.is_empty(),
        "{:?}",
        named.protected_changed
    );
    assert!(named.outside_owns.is_empty(), "{:?}", named.outside_owns);
}

/// Ruling C-28 (4): a sync task resolving a conflict in `docs/café[1].md` owns it
/// escaped (`escape_path`), and a non-ASCII path is protected at the done gate unless
/// `owns` names it: the escaped entry names it, so the task passes. A protected
/// `.claude/[x].json` is allowed only when `owns` names it exactly, escaped; a real
/// class covering it never does.
#[test]
fn an_escaped_owns_entry_names_a_protected_path_at_the_done_gate() {
    let repo = repo();
    let pre = preflight(real_git(), &repo.root, T).unwrap();
    let matcher = ProtectedMatcher::new(&builtin()).unwrap();
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/na02/t1");
    prepare_worktree(
        real_git(),
        &repo.root,
        "anthrex/na02/t1",
        &pre.base_sha,
        &task,
        T,
    )
    .unwrap();
    commit_file(&task, "docs/café[1].md", "café\n", "resolved");
    commit_file(&task, ".claude/[x].json", "{}\n", "settings");
    let none = OwnsMatcher::new(&[]).unwrap();
    let done = |owns: &[String]| {
        verify_done(
            real_git(),
            &task,
            &pre.base_sha,
            &pre.base_sha,
            owns,
            &none,
            &matcher,
            None,
            T,
        )
        .unwrap()
    };
    let cafe = escape_path("docs/café[1].md");
    let both = done(&[cafe.clone(), escape_path(".claude/[x].json")]);
    assert!(
        both.protected_changed.is_empty(),
        "{:?}",
        both.protected_changed
    );
    assert!(both.outside_owns.is_empty(), "{:?}", both.outside_owns);

    let only_docs = done(std::slice::from_ref(&cafe));
    assert_eq!(
        only_docs.protected_changed,
        [".claude/[x].json".to_string()]
    );

    let class = done(&[cafe, ".claude/[xy].json".to_string()]);
    assert_eq!(class.protected_changed, [".claude/[x].json".to_string()]);
}
