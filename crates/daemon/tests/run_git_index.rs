//! M8a final fix batch F1c (re-review 4, C1): a worker may write its task checkout's
//! `index`, so it can replace it with a symbolic link: to the user's own `.git/index`,
//! or to a path that does not exist. Git's lockfile code resolves such a link before it
//! takes the lock, so before this fix every engine command that writes the index there
//! (salvage's `add -A` and `write-tree`, the hand-back's `read-tree` and
//! `update-index`, an abort's and a re-point's `read-tree`) wrote the link's target:
//! the user's index filled with the task's tree (their next commit carried it), or a
//! new file created wherever the link pointed.
//!
//! Now every engine git command in a task checkout works on an engine-owned copy of
//! the index (`GIT_INDEX_FILE`), installed by rename; a linked index is refused, and a
//! salvage replaces it with one rebuilt from `HEAD`. Each test here plants the link and
//! asserts the victim is byte-identical and the missing path still missing, whatever
//! the operation returned.

mod support;

use daemon::run::git::{abort_merge, create_run_branch, hand_back, prepare_worktree, salvage};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, out, real_git, repo, wrapper_git, write, wt_dir};

struct World {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    _victims: tempfile::TempDir,
    run: String,
    integration: PathBuf,
    task: PathBuf,
    base: String,
    /// The user's own index, and its bytes before the operation.
    user_index: PathBuf,
    user_index_bytes: Vec<u8>,
    /// A path that does not exist.
    missing: PathBuf,
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
    let user_index = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    ));
    let user_index_bytes = std::fs::read(&user_index).unwrap();
    let victims = tempfile::tempdir().unwrap();
    let missing = victims.path().join("nowhere").join("made-by-git");
    World {
        repo,
        _wt: wt,
        _victims: victims,
        run: run.to_string(),
        integration,
        task,
        base,
        user_index,
        user_index_bytes,
        missing,
    }
}

impl World {
    /// The task checkout's own git directory, where the worker's grant lies.
    fn admin(&self) -> PathBuf {
        PathBuf::from(out(&self.task, &["rev-parse", "--absolute-git-dir"]))
    }

    /// The worker's plant: its `index` replaced by a link to `target`.
    fn link_index(&self, target: &Path) {
        let index = self.admin().join("index");
        let _ = std::fs::remove_file(&index);
        std::os::unix::fs::symlink(target, &index).unwrap();
    }

    /// Neither victim was written, and the user's checkout shows nothing staged.
    fn assert_untouched(&self, what: &str) {
        assert_eq!(
            std::fs::read(&self.user_index).unwrap(),
            self.user_index_bytes,
            "{what}: the user's index was written"
        );
        assert!(
            !self.missing.exists() && !self.missing.parent().unwrap().exists(),
            "{what}: a file was created through the link"
        );
        assert_eq!(
            out(&self.repo.root, &["status", "--porcelain"]),
            "",
            "{what}: the user's checkout changed"
        );
        assert_eq!(out(&self.repo.root, &["rev-parse", "main"]), self.base);
    }

    fn victims(&self) -> [PathBuf; 2] {
        [self.user_index.clone(), self.missing.clone()]
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

/// The reproduction the re-review ran against the real engine: salvage with the index
/// linked to a victim. It salvages the worktree's files (the index is rebuilt from
/// `HEAD`), and writes neither victim.
#[test]
fn a_salvage_never_writes_through_a_linked_index() {
    for (n, target) in [0, 1].into_iter().enumerate() {
        let w = world(&format!("ix0{n}"));
        commit_file(&w.task, "a.txt", "committed\n", "task work");
        write(&w.task, "b.txt", "uncommitted\n");
        let victim = &w.victims()[target];
        w.link_index(victim);

        let reference = format!("refs/anthrex/salvage/{}/t1/1", w.run);
        let result = salvage(real_git(), &w.task, &reference, "salvage", T);
        w.assert_untouched(&format!("salvage, link to {}", victim.display()));
        assert_eq!(result.unwrap().as_deref(), Some(reference.as_str()));
        let files = out(&w.repo.root, &["ls-tree", "--name-only", &reference]);
        assert!(
            files.contains("a.txt\nb.txt\n"),
            "the salvage saved the worktree: {files}"
        );
        let meta = std::fs::symlink_metadata(w.admin().join("index")).unwrap();
        assert!(
            meta.is_file(),
            "the link was replaced by the engine's index"
        );
    }
}

/// The hand-back, clean and conflicted: refused, and no victim written.
#[test]
fn a_hand_back_never_writes_through_a_linked_index() {
    for (n, (conflict, target)) in [(false, 0), (false, 1), (true, 0), (true, 1)]
        .into_iter()
        .enumerate()
    {
        let w = world(&format!("ix1{n}"));
        let run_head = if conflict {
            conflict_case(&w)
        } else {
            clean_case(&w)
        };
        let victim = &w.victims()[target];
        w.link_index(victim);
        let result = hand_back(real_git(), &w.task, &run_head, T);
        w.assert_untouched(&format!("hand-back (conflict {conflict}): {result:?}"));
        let err = result.unwrap_err();
        assert!(err.contains("index"), "{err}");
    }
}

/// An abort of a conflicted hand-back's merge, and a re-point: no victim written.
#[test]
fn an_abort_or_a_re_point_never_writes_through_a_linked_index() {
    for target in [0, 1] {
        let w = world(&format!("ix2{target}"));
        let run_head = conflict_case(&w);
        let back = hand_back(real_git(), &w.task, &run_head, T).unwrap();
        assert_eq!(back.files, ["f.txt"]);
        let victim = &w.victims()[target];
        w.link_index(victim);
        let result = abort_merge(real_git(), &w.task, T);
        w.assert_untouched(&format!("abort: {result:?}"));
        assert!(result.is_err());
    }
    for target in [0, 1] {
        let w = world(&format!("ix3{target}"));
        // No commit of the task's own, and a newer run head: a re-point.
        let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
        let victim = &w.victims()[target];
        w.link_index(victim);
        let result = prepare_worktree(
            real_git(),
            &w.repo.root,
            &format!("anthrex/{}/t1", w.run),
            &run_head,
            &w.task,
            T,
        );
        w.assert_untouched(&format!("re-point: {result:?}"));
        assert!(result.is_err());
    }
}

/// The race: a link planted after every engine check, while each engine git command
/// in the checkout starts (what a still-running worker process could do). Git works
/// on the engine's own copy, and the copy is installed by rename, so the link is
/// replaced, never written through, whatever the operations return.
#[test]
fn a_link_planted_after_the_checks_is_never_written_through() {
    for (n, op) in ["salvage", "clean", "conflict", "re-point"]
        .into_iter()
        .enumerate()
    {
        let w = world(&format!("ix4{n}"));
        let run_head = match op {
            "clean" => clean_case(&w),
            "conflict" => conflict_case(&w),
            "re-point" => commit_file(&w.integration, "r.txt", "run\n", "run work"),
            _ => {
                commit_file(&w.task, "a.txt", "committed\n", "task work");
                write(&w.task, "b.txt", "uncommitted\n");
                String::new()
            }
        };
        let tools = tempfile::tempdir().unwrap();
        let marker = tools.path().join("planted");
        let git = wrapper_git(
            tools.path(),
            &format!(
                r#"case " $* " in
  *" add "*|*" write-tree"*|*" read-tree "*|*" update-index "*)
    : > '{marker}'
    rm -f '{index}'
    ln -s '{victim}' '{index}' ;;
esac"#,
                marker = marker.display(),
                index = w.admin().join("index").display(),
                victim = w.user_index.display(),
            ),
        );
        let result = match op {
            "salvage" => salvage(
                git.as_os_str(),
                &w.task,
                &format!("refs/anthrex/salvage/{}/t1/1", w.run),
                "salvage",
                T,
            )
            .map(|_| ()),
            "re-point" => prepare_worktree(
                git.as_os_str(),
                &w.repo.root,
                &format!("anthrex/{}/t1", w.run),
                &run_head,
                &w.task,
                T,
            )
            .map(|_| ()),
            _ => hand_back(git.as_os_str(), &w.task, &run_head, T).map(|_| ()),
        };
        assert!(marker.exists(), "{op}: the plant never ran");
        w.assert_untouched(&format!("{op} with a racing link: {result:?}"));
    }
}
