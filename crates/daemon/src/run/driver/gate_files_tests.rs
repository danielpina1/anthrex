//! Milestone 9.13 decision 9: gate files read from a real repository at its base
//! commit.

use std::path::Path;
use std::process::Command;

use proto::ProfileSpec;

use super::*;
use crate::run::gate_words::gate_words;
use crate::run::plan::resolve_profile;

const GIT: &str = "git";
const TIMEOUT: Duration = Duration::from_secs(30);

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(GIT)
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A repository whose one commit holds an executable `scripts/check.sh`, a
/// `Makefile`, a directory `test/`, a symlink `link.sh` and `scripts/sub/x.sh`; its
/// base commit.
fn repo(root: &Path) -> String {
    write(&root.join("scripts/check.sh"), "#!/bin/sh\nexit 0\n");
    write(&root.join("scripts/sub/x.sh"), "exit 0\n");
    write(&root.join("Makefile"), "test:\n\ttrue\n");
    write(&root.join("test/a.txt"), "a\n");
    std::os::unix::fs::symlink("scripts/check.sh", root.join("link.sh")).unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "-A"]);
    git(root, &["update-index", "--chmod=+x", "scripts/check.sh"]);
    git(root, &["commit", "-q", "-m", "base"]);
    git(root, &["rev-parse", "HEAD"])
}

fn words_of(check: &str) -> Vec<String> {
    let mut profile = resolve_profile(&ProfileSpec::default(), &ProfileSpec::default());
    profile.check = Some(check.to_string());
    gate_words(&profile)
}

fn pinned(root: &Path, base: &str, path: &str) -> PinnedFile {
    PinnedFile {
        path: path.to_string(),
        blob: git(root, &["rev-parse", &format!("{base}:{path}")]),
    }
}

fn absent_pin(path: &str) -> PinnedFile {
    PinnedFile {
        path: path.to_string(),
        blob: String::new(),
    }
}

#[test]
fn gate_files_are_read_at_start() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let base = repo(root);
    let mode = git(root, &["ls-tree", &base, "--", "scripts/check.sh"]);
    assert!(mode.starts_with("100755 blob "), "{mode}");
    // C15: `gate_words` adds `Makefile` for `make`; `read` adds nothing itself.
    let words = words_of("sh scripts/check.sh test link.sh make");
    assert!(words.contains(&"Makefile".to_string()), "{words:?}");
    let files = read(OsStr::new(GIT), root, &base, &words, TIMEOUT).unwrap();
    // PR #60 review I3: the names make prefers over the tracked `Makefile` are pinned
    // absent (`blob` empty).
    assert_eq!(
        files,
        vec![
            pinned(root, &base, "Makefile"),
            pinned(root, &base, "scripts/check.sh"),
            absent_pin("GNUmakefile"),
            absent_pin("makefile"),
        ]
    );
    // `read` itself adds no implicit file.
    let bare = read(OsStr::new(GIT), root, &base, &["make".to_string()], TIMEOUT).unwrap();
    assert_eq!(bare, Vec::new());
}

/// Milestone 9.13 controller ruling: a word that names a directory (`.`, `tests/`,
/// `scripts/`) pins nothing, not even the directory's direct files; a file the same
/// command names is still pinned.
#[test]
fn a_directory_word_pins_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("pyproject.toml"), "[project]\n");
    write(&root.join("tests/test_a.py"), "def test_a(): pass\n");
    let base = repo(root);
    for check in ["ruff check .", "pytest tests/", "sh scripts/", "eslint ."] {
        let files = read(OsStr::new(GIT), root, &base, &words_of(check), TIMEOUT).unwrap();
        assert_eq!(files, Vec::new(), "{check}");
    }
    let files = read(
        OsStr::new(GIT),
        root,
        &base,
        &words_of("pytest tests/ && sh scripts/check.sh ."),
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(files, vec![pinned(root, &base, "scripts/check.sh")]);
}

#[test]
fn a_failed_read_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    repo(root);
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    let words = vec!["scripts/check.sh".to_string()];
    let err = read(OsStr::new(GIT), root, unknown, &words, TIMEOUT).unwrap_err();
    assert!(err.contains("ls-tree"), "{err}");
}

#[test]
fn no_words_runs_no_git() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("no-such-git");
    let files = read(missing.as_os_str(), tmp.path(), "HEAD", &[], TIMEOUT).unwrap();
    assert_eq!(files, Vec::new());
    // The same binary with a word fails: the empty case really ran no git.
    let words = vec!["x".to_string()];
    assert!(read(missing.as_os_str(), tmp.path(), "HEAD", &words, TIMEOUT).is_err());
}

#[test]
fn a_pathspec_magic_word_is_literal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let base = repo(root);
    let words = vec![":(glob)**".to_string()];
    let files = read(OsStr::new(GIT), root, &base, &words, TIMEOUT).unwrap();
    assert_eq!(files, Vec::new());
}

/// PR #60 review I3: with only `Makefile` tracked, a checkout where a `GNUmakefile`
/// appeared (a file, or a symbolic link) is refused before `make test` runs; the base
/// checkout passes, on a case-insensitive volume too (`makefile` is not `Makefile`).
#[test]
fn verify_refuses_a_created_gnumakefile() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let base = repo(root);
    let files = read(
        OsStr::new(GIT),
        root,
        &base,
        &words_of("make test"),
        TIMEOUT,
    )
    .unwrap();
    assert!(files.contains(&absent_pin("GNUmakefile")), "{files:?}");
    assert!(files.contains(&absent_pin("makefile")), "{files:?}");
    assert_eq!(verify(root, &files), Ok(()));
    write(&root.join("GNUmakefile"), "test: ; true\n");
    assert_eq!(verify(root, &files), Err("GNUmakefile".to_string()));
    std::fs::remove_file(root.join("GNUmakefile")).unwrap();
    std::os::unix::fs::symlink("Makefile", root.join("GNUmakefile")).unwrap();
    assert_eq!(verify(root, &files), Err("GNUmakefile".to_string()));
    std::fs::remove_file(root.join("GNUmakefile")).unwrap();
    // A dangling link counts too.
    std::os::unix::fs::symlink("nowhere", root.join("GNUmakefile")).unwrap();
    assert_eq!(verify(root, &files), Err("GNUmakefile".to_string()));
}

/// PR #60 review I3: run start pins `GNUmakefile` absent beside the tracked `Makefile`
/// and protects it (`gate_words::protect`), so a task whose branch creates one is
/// bounced at the done gate unless its `owns` names it exactly (a grant).
#[test]
fn the_done_gate_bounces_a_created_gnumakefile_unless_granted() {
    use crate::run::gate_words::protect;
    use crate::run::git::{prepare_worktree, verify_done};
    use crate::run::globs::{OwnsMatcher, ProtectedMatcher};
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let base = repo(&root);
    let files = read(
        OsStr::new(GIT),
        &root,
        &base,
        &words_of("make test"),
        TIMEOUT,
    )
    .unwrap();
    let mut protected = crate::run::plan::builtin_protected();
    protect(&mut protected, &files);
    let wt = tmp.path().join("wt");
    let branch = "anthrex/gm01/t1";
    prepare_worktree(OsStr::new(GIT), &root, branch, &base, &wt, TIMEOUT).unwrap();
    write(&wt.join("src/a.rs"), "a\n");
    write(&wt.join("GNUmakefile"), "test: ; true\n");
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "-q", "-m", "work"]);
    let generated = OwnsMatcher::new(&[]).unwrap();
    let matcher = ProtectedMatcher::new(&protected).unwrap();
    let check = |owns: &[&str]| {
        let owns: Vec<String> = owns.iter().map(|s| s.to_string()).collect();
        let (g, m) = (&generated, &matcher);
        verify_done(
            OsStr::new(GIT),
            &wt,
            &base,
            &base,
            &owns,
            g,
            m,
            None,
            TIMEOUT,
        )
        .unwrap()
    };
    let r = check(&["src/**", "*"]);
    assert_eq!(
        r.protected_changed,
        vec!["GNUmakefile".to_string()],
        "{r:?}"
    );
    let r = check(&["src/**", "GNUmakefile"]);
    assert!(r.protected_changed.is_empty(), "{r:?}");
}
