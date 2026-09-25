//! Final fix batch F1b: a worker's commits reach the repository only through the
//! engine's import. The worker's git (here plain git with the worker's environment:
//! `GIT_OBJECT_DIRECTORY` its private directory, the common store its alternate) writes
//! its objects outside the repository and commits on a detached `HEAD`; the engine
//! imports what its `HEAD` reaches, re-hashing every object, and records it on the
//! task's branch. An object whose content does not match its name is refused, a
//! tampered private directory is refused, an interrupted import is finished by the
//! next, and the hand-back, the salvage and the done check all work on the imported
//! commits while the worker keeps working through its alternate.

mod support;

use daemon::run::git::{
    DoneChecked, hand_back, prepare_task_worktree, private_dir, salvage, sync, verify_done,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::plan::BUILTIN_PROTECTED;
use daemon::run::role_launch::task_objects_dir;
use std::path::PathBuf;
use std::process::Command;
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wrapper_git, wt_dir};

struct World {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    _data: tempfile::TempDir,
    run: String,
    common: PathBuf,
    objects: PathBuf,
    task: PathBuf,
    base: String,
}

fn world(run: &str) -> World {
    let repo = repo();
    commit_file(&repo.root, "f.txt", "base\n", "f");
    let base = commit_file(&repo.root, "g.txt", "one\ntwo\nthree\nfour\nfive\n", "g");
    let (wt, wt_path) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let common = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap();
    let objects = private_dir(&common, &task_objects_dir(data.path(), "t1")).unwrap();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare_task_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        Some(&objects),
        T,
    )
    .unwrap();
    World {
        repo,
        _wt: wt,
        _data: data,
        run: run.to_string(),
        common,
        objects,
        task,
        base,
    }
}

impl World {
    /// Git as the worker runs it: its objects to its private directory.
    fn worker(&self, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args([
                "-c",
                "user.name=w",
                "-c",
                "user.email=w@w",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(&self.task)
            .env("GIT_OBJECT_DIRECTORY", &self.objects)
            .env(
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                self.common.join("objects"),
            )
            .output()
            .unwrap()
    }

    fn worker_ok(&self, args: &[&str]) -> String {
        let output = self.worker(args);
        assert!(
            output.status.success(),
            "worker git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    /// The worker writes `path` and commits it; its new `HEAD`.
    fn commit(&self, path: &str, content: &str) -> String {
        support::run_git::write(&self.task, path, content);
        self.worker_ok(&["add", "--", path]);
        self.worker_ok(&["commit", "-q", "-m", path]);
        self.worker_ok(&["rev-parse", "HEAD"])
    }

    fn own(&self) -> String {
        format!("refs/heads/anthrex/{}/t1", self.run)
    }

    fn branch(&self) -> String {
        out(&self.repo.root, &["rev-parse", &self.own()])
    }

    /// Whether the repository itself (the common store, no alternate) has `id`.
    fn has(&self, id: &str) -> bool {
        try_git(&self.repo.root, &["cat-file", "-e", id])
            .status
            .success()
    }

    fn packs(&self) -> usize {
        std::fs::read_dir(self.common.join("objects/pack"))
            .map(|dir| {
                dir.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "pack"))
                    .count()
            })
            .unwrap_or(0)
    }

    fn loose(&self, id: &str) -> PathBuf {
        self.objects.join(&id[..2]).join(&id[2..])
    }
}

fn done(w: &World) -> DoneChecked {
    let generated = OwnsMatcher::new(&[]).unwrap();
    let protected = ProtectedMatcher::new(
        &BUILTIN_PROTECTED
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    verify_done(
        real_git(),
        &w.task,
        &w.base,
        &w.base,
        &["**".to_string()],
        &generated,
        &protected,
        None,
        T,
    )
    .unwrap()
}

#[test]
fn a_workers_commits_are_imported_and_recorded_once() {
    let w = world("im01");
    w.commit("a.txt", "a\n");
    let tip = w.commit("dir/b.txt", "b\n");
    assert!(!w.has(&tip), "the worker's commit went to the common store");
    assert_eq!(w.branch(), w.base, "the worker moved its branch");

    let r = done(&w);
    assert_eq!((r.commits, r.head.as_str()), (2, tip.as_str()));
    assert_eq!(r.head_branch, Some(format!("anthrex/{}/t1", w.run)));
    assert_eq!(w.branch(), tip);
    assert!(w.has(&tip));
    out(
        &w.repo.root,
        &["fsck", "--no-dangling", "--connectivity-only"],
    );
    assert_eq!(
        out(&w.repo.root, &["show", &format!("{tip}:dir/b.txt")]),
        "b"
    );

    // Idempotent: nothing more is imported or written.
    let packs = w.packs();
    assert_eq!(sync(real_git(), &w.task, T).unwrap(), tip);
    assert_eq!(w.packs(), packs);
    assert_eq!(w.branch(), tip);

    // An amend (history rewritten on the detached `HEAD`) is recorded as it is.
    w.worker_ok(&["commit", "-q", "--amend", "-m", "amended"]);
    let amended = w.worker_ok(&["rev-parse", "HEAD"]);
    assert_eq!(sync(real_git(), &w.task, T).unwrap(), amended);
    assert_eq!(w.branch(), amended);
    assert_eq!(out(&w.repo.root, &["rev-parse", "main"]), w.base);
}

/// Reproduction (c): an object in the private directory whose content does not match
/// its name is never trusted under that name; the import is refused and nothing moves.
#[test]
fn an_object_whose_content_does_not_match_its_name_is_refused() {
    let w = world("im02");
    let tip = w.commit("a.txt", "good\n");
    let good = w.worker_ok(&["rev-parse", "HEAD:a.txt"]);
    // Another valid object's file, planted under the good blob's name.
    let evil = Command::new("sh")
        .args(["-c", "printf 'evil\\n' | git hash-object -w --stdin"])
        .current_dir(&w.task)
        .env("GIT_OBJECT_DIRECTORY", &w.objects)
        .output()
        .unwrap();
    let evil = String::from_utf8(evil.stdout).unwrap().trim().to_string();
    let target = w.loose(&good);
    let mut perms = std::fs::metadata(&target).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&target, perms).unwrap();
    std::fs::copy(w.loose(&evil), &target).unwrap();

    let err = sync(real_git(), &w.task, T).unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(w.branch(), w.base, "the branch moved: {err}");
    assert!(!w.has(&good), "the mismatched object was trusted: {err}");
    assert!(
        !w.has(&tip),
        "a commit was imported without its tree: {err}"
    );
    assert_eq!(out(&w.repo.root, &["rev-parse", "main"]), w.base);
    // The done check refuses the claim the same way.
    let generated = OwnsMatcher::new(&[]).unwrap();
    let protected = ProtectedMatcher::new(&[]).unwrap();
    assert!(
        verify_done(
            real_git(),
            &w.task,
            &w.base,
            &w.base,
            &["**".to_string()],
            &generated,
            &protected,
            None,
            T,
        )
        .is_err()
    );
}

#[test]
fn a_tampered_private_directory_is_refused() {
    let w = world("im03");
    w.commit("a.txt", "a\n");
    // Its own alternates would make the import read another object store.
    let info = w.objects.join("info");
    std::fs::create_dir_all(&info).unwrap();
    std::fs::write(info.join("alternates"), "/elsewhere/objects\n").unwrap();
    let err = sync(real_git(), &w.task, T).unwrap_err();
    assert!(err.contains("tampered"), "{err}");
    std::fs::remove_file(info.join("alternates")).unwrap();

    // Replaced by a link (the worker's grant covers the directory itself).
    let moved = w.objects.with_file_name("moved");
    std::fs::rename(&w.objects, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &w.objects).unwrap();
    let err = sync(real_git(), &w.task, T).unwrap_err();
    assert!(err.contains("tampered"), "{err}");
    assert_eq!(w.branch(), w.base);

    std::fs::remove_file(&w.objects).unwrap();
    std::fs::rename(&moved, &w.objects).unwrap();
    let tip = w.worker_ok(&["rev-parse", "HEAD"]);
    assert_eq!(sync(real_git(), &w.task, T).unwrap(), tip);
}

/// Decision 43: an import interrupted at either step (the daemon died) is finished by
/// the next one, which imports nothing twice.
#[test]
fn an_interrupted_import_is_finished_by_the_next() {
    for step in ["index-pack", "update-ref"] {
        let w = world(&format!("im04{}", &step[..1]));
        let tip = w.commit("a.txt", "a\n");
        let tools = tempfile::tempdir().unwrap();
        let git = wrapper_git(
            tools.path(),
            &format!(r#"case " $* " in *" {step} "*) exit 99;; esac"#),
        );
        assert!(sync(git.as_os_str(), &w.task, T).is_err());
        assert_eq!(w.branch(), w.base, "{step}");
        assert_eq!(w.has(&tip), step == "update-ref", "{step}");
        let packs = w.packs();

        assert_eq!(sync(real_git(), &w.task, T).unwrap(), tip, "{step}");
        assert_eq!(w.branch(), tip, "{step}");
        let imported = usize::from(step == "index-pack");
        assert_eq!(w.packs(), packs + imported, "{step}");
        out(
            &w.repo.root,
            &["fsck", "--no-dangling", "--connectivity-only"],
        );
    }
}

/// The hand-back writes its merge into the repository (the engine's store), and the
/// worker sees it through its alternate: it resolves a conflicted one and commits, and
/// that commit is imported in turn. A clean one moves its `HEAD`, and it works on.
#[test]
fn hand_backs_reach_the_worker_through_its_alternate() {
    let w = world("im05");
    w.commit("g.txt", "one\ntwo\nthree\nfour\nFIVE\n");
    // The run head changes the same line: a conflict.
    let run_head = {
        let side = w.repo.root.join("side");
        out(
            &w.repo.root,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                side.to_str().unwrap(),
                &w.base,
            ],
        );
        commit_file(&side, "g.txt", "one\ntwo\nthree\nfour\nfive!\n", "run")
    };
    let back = hand_back(real_git(), &w.task, &run_head, T).unwrap();
    assert_eq!(back.files, vec!["g.txt".to_string()]);
    support::run_git::write(&w.task, "g.txt", "one\ntwo\nthree\nfour\nresolved\n");
    w.worker_ok(&["add", "g.txt"]);
    w.worker_ok(&["commit", "-q", "--no-edit"]);
    let resolved = w.worker_ok(&["rev-parse", "HEAD"]);
    assert_eq!(w.worker_ok(&["rev-parse", "HEAD^2"]), run_head);
    assert!(!w.has(&resolved));

    let r = done(&w);
    assert_eq!(r.head, resolved);
    assert_eq!(w.branch(), resolved);
    assert_eq!(
        out(&w.repo.root, &["show", &format!("{resolved}:g.txt")]),
        "one\ntwo\nthree\nfour\nresolved"
    );

    // A clean hand-back: the merge is the engine's, and the worker commits on top.
    let later = {
        let side = w.repo.root.join("side");
        commit_file(&side, "h.txt", "h\n", "run again")
    };
    let back = hand_back(real_git(), &w.task, &later, T).unwrap();
    assert!(back.files.is_empty(), "{back:?}");
    assert_eq!(head(&w.task), back.head);
    assert!(w.has(&back.head));
    assert!(
        !w.loose(&back.head).exists(),
        "the engine wrote the private dir"
    );
    let top = w.commit("c.txt", "c\n");
    assert_eq!(w.worker_ok(&["rev-parse", "HEAD^"]), back.head);
    assert_eq!(sync(real_git(), &w.task, T).unwrap(), top);
}

/// The salvage imports the worker's commits and the blobs it staged but never
/// committed (in its private directory only), a rename among them, and saves them all.
/// The done check sees the same staged work as dirty, not as an error.
#[test]
fn staged_work_in_the_private_directory_is_judged_and_salvaged() {
    let w = world("im06");
    let tip = w.commit("a.txt", "a\n");
    // A rename with an edit (an inexact rename, which `status` detects by content), and
    // a new staged file.
    w.worker_ok(&["mv", "g.txt", "moved.txt"]);
    support::run_git::write(&w.task, "moved.txt", "one\ntwo\nthree\nfour\nsix\n");
    support::run_git::write(&w.task, "new.txt", "new\n");
    w.worker_ok(&["add", "-A"]);
    let staged = w.worker_ok(&["rev-parse", ":new.txt"]);
    assert!(!w.has(&staged));

    let r = done(&w);
    assert_eq!(r.head, tip);
    assert!(r.dirty_tracked > 0, "{r:?}");

    let reference = format!("refs/anthrex/salvage/{}/t1", w.run);
    let saved = salvage(real_git(), &w.task, &reference, "salvage", T)
        .unwrap()
        .expect("something to save");
    assert_eq!(out(&w.repo.root, &["rev-parse", &format!("{saved}^")]), tip);
    assert_eq!(
        out(&w.repo.root, &["show", &format!("{saved}:new.txt")]),
        "new"
    );
    assert_eq!(
        out(&w.repo.root, &["show", &format!("{saved}:moved.txt")]),
        "one\ntwo\nthree\nfour\nsix"
    );
    out(
        &w.repo.root,
        &["fsck", "--no-dangling", "--connectivity-only"],
    );
}
