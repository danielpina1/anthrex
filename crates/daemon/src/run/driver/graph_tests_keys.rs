//! Milestone 9.1 task M9.1.7, review ruling C-8: what a command graph's cache key and
//! the toolchain id are read from, and two module directories with one name.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::tests::{GIT, command_tiers, git, names, runs, write};
use super::*;

#[test]
fn two_module_directories_with_one_name_are_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    write(&repo.join("mods/a/x.txt"), "a");
    write(&repo.join("other/a/x.txt"), "a");
    write(&repo.join("graph.sh"), "echo '{\"a\":[]}'\n");
    let state = module_graph(
        OsStr::new(GIT),
        &repo,
        &tmp.path().join("data"),
        &command_tiers("sh graph.sh"),
        &names(&["mods/*", "other/*"]),
        &[],
        &[],
        None,
    );
    assert_eq!(state, GraphState::Unknown("two modules are named a".into()));
}

/// A command graph in a git repository at `<tmp>/repo`, counting its runs.
fn counted_graph(tmp: &Path) -> (PathBuf, PathBuf) {
    let repo = tmp.join("repo");
    write(&repo.join("mods/a/x.txt"), "a");
    let counter = tmp.join("graph-runs");
    write(
        &repo.join("graph.sh"),
        &format!("echo run >> '{}'\necho '{{\"a\":[]}}'\n", counter.display()),
    );
    git(&repo, &["init", "-q", "-b", "main"]);
    (repo, counter)
}

#[test]
fn a_manifests_glob_is_expanded_and_its_edit_re_runs_the_command() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, counter) = counted_graph(tmp.path());
    write(&repo.join("deps/x.txt"), "one\n");
    write(&repo.join("deps/sub/z.txt"), "one\n");
    let manifests = names(&["deps/*.txt"]);
    let read = || {
        module_graph(
            OsStr::new(GIT),
            &repo,
            &tmp.path().join("data"),
            &command_tiers("sh graph.sh"),
            &names(&["mods/*"]),
            &manifests,
            &[],
            None,
        )
    };
    assert!(matches!(read(), GraphState::Known(_)));
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(runs(&counter).len(), 1, "cached");
    write(&repo.join("deps/x.txt"), "two\n");
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(runs(&counter).len(), 2, "a matching file's edit re-runs it");
    write(&repo.join("deps/y.txt"), "new\n");
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(runs(&counter).len(), 3, "a new matching file re-runs it");
    // `*` never crosses a `/` (the `owns` rules).
    write(&repo.join("deps/sub/z.txt"), "two\n");
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(runs(&counter).len(), 3, "a file the glob misses is no key");
}

#[test]
fn with_no_manifests_a_new_tree_re_runs_the_command() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, counter) = counted_graph(tmp.path());
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    let read = || {
        module_graph(
            OsStr::new(GIT),
            &repo,
            &tmp.path().join("data"),
            &command_tiers("sh graph.sh"),
            &names(&["mods/*"]),
            &[],
            &[],
            None,
        )
    };
    assert!(matches!(read(), GraphState::Known(_)));
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(runs(&counter).len(), 1, "cached on one tree");
    let script = std::fs::read_to_string(repo.join("graph.sh")).unwrap();
    write(
        &repo.join("graph.sh"),
        &script.replace("{\"a\":[]}", "{\"a\": []}"),
    );
    git(&repo, &["commit", "-q", "-am", "graph.sh changed"]);
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(runs(&counter).len(), 2, "a new tree re-runs it");
}

#[test]
fn a_toolchain_that_prints_only_to_stderr_is_told_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write(
        &dir.join("java17.sh"),
        "echo 'openjdk version \"17.0.2\"' >&2\n",
    );
    write(
        &dir.join("java21.sh"),
        "echo 'openjdk version \"21.0.1\"' >&2\n",
    );
    let seventeen = toolchain(dir, "sh java17.sh", &[], None);
    let twenty_one = toolchain(dir, "sh java21.sh", &[], None);
    assert_ne!(seventeen, TOOLCHAIN_UNKNOWN);
    assert_eq!(seventeen.len(), 16);
    assert_ne!(
        seventeen, twenty_one,
        "stderr is the version when stdout is empty"
    );
    assert_eq!(
        toolchain(dir, "sh java17.sh", &[], None),
        seventeen,
        "stable"
    );
}
