//! M8a final fix batch F1, fix round 5 (post-breaker; re-review 3, N1): a sandboxed
//! worker may write its worktree git dir's commit files (`HEAD`, `index`, `ORIG_HEAD`,
//! `MERGE_HEAD`, `MERGE_MSG`, `AUTO_MERGE`, …). Four fix rounds each closed one route by
//! which the engine's unsandboxed git, running in that git dir, was steered onto the base
//! branch. This round removes the class: no engine git command that writes a ref or a
//! pseudo-ref runs there. The hand-back computes its merge with `merge-tree
//! --write-tree`, moves only the task's own branch (`update-ref --no-deref`, explicit
//! old value), updates the index and files with `read-tree` and `update-index`, and
//! writes `MERGE_HEAD` and `MERGE_MSG` itself, by rename, which replaces a symbolic
//! link and never follows one. A symbolic pseudo-ref is also refused before every
//! engine call (defence in depth).

mod support;

use daemon::run::git::{create_run_branch, hand_back, prepare_worktree};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wt_dir};

struct World {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    run: String,
    integration: PathBuf,
    task: PathBuf,
    base: String,
}

fn world(run: &str) -> World {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (wt, wt_path) = wt_dir();
    let integration = wt_path.join(format!("runs/{run}/integration"));
    create_run_branch(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/integration"),
        &base,
        &integration,
        T,
    )
    .unwrap();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        T,
    )
    .unwrap();
    World {
        repo,
        _wt: wt,
        run: run.to_string(),
        integration,
        task,
        base,
    }
}

impl World {
    fn main(&self) -> String {
        out(&self.repo.root, &["rev-parse", "main"])
    }

    /// The task worktree's own git directory, where the worker's grant lies.
    fn admin(&self) -> PathBuf {
        PathBuf::from(out(&self.task, &["rev-parse", "--absolute-git-dir"]))
    }

    fn main_file(&self) -> PathBuf {
        let common = PathBuf::from(out(
            &self.repo.root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ));
        common.join("refs/heads/main")
    }

    fn own(&self) -> String {
        format!("refs/heads/anthrex/{}/t1", self.run)
    }

    /// Every ref but the task's own branch, which the hand-back may move.
    fn other_refs(&self) -> String {
        let own = self.own();
        out(
            &self.repo.root,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )
        .lines()
        .filter(|line| !line.starts_with(&format!("{own} ")))
        .collect::<Vec<_>>()
        .join("\n")
    }
}

/// A task commit and a run head that merge cleanly.
fn clean_case(w: &World) -> String {
    commit_file(&w.task, "t.txt", "task\n", "task work");
    commit_file(&w.integration, "r.txt", "run\n", "run work")
}

/// A task commit and a run head that both change `f.txt`.
fn conflict_case(w: &World) -> String {
    commit_file(&w.task, "f.txt", "task\n", "task edit");
    commit_file(&w.integration, "f.txt", "run\n", "run edit")
}

const SYMREF_MAIN: &str = "ref: refs/heads/main\n";

/// Re-review 3, N1: the worker leaves `ref: refs/heads/main` in its `ORIG_HEAD` (a
/// granted file; no race needed). Before this round, the hand-back's `git merge` wrote
/// `ORIG_HEAD` through it and moved the base to the task's tip. Now the hand-back runs
/// no command that writes `ORIG_HEAD`, and a symbolic pseudo-ref is refused anyway.
#[test]
fn a_symbolic_orig_head_never_moves_the_base() {
    for (run, conflict) in [("pr01", false), ("pr02", true)] {
        let w = world(run);
        let run_head = if conflict {
            conflict_case(&w)
        } else {
            clean_case(&w)
        };
        let refs = w.other_refs();
        std::fs::write(w.admin().join("ORIG_HEAD"), SYMREF_MAIN).unwrap();

        let result = hand_back(real_git(), &w.task, &run_head, T);
        assert_eq!(w.main(), w.base, "the base moved: {result:?}");
        assert_eq!(w.other_refs(), refs, "a ref moved: {result:?}");
        let err = result.unwrap_err();
        assert!(
            err.contains("ORIG_HEAD") && err.contains("tampered"),
            "{err}"
        );
    }
}

/// N1's direct fix, for the other pseudo-refs a worker may write: a symbolic
/// `MERGE_HEAD` or `AUTO_MERGE` (a `ref:` file or a symbolic link) is refused before
/// any engine call, so no read follows it either.
#[test]
fn a_symbolic_merge_head_or_auto_merge_is_refused() {
    for (index, name) in ["MERGE_HEAD", "AUTO_MERGE"].iter().enumerate() {
        for link in [false, true] {
            let w = world(&format!("pr1{index}{}", u8::from(link)));
            let run_head = clean_case(&w);
            let file = w.admin().join(name);
            if link {
                std::os::unix::fs::symlink(w.main_file(), &file).unwrap();
            } else {
                std::fs::write(&file, SYMREF_MAIN).unwrap();
            }

            let result = hand_back(real_git(), &w.task, &run_head, T);
            assert_eq!(w.main(), w.base, "the base moved: {result:?}");
            let err = result.unwrap_err();
            assert!(err.contains(name) && err.contains("tampered"), "{err}");
        }
    }
}

/// Every file of `admin` but `index` (which these commands write, by rename), with
/// its content or, for a link, its target.
fn snapshot(admin: &Path) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(admin)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name() != "index")
        .filter_map(|entry| {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).ok()?;
            let what = if meta.file_type().is_symlink() {
                format!("-> {}", std::fs::read_link(&path).ok()?.display())
            } else if meta.is_file() {
                std::fs::read_to_string(&path).ok()?
            } else {
                "dir".to_string()
            };
            Some((entry.file_name().to_string_lossy().into_owned(), what))
        })
        .collect();
    files.sort();
    files
}

/// `git update-index -z --index-info` in `dir`, fed `input`.
fn index_info(dir: &Path, input: &[u8]) {
    let mut child = std::process::Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["update-index", "-z", "--index-info"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    assert!(child.wait().unwrap().success());
}

/// The plumbing the hand-back now uses, run by hand with plain git in a worktree whose
/// `HEAD` names the base and whose every pseudo-ref is a symbolic ref to it: none of
/// `merge-tree --write-tree`, `commit-tree`, `read-tree` (two-way, and `--reset`),
/// `update-index --index-info` or `update-ref --no-deref <own>` writes `ORIG_HEAD`, any
/// other pseudo-ref, or any ref but the one named.
#[test]
fn the_hand_backs_plumbing_writes_no_other_ref_or_pseudo_ref() {
    let w = world("pr31");
    let run_head = conflict_case(&w);
    let onto = head(&w.task);
    let admin = w.admin();
    std::fs::write(admin.join("HEAD"), SYMREF_MAIN).unwrap();
    for name in [
        "ORIG_HEAD",
        "MERGE_HEAD",
        "AUTO_MERGE",
        "REBASE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "FETCH_HEAD",
    ] {
        std::fs::write(admin.join(name), SYMREF_MAIN).unwrap();
    }
    let before = snapshot(&admin);
    let refs = w.other_refs();

    let merged = try_git(
        &w.task,
        &[
            "merge-tree",
            "--write-tree",
            "-z",
            "--no-messages",
            &onto,
            &run_head,
        ],
    );
    assert_eq!(merged.status.code(), Some(1), "no conflict");
    let text = String::from_utf8(merged.stdout).unwrap();
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let tree = fields.next().unwrap().to_string();
    let entries: Vec<&str> = fields.collect();
    assert_eq!(entries.len(), 3, "{entries:?}");
    out(&w.task, &["read-tree", "-m", "-u", &onto, &tree]);
    let mut input = format!("0 {} 0\tf.txt\0", "0".repeat(onto.len())).into_bytes();
    for entry in &entries {
        input.extend_from_slice(entry.as_bytes());
        input.push(0);
    }
    index_info(&w.task, &input);
    assert_eq!(out(&w.task, &["ls-files", "-u"]).lines().count(), 3);
    let commit = out(
        &w.task,
        &[
            "commit-tree",
            &tree,
            "-p",
            &onto,
            "-p",
            &run_head,
            "-m",
            "m",
        ],
    );
    out(&w.task, &["read-tree", "--reset", "-u", &onto]);
    out(&w.task, &["read-tree", "-m", "-u", &onto, &commit]);
    out(
        &w.task,
        &["update-ref", "--no-deref", &w.own(), &commit, &onto],
    );

    assert_eq!(snapshot(&admin), before, "a pseudo-ref was written");
    assert_eq!(w.other_refs(), refs, "a ref moved");
    assert_eq!(w.main(), w.base);
    assert_eq!(out(&w.repo.root, &["rev-parse", &w.own()]), commit);
}
