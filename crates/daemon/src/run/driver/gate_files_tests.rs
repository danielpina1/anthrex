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
    assert_eq!(
        files,
        vec![
            pinned(root, &base, "Makefile"),
            pinned(root, &base, "scripts/check.sh"),
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
