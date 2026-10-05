//! Milestone 9.6 task M9.6.12 (AGENTS.md hard rule 11, the task's addendum): every git
//! call of the documents commit passes `-C <dir> --no-optional-locks` and `-c
//! core.protectHFS=true -c core.protectNTFS=true`, reaches git with none of rule 11's
//! five variables inherited, and `GIT_INDEX_FILE` is set again, explicitly, to the
//! driver's own temporary index on exactly the index commands. Alone in its own test
//! binary, because it sets variables in this process's environment (`run_git_env.rs`'s
//! module doc is the precedent). Keep this file to this one test.

mod support;

use daemon::run::git::{DocsCommit, DocsOutcome, commit_docs, create_run_branch};
use std::ffi::OsString;
use support::recording_git;
use support::run_git::{T, commit_file, head, real_git, repo, wt_dir};

const PROTECT: [&str; 4] = ["-c", "core.protectHFS=true", "-c", "core.protectNTFS=true"];
const WRITE_FLAGS: [&str; 6] = [
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "commit.gpgSign=false",
    "-c",
    "core.logAllRefUpdates=false",
];

#[test]
fn the_docs_commit_scrubs_the_inherited_env_and_sets_its_own_index() {
    let repo = repo();
    let base = commit_file(&repo.root, "README", "r\n", "base");
    let (_wt, wt) = wt_dir();
    let integration = wt.join("runs/de01/integration");
    let branch = "anthrex/de01/integration";
    create_run_branch(real_git(), &repo.root, branch, &base, &integration, T).unwrap();
    let data = tempfile::tempdir().unwrap();
    let index = data.path().join("design/commit.index");
    let scripts = tempfile::tempdir().unwrap();
    let git = recording_git(scripts.path());
    let files = vec![("docs/specs/a.md".to_string(), b"# A\n".to_vec())];

    let previous: Vec<(&str, Option<OsString>)> = ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"]
        .iter()
        .map(|k| (*k, std::env::var_os(k)))
        .collect();
    // SAFETY: this is the only test in this binary, so no other thread spawns a process
    // or reads the environment while these writes happen (module doc).
    unsafe {
        std::env::set_var("GIT_DIR", "/nonexistent/leak-marker");
        std::env::set_var("GIT_INDEX_FILE", "/nonexistent/leak-index");
        std::env::set_var("GIT_WORK_TREE", "/nonexistent/leak-tree");
    }
    let docs = DocsCommit {
        root: &repo.root,
        index: &index,
        branch,
        expected: &base,
        integration: &integration,
        files: &files,
        appends: &[],
        stage: None,
        message: "docs: spec and plan for A",
    };
    let outcome = commit_docs(git.as_os_str(), &docs, T);
    // SAFETY: as above.
    unsafe {
        for (key, value) in &previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
    let new = match outcome {
        Ok(DocsOutcome::Committed { head, .. }) => head,
        other => panic!("{other:?}"),
    };
    assert_ne!(new, base);

    let log = std::fs::read_to_string(scripts.path().join("git.log")).unwrap();
    // Each call: its argv line, then the GIT_ variables it saw.
    let mut calls: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    for line in log.lines() {
        let mut fields = line.split('\t');
        match fields.next() {
            Some("argv") => calls.push((fields.map(String::from).collect(), Vec::new())),
            Some("env") => calls.last_mut().unwrap().1.push(fields.collect()),
            other => panic!("{other:?}"),
        }
    }
    assert!(calls.len() >= 6, "{log}");
    let mut indexed = Vec::new();
    for (argv, env) in &calls {
        assert_eq!(argv[0], "-C", "{argv:?}");
        assert_eq!(argv[2], "--no-optional-locks", "{argv:?}");
        let joined = argv.join(" ");
        assert!(joined.contains(&PROTECT.join(" ")), "{argv:?}");
        for leak in [
            "GIT_DIR=",
            "GIT_WORK_TREE=",
            "GIT_COMMON_DIR=",
            "GIT_PREFIX=",
        ] {
            assert!(
                !env.iter().any(|e| e.starts_with(leak)),
                "{argv:?}: {env:?}"
            );
        }
        let sub = argv
            .iter()
            .find(|a| {
                [
                    "ls-tree",
                    "read-tree",
                    "hash-object",
                    "update-index",
                    "write-tree",
                    "rev-parse",
                    "show",
                    "commit-tree",
                    "update-ref",
                    "checkout",
                ]
                .contains(&a.as_str())
            })
            .unwrap_or_else(|| panic!("an unexpected call {argv:?}"));
        if ["read-tree", "update-index", "write-tree"].contains(&sub.as_str()) {
            let want = format!("GIT_INDEX_FILE={}", index.display());
            assert!(env.contains(&want), "{argv:?}: {env:?}");
            indexed.push(sub.clone());
        } else {
            assert!(
                !env.iter().any(|e| e.starts_with("GIT_INDEX_FILE=")),
                "{argv:?}: {env:?}"
            );
        }
        if [
            "read-tree",
            "hash-object",
            "update-index",
            "write-tree",
            "commit-tree",
            "update-ref",
            "checkout",
        ]
        .contains(&sub.as_str())
        {
            assert!(joined.contains(&WRITE_FLAGS.join(" ")), "{argv:?}");
        }
    }
    assert_eq!(
        indexed,
        ["read-tree", "update-index", "write-tree"],
        "{log}"
    );
    assert_eq!(head(&repo.root), base, "the base branch did not move");
}
