use super::*;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// A repository committing `files` (and a `.codex/link` symlink when `link`), and the
/// guard its HEAD gives, as `run start` builds it.
fn repo(files: &[(&str, &[u8])], link: bool) -> (tempfile::TempDir, CodexConfigGuard) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("README"), "r\n").unwrap();
    for (name, content) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    if link {
        std::fs::create_dir_all(root.join(".codex")).unwrap();
        std::os::unix::fs::symlink("../README", root.join(".codex/link")).unwrap();
    }
    git(root, &["add", "-A"]);
    git(
        root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "base",
        ],
    );
    let base = git(root, &["rev-parse", "HEAD"]).trim().to_string();
    let listing = git(
        root,
        &["ls-tree", "-r", "-z", "--full-tree", &base, "--", CODEX_DIR],
    );
    let guard = CodexConfigGuard {
        format: ObjectFormat::of(&base),
        entries: parse_ls_tree(&listing).unwrap(),
    };
    (dir, guard)
}

#[test]
fn blob_ids_match_gits() {
    assert_eq!(
        ObjectFormat::Sha1.blob_id(b""),
        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
    );
    assert_eq!(
        ObjectFormat::Sha1.blob_id(b"hello\n"),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
    assert_eq!(
        ObjectFormat::Sha256.blob_id(b""),
        "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813"
    );
    // A multi-block message, against git itself.
    let dir = tempfile::tempdir().unwrap();
    let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.path().join("big"), &big).unwrap();
    let theirs = git(dir.path(), &["hash-object", "--no-filters", "big"]);
    assert_eq!(ObjectFormat::Sha1.blob_id(&big), theirs.trim());
}

#[test]
fn a_checkout_like_its_base_passes() {
    let (dir, guard) = repo(&[(".codex/config.toml", b"model = \"x\"\n")], true);
    assert_eq!(guard.entries.len(), 2, "{guard:?}");
    guard.check(dir.path()).unwrap();

    // No `.codex` in the base or the checkout.
    let (empty, none) = repo(&[], false);
    assert!(none.entries.is_empty());
    none.check(empty.path()).unwrap();
    // An empty directory is not config.
    std::fs::create_dir_all(empty.path().join(".codex/sub")).unwrap();
    none.check(empty.path()).unwrap();
}

#[test]
fn a_written_changed_removed_or_swapped_entry_is_refused() {
    // A file the base does not have: the worker's own config (review C-I1 (a)), or one
    // merged from another task (C-I1 (b)).
    let (dir, none) = repo(&[], false);
    std::fs::create_dir_all(dir.path().join(".codex")).unwrap();
    std::fs::write(dir.path().join(".codex/config.toml"), "[mcp_servers.x]\n").unwrap();
    let error = none.check(dir.path()).unwrap_err();
    assert!(error.contains(".codex/config.toml"), "{error}");
    assert!(error.contains("no Codex session starts"), "{error}");

    // A trusted file changed.
    let (dir, guard) = repo(&[(".codex/config.toml", b"model = \"x\"\n")], false);
    std::fs::write(dir.path().join(".codex/config.toml"), "model = \"y\"\n").unwrap();
    let error = guard.check(dir.path()).unwrap_err();
    assert!(error.contains(".codex/config.toml"), "{error}");

    // One added beside it.
    let (dir, guard) = repo(&[(".codex/config.toml", b"model = \"x\"\n")], false);
    std::fs::write(dir.path().join(".codex/hooks.json"), "{}").unwrap();
    let error = guard.check(dir.path()).unwrap_err();
    assert!(error.contains(".codex/hooks.json"), "{error}");
    assert!(!error.contains(".codex/config.toml"), "{error}");

    // Removed.
    let (dir, guard) = repo(&[(".codex/config.toml", b"model = \"x\"\n")], false);
    std::fs::remove_file(dir.path().join(".codex/config.toml")).unwrap();
    assert!(guard.check(dir.path()).is_err());

    // The directory swapped for a link to one with the same file: never followed.
    let (dir, guard) = repo(&[(".codex/config.toml", b"model = \"x\"\n")], false);
    let elsewhere = dir.path().join("elsewhere");
    std::fs::rename(dir.path().join(".codex"), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.path().join(".codex")).unwrap();
    let error = guard.check(dir.path()).unwrap_err();
    assert!(error.contains(".codex"), "{error}");

    // A file swapped for a link to identical content.
    let (dir, guard) = repo(&[(".codex/config.toml", b"model = \"x\"\n")], false);
    std::fs::write(dir.path().join("copy"), "model = \"x\"\n").unwrap();
    std::fs::remove_file(dir.path().join(".codex/config.toml")).unwrap();
    std::os::unix::fs::symlink("../copy", dir.path().join(".codex/config.toml")).unwrap();
    assert!(guard.check(dir.path()).is_err());
}

#[test]
fn a_fifo_is_refused_without_blocking() {
    let (dir, none) = repo(&[], false);
    std::fs::create_dir_all(dir.path().join(".codex")).unwrap();
    let fifo = dir.path().join(".codex/config.toml");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: a plain mkfifo on a path this test owns.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let error = none.check(dir.path()).unwrap_err();
    assert!(error.contains("FIFO"), "{error}");
}

#[test]
fn ls_tree_records_are_parsed() {
    let listing = "100644 blob aaaa\t.codex/a\x00120000 blob bbbb\t.codex/l\x00160000 commit cccc\t.codex/s\x00";
    let entries = parse_ls_tree(listing).unwrap();
    assert_eq!(
        entries.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [EntryKind::File, EntryKind::Link, EntryKind::Gitlink]
    );
    assert!(parse_ls_tree("garbage\0").is_err());
}

/// F2 review N4: a tracked `.codex` file that a checkout converts (here `eol=crlf`)
/// differs from its blob, so it is refused (fail closed), and the message says why that
/// can happen with nobody changing it.
#[test]
fn a_converted_tracked_file_is_refused_with_the_reason_named() {
    let (dir, guard) = repo(
        &[
            (".gitattributes", b".codex/* text eol=crlf\n"),
            (".codex/config.toml", b"model = \"x\"\n"),
        ],
        false,
    );
    std::fs::remove_file(dir.path().join(".codex/config.toml")).unwrap();
    git(dir.path(), &["checkout", "--", ".codex/config.toml"]);
    let bytes = std::fs::read(dir.path().join(".codex/config.toml")).unwrap();
    assert!(bytes.ends_with(b"\r\n"), "{bytes:?}");
    let error = guard.check(dir.path()).unwrap_err();
    assert!(error.contains(".codex/config.toml"), "{error}");
    assert!(error.contains("line-ending or filter"), "{error}");
}
