//! Milestone 9.1 task M9.1.7, review ruling C-8: what a command graph's cache key and
//! the toolchain id are read from, and two module directories with one name; ruling
//! C-9: a manifest the listing cannot fingerprint keys the graph on the `HEAD` tree.

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

fn commit_all(repo: &Path, message: &str) {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
}

/// Reads a command graph with `manifests` twice, commits `change`, and reads it
/// again: how many times the command ran.
fn runs_across_a_commit(
    tmp: &Path,
    manifests: &[&str],
    setup: &dyn Fn(&Path),
    change: &dyn Fn(&Path),
) -> usize {
    let (repo, counter) = counted_graph(tmp);
    setup(&repo);
    commit_all(&repo, "base");
    let manifests = names(manifests);
    let read = || {
        module_graph(
            OsStr::new(GIT),
            &repo,
            &tmp.join("data"),
            &command_tiers("sh graph.sh"),
            &names(&["mods/*"]),
            &manifests,
            &[],
            None,
        )
    };
    assert!(matches!(read(), GraphState::Known(_)));
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(runs(&counter).len(), 1, "cached on one tree");
    change(&repo);
    commit_all(&repo, "change");
    assert!(matches!(read(), GraphState::Known(_)));
    runs(&counter).len()
}

fn edit_graph_sh(repo: &Path) {
    let script = std::fs::read_to_string(repo.join("graph.sh")).unwrap();
    write(
        &repo.join("graph.sh"),
        &script.replace("{\"a\":[]}", "{\"a\": []}"),
    );
}

#[test]
fn a_gitignored_manifest_keys_on_the_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let setup = |repo: &Path| {
        write(&repo.join(".gitignore"), "deps.txt\n");
        write(&repo.join("deps.txt"), "one\n");
    };
    let n = runs_across_a_commit(tmp.path(), &["deps.txt"], &setup, &edit_graph_sh);
    assert_eq!(n, 2, "a commit re-runs a graph whose manifest is ignored");
}

#[test]
fn a_symlinked_manifest_keys_on_the_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let setup = |repo: &Path| {
        write(&repo.join("real/deps.txt"), "one\n");
        std::os::unix::fs::symlink("real/deps.txt", repo.join("deps.txt")).unwrap();
    };
    let change = |repo: &Path| write(&repo.join("real/deps.txt"), "two\n");
    let n = runs_across_a_commit(tmp.path(), &["deps.txt"], &setup, &change);
    assert_eq!(n, 2, "a commit re-runs a graph whose manifest is a link");
}

#[test]
fn a_manifest_in_a_symlinked_directory_keys_on_the_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let setup = |repo: &Path| {
        write(&repo.join("real/deps.txt"), "one\n");
        std::os::unix::fs::symlink("real", repo.join("linked")).unwrap();
    };
    let change = |repo: &Path| write(&repo.join("real/deps.txt"), "two\n");
    let n = runs_across_a_commit(tmp.path(), &["linked/deps.txt"], &setup, &change);
    assert_eq!(
        n, 2,
        "a commit re-runs a graph whose manifest is under a link"
    );
}

#[test]
fn a_manifest_that_matches_nothing_keys_on_the_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let n = runs_across_a_commit(tmp.path(), &["nothing.txt"], &|_| {}, &edit_graph_sh);
    assert_eq!(n, 2, "a commit changing graph.sh re-runs it");
}

#[test]
fn regular_manifests_do_not_key_on_the_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let setup = |repo: &Path| write(&repo.join("deps.txt"), "one\n");
    let change = |repo: &Path| write(&repo.join("README"), "unrelated\n");
    let n = runs_across_a_commit(tmp.path(), &["deps.txt"], &setup, &change);
    assert_eq!(n, 1, "an unrelated commit keeps the cached graph");
}

#[test]
fn an_unborn_head_where_the_tree_is_needed_caches_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, counter) = counted_graph(tmp.path());
    let read = || {
        module_graph(
            OsStr::new(GIT),
            &repo,
            &tmp.path().join("data"),
            &command_tiers("sh graph.sh"),
            &names(&["mods/*"]),
            &names(&["nothing.txt"]),
            &[],
            None,
        )
    };
    assert!(matches!(read(), GraphState::Known(_)));
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(runs(&counter).len(), 2, "no key, so no cache hit");
    assert!(!tmp.path().join("data").join(GRAPH_CACHE_FILE).exists());
}

/// A cargo workspace (`members = ["crates/*"]`, crates `a` and `b`), read through a
/// stub runner that counts calls: how many ran over two reads, a commit of `change`,
/// and a third read.
fn cargo_runs_across_a_commit(setup: &dyn Fn(&Path), change: &dyn Fn(&Path)) -> usize {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    write(
        &repo.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    );
    for name in ["a", "b"] {
        write(
            &repo.join(format!("crates/{name}/Cargo.toml")),
            &format!("[package]\nname = \"ax-{name}\"\n"),
        );
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    setup(&repo);
    commit_all(&repo, "base");
    let calls = std::cell::Cell::new(0);
    let stub = |_: &Path, _: &str, _: &[(String, String)], _: Duration, _: Option<&ConfineSpec>| {
        calls.set(calls.get() + 1);
        let outcome = ShellOutcome {
            ok: true,
            code: Some(0),
            timed_out: false,
            tail: String::new(),
            secs: 0,
        };
        let json = r#"{"workspace_root":"/w","workspace_members":[],"packages":[]}"#;
        (outcome, Some(json.to_string()))
    };
    let tiers = TierProfile {
        module_graph: GraphSource::Cargo,
        module_names: ModuleNames::Cargo,
        ..TierProfile::default()
    };
    let read = || {
        module_graph_with(
            OsStr::new(GIT),
            &repo,
            &tmp.path().join("data"),
            &tiers,
            (&[], &[]),
            &[],
            None,
            GRAPH_TIMEOUT,
            &stub,
        )
    };
    assert!(matches!(read(), GraphState::Known(_)));
    assert!(matches!(read(), GraphState::Known(_)));
    assert_eq!(calls.get(), 1, "cached on one tree");
    change(&repo);
    commit_all(&repo, "change");
    assert!(matches!(read(), GraphState::Known(_)));
    calls.get()
}

fn unrelated(repo: &Path) {
    write(&repo.join("README"), "unrelated\n");
}

#[test]
fn a_listed_cargo_workspace_does_not_key_on_the_tree() {
    assert_eq!(cargo_runs_across_a_commit(&|_| {}, &unrelated), 1);
}

#[test]
fn an_ignored_cargo_member_keys_on_the_tree() {
    let ignore = |repo: &Path| write(&repo.join(".gitignore"), "crates/b/\n");
    assert_eq!(cargo_runs_across_a_commit(&ignore, &unrelated), 2);
}

#[test]
fn a_symlinked_cargo_manifest_keys_on_the_tree() {
    let link = |repo: &Path| {
        write(
            &repo.join("real/Cargo.toml"),
            "[package]\nname = \"ax-b\"\n",
        );
        std::fs::remove_file(repo.join("crates/b/Cargo.toml")).unwrap();
        std::os::unix::fs::symlink("../../real/Cargo.toml", repo.join("crates/b/Cargo.toml"))
            .unwrap();
    };
    assert_eq!(cargo_runs_across_a_commit(&link, &unrelated), 2);
}
