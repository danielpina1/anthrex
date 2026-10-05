//! Milestone 9.6 decision 23 (task M9.6.12): the documents commit's git plumbing,
//! `run::git::commit_docs`, against temporary repositories with a repository-local
//! identity. The tree is built through the driver's own temporary index (never the
//! user's, never a checkout's), every path passes git's `verify_path` under
//! `core.protectHFS` and `core.protectNTFS`, a folder that goes through a tracked
//! symbolic link is refused, the run branch moves by compare-and-swap, and the
//! integration worktree is put back on it. Accept then carries the documents onto the
//! base branch, and a discard leaves them on no branch.

mod support;

use daemon::run::git::{
    AcceptOutcome, DocsCommit, DocsOutcome, accept, commit_docs, create_run_branch,
    delete_branches, docs_symlink,
};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wt_dir};

const RUN: &str = "dc01";
const SPEC: &str = "# Password reset\n\n## Requirements\nR1 Tokens expire.\n";
const PLAN: &str = "# Plan: reset\n\n## Stage 1\n";

/// A repository with a base commit, the run branch `anthrex/dc01/integration` at it
/// and its integration worktree, and a data directory for the driver's index.
struct Rig {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    data: tempfile::TempDir,
    integration: PathBuf,
    base: String,
}

impl Rig {
    fn new() -> Rig {
        Rig::with(|_| {})
    }

    /// [`Rig::new`], `before` run in the repository ahead of its base commit.
    fn with(before: impl FnOnce(&Path)) -> Rig {
        let repo = repo();
        commit_file(&repo.root, "README", "readme\n", "first");
        before(&repo.root);
        out(&repo.root, &["add", "-A"]);
        out(&repo.root, &["commit", "-q", "--allow-empty", "-m", "base"]);
        let base = head(&repo.root);
        let (wt, wt_path) = wt_dir();
        let integration = wt_path.join(format!("runs/{RUN}/integration"));
        create_run_branch(real_git(), &repo.root, &branch(), &base, &integration, T).unwrap();
        Rig {
            repo,
            _wt: wt,
            data: tempfile::tempdir().unwrap(),
            integration,
            base,
        }
    }

    fn root(&self) -> &Path {
        &self.repo.root
    }

    fn index(&self) -> PathBuf {
        self.data.path().join("design/commit.index")
    }

    fn commit(&self, files: &[(&str, &str)]) -> Result<DocsOutcome, String> {
        self.commit_with(real_git(), &self.base, files)
    }

    fn commit_with(
        &self,
        git: &std::ffi::OsStr,
        expected: &str,
        files: &[(&str, &str)],
    ) -> Result<DocsOutcome, String> {
        self.commit_appending(git, expected, files, &[])
    }

    fn commit_appending(
        &self,
        git: &std::ffi::OsStr,
        expected: &str,
        files: &[(&str, &str)],
        appends: &[(String, Vec<u8>)],
    ) -> Result<DocsOutcome, String> {
        let files: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec()))
            .collect();
        let index = self.index();
        let branch = branch();
        let docs = DocsCommit {
            root: self.root(),
            index: &index,
            branch: &branch,
            expected,
            integration: &self.integration,
            files: &files,
            appends,
            stage: None,
            message: "docs: spec and plan for Reset passwords",
        };
        commit_docs(git, &docs, T)
    }

    fn branch_head(&self) -> String {
        out(
            self.root(),
            &["rev-parse", &format!("refs/heads/{}", branch())],
        )
    }
}

fn branch() -> String {
    format!("anthrex/{RUN}/integration")
}

fn committed(outcome: Result<DocsOutcome, String>) -> String {
    match outcome {
        Ok(DocsOutcome::Committed { head, reattach, .. }) => {
            assert_eq!(reattach, None, "the integration worktree went back");
            head
        }
        other => panic!("not committed: {other:?}"),
    }
}

/// The files and the message land in one commit on the run branch, whose only parent is
/// the run head; the integration worktree is on the branch and has the files; the
/// user's checkout, its index and the base branch are untouched; the temporary index is
/// gone.
#[test]
fn the_documents_are_one_commit_on_the_run_branch() {
    let rig = Rig::new();
    let user_index = std::fs::read(rig.root().join(".git/index")).unwrap();
    let new = committed(rig.commit(&[
        ("docs/anthrex/specs/2026-10-05-password-reset.md", SPEC),
        ("docs/anthrex/plans/2026-10-05-password-reset.md", PLAN),
    ]));
    assert_eq!(rig.branch_head(), new);
    let parents = out(rig.root(), &["rev-list", "--parents", "-n", "1", &new]);
    assert_eq!(
        parents,
        format!("{new} {}", rig.base),
        "one parent: the run head"
    );
    let show = |path: &str| out(rig.root(), &["show", &format!("{new}:{path}")]);
    assert_eq!(
        show("docs/anthrex/specs/2026-10-05-password-reset.md"),
        SPEC.trim()
    );
    assert_eq!(
        show("docs/anthrex/plans/2026-10-05-password-reset.md"),
        PLAN.trim()
    );
    assert_eq!(show("README"), "readme", "the run head's tree is kept");
    let message = out(rig.root(), &["log", "-1", "--format=%B", &new]);
    assert_eq!(message, "docs: spec and plan for Reset passwords");
    let changed = out(
        rig.root(),
        &["diff-tree", "--no-commit-id", "--name-only", "-r", &new],
    );
    assert_eq!(
        changed,
        "docs/anthrex/plans/2026-10-05-password-reset.md\ndocs/anthrex/specs/2026-10-05-password-reset.md"
    );

    let on = out(&rig.integration, &["symbolic-ref", "HEAD"]);
    assert_eq!(on, format!("refs/heads/{}", branch()));
    let on_disk = rig
        .integration
        .join("docs/anthrex/specs/2026-10-05-password-reset.md");
    assert_eq!(std::fs::read_to_string(on_disk).unwrap(), SPEC);

    assert_eq!(head(rig.root()), rig.base, "the base branch did not move");
    assert_eq!(
        std::fs::read(rig.root().join(".git/index")).unwrap(),
        user_index,
        "the user's index is untouched"
    );
    assert_eq!(out(rig.root(), &["status", "--porcelain"]), "");
    assert!(!rig.index().exists(), "the temporary index is removed");
}

/// Ruling WB-B-I2: the commit never replaces a tracked file. The run head tracks the
/// spec's name and the plan's `-2`: every document takes the first suffix free for all
/// of them, `-3`, the tracked blobs are kept, and the outcome names the paths used.
#[test]
fn the_documents_take_the_first_suffix_free_for_all_of_them() {
    let spec = "docs/anthrex/specs/2026-10-05-password-reset.md";
    let plan = "docs/anthrex/plans/2026-10-05-password-reset.md";
    let rig = Rig::with(|root| {
        for (path, text) in [
            (spec, "old spec\n"),
            (&plan.replace(".md", "-2.md")[..], "old plan\n"),
        ] {
            let at = root.join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, text).unwrap();
        }
    });
    let outcome = rig.commit(&[(spec, SPEC), (plan, PLAN)]).unwrap();
    let DocsOutcome::Committed {
        head: new, files, ..
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    let third = |p: &str| p.replace(".md", "-3.md");
    assert_eq!(files, [third(spec), third(plan)]);
    let show = |path: &str| out(rig.root(), &["show", &format!("{new}:{path}")]);
    assert_eq!(show(spec), "old spec");
    assert_eq!(show(&plan.replace(".md", "-2.md")), "old plan");
    assert_eq!(show(&third(spec)), SPEC.trim());
    assert_eq!(show(&third(plan)), PLAN.trim());
    let again = committed(rig.commit(&[(spec, SPEC), (plan, PLAN)]));
    assert_eq!(again, new, "sent again: the same names, its own commit");
}

/// Ruling WB-B m1 (the final fix wave's FW-40): the spec a later round appends to is
/// read as raw bytes, up to 1 MiB. One over git's default 256 KiB output commits, a
/// byte that is not UTF-8 kept as it was; one over 1 MiB is refused with its text.
#[test]
fn an_earlier_spec_is_appended_as_raw_bytes_up_to_1_mib() {
    let spec = "docs/anthrex/specs/2026-10-05-password-reset.md";
    let amendment = b"## Round 2 amendment\n\nR3 Links expire sooner.\n".to_vec();
    let mut earlier = b"# Password reset\n\xff not UTF-8\n".to_vec();
    earlier.extend(b"R1 Tokens expire.\n".repeat(300 * 1024 / 18));
    assert!(earlier.len() > 256 * 1024 && earlier.len() < 1024 * 1024);
    let written = earlier.clone();
    let rig = Rig::with(move |root| {
        std::fs::create_dir_all(root.join("docs/anthrex/specs")).unwrap();
        std::fs::write(root.join(spec), &written).unwrap();
    });
    let appends = [(spec.to_string(), amendment.clone())];
    let new = committed(rig.commit_appending(real_git(), &rig.base, &[], &appends));
    let blob = try_git(rig.root(), &["cat-file", "blob", &format!("{new}:{spec}")]);
    assert!(blob.status.success());
    let mut expected = earlier;
    expected.extend(b"\n");
    expected.extend(&amendment);
    assert_eq!(
        blob.stdout, expected,
        "the earlier bytes, then the amendment"
    );

    let mut over = b"# Password reset\n".to_vec();
    over.resize(1024 * 1024 + 1, b'x');
    let rig = Rig::with(move |root| {
        std::fs::create_dir_all(root.join("docs/anthrex/specs")).unwrap();
        std::fs::write(root.join(spec), &over).unwrap();
    });
    let refused = rig.commit_appending(real_git(), &rig.base, &[], &appends);
    let text = format!("the committed spec {spec} is over 1 MiB (1048577 bytes)");
    assert_eq!(refused, Err(text));
    assert_eq!(rig.branch_head(), rig.base, "nothing committed");
}

/// A commit sent again after a restart, whose first send moved the branch, finds its
/// own commit (parent the run head, the same tree) and answers with it.
#[test]
fn a_commit_sent_again_finds_its_own_commit() {
    let rig = Rig::new();
    let files = [("docs/specs/a.md", SPEC), ("docs/plans/a.md", PLAN)];
    let first = committed(rig.commit(&files));
    let again = committed(rig.commit(&files));
    assert_eq!(again, first);
    assert_eq!(rig.branch_head(), first);
}

/// Review m2 (ruling T12-3): a commit with the same tree on the run head but another
/// message is someone else's, not this commit's: the branch moved.
#[test]
fn a_commit_of_the_same_tree_with_another_message_is_not_adopted() {
    let rig = Rig::new();
    let files = [("docs/specs/a.md", SPEC)];
    let ours = committed(rig.commit(&files));
    let tree = out(rig.root(), &["rev-parse", &format!("{ours}^{{tree}}")]);
    let theirs = out(
        rig.root(),
        &[
            "commit-tree",
            &tree,
            "-p",
            &rig.base,
            "-m",
            "a user's commit",
        ],
    );
    let refname = format!("refs/heads/{}", branch());
    out(rig.root(), &["update-ref", &refname, &theirs]);
    match rig.commit(&files) {
        Err(error) => assert!(error.contains("moved"), "{error}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(rig.branch_head(), theirs);
}

/// Review m5 (ruling T12-3): a run branch git cannot read is an error with git's text,
/// not a branch that does not exist.
#[test]
fn an_unreadable_run_branch_is_an_error_not_a_missing_branch() {
    let rig = Rig::new();
    let file = rig.root().join(format!(".git/refs/heads/{}", branch()));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "not a sha\n").unwrap();
    match rig.commit(&[("docs/specs/a.md", SPEC)]) {
        Err(error) => {
            assert!(!error.contains("does not exist"), "{error}");
            assert!(error.contains("rev-parse"), "{error}");
        }
        other => panic!("{other:?}"),
    }
}

/// A branch moved by anyone else is a failure that names it; nothing is written.
#[test]
fn a_run_branch_moved_elsewhere_fails() {
    let rig = Rig::new();
    let other = commit_file(&rig.integration, "x.txt", "x\n", "someone else");
    let error = match rig.commit(&[("docs/specs/a.md", SPEC)]) {
        Err(error) => error,
        other => panic!("{other:?}"),
    };
    assert!(
        error.contains(&format!("refs/heads/{}", branch())) && error.contains("moved"),
        "{error}"
    );
    assert_eq!(rig.branch_head(), other);
}

/// The ruling's path safety: a folder whose component is a tracked symbolic link in
/// the run head's tree is refused, naming it, before anything is written.
#[test]
fn a_folder_through_a_tracked_symlink_is_refused() {
    let rig = Rig::with(|root| {
        std::fs::create_dir_all(root.join("elsewhere")).unwrap();
        std::fs::write(root.join("elsewhere/keep"), "k\n").unwrap();
        std::os::unix::fs::symlink("elsewhere", root.join("docs")).unwrap();
    });
    let outcome = rig.commit(&[("docs/anthrex/specs/a.md", SPEC)]);
    assert_eq!(
        outcome,
        Ok(DocsOutcome::Symlink {
            path: "docs".into()
        })
    );
    assert_eq!(rig.branch_head(), rig.base);
    // A deeper link is found too.
    let rig = Rig::with(|root| {
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::os::unix::fs::symlink("/tmp", root.join("docs/anthrex")).unwrap();
    });
    let outcome = rig.commit(&[("docs/anthrex/specs/a.md", SPEC)]);
    assert_eq!(
        outcome,
        Ok(DocsOutcome::Symlink {
            path: "docs/anthrex".into()
        })
    );
}

/// Review m3 (ruling T12-3): the walk compares path components ignoring case, so a
/// tracked `Docs` link is found for `docs` (macOS's `core.ignorecase` would write
/// through it in a checkout), and the start check (ruling T12-1) finds the same.
#[test]
fn a_symlink_that_differs_only_in_case_is_refused() {
    let rig = Rig::with(|root| {
        std::fs::create_dir_all(root.join("elsewhere")).unwrap();
        std::fs::write(root.join("elsewhere/keep"), "k\n").unwrap();
        std::os::unix::fs::symlink("elsewhere", root.join("Docs")).unwrap();
    });
    let outcome = rig.commit(&[("docs/anthrex/specs/a.md", SPEC)]);
    assert_eq!(
        outcome,
        Ok(DocsOutcome::Symlink {
            path: "Docs".into()
        })
    );
    let folders = ["docs/anthrex/specs".to_string()];
    let found = docs_symlink(real_git(), rig.root(), &rig.base, &folders, T).unwrap();
    assert_eq!(found.as_deref(), Some("Docs"));
    let plain = Rig::new();
    let found = docs_symlink(real_git(), plain.root(), &plain.base, &folders, T).unwrap();
    assert_eq!(found, None);
}

/// Git's own `verify_path`, with `core.protectHFS` and `core.protectNTFS`, refuses a
/// path HFS+ or NTFS would read as `.git` (a zero-width non-joiner, a case change):
/// a failure, not a commit.
#[test]
fn hfs_and_ntfs_spellings_of_git_are_refused() {
    let rig = Rig::new();
    for path in [
        "docs/.g\u{200c}it/x.md",
        "docs/.GIT/x.md",
        "docs/git~1/x.md",
    ] {
        match rig.commit(&[(path, SPEC)]) {
            Err(error) => assert!(error.contains("update-index"), "{path}: {error}"),
            other => panic!("{path}: {other:?}"),
        }
        assert_eq!(rig.branch_head(), rig.base, "{path}");
        assert!(
            !rig.index().exists(),
            "{path}: the temporary index is removed"
        );
    }
}

/// Accept merges the run branch, so the documents reach the base branch; a discard
/// deletes the run's branches, so they are on no branch.
#[test]
fn discard_drops_the_documents_and_accept_merges_them() {
    let rig = Rig::new();
    let new = committed(rig.commit(&[("docs/anthrex/specs/a.md", SPEC)]));
    let accepted = accept(
        real_git(),
        rig.root(),
        "main",
        &rig.base,
        &branch(),
        &new,
        "anthrex: accept run dc01",
        T,
    )
    .unwrap();
    assert!(
        matches!(accepted, AcceptOutcome::Merged { .. }),
        "{accepted:?}"
    );
    let on_main = out(rig.root(), &["show", "main:docs/anthrex/specs/a.md"]);
    assert_eq!(on_main, SPEC.trim());

    let rig = Rig::new();
    let new = committed(rig.commit(&[("docs/anthrex/specs/a.md", SPEC)]));
    out(
        rig.root(),
        &[
            "worktree",
            "remove",
            "-f",
            "-f",
            rig.integration.to_str().unwrap(),
        ],
    );
    delete_branches(real_git(), rig.root(), &format!("anthrex/{RUN}/"), T).unwrap();
    let holding = out(rig.root(), &["branch", "--contains", &new]);
    assert_eq!(holding, "", "no branch holds the documents");
    let shown = try_git(rig.root(), &["show", "main:docs/anthrex/specs/a.md"]);
    assert!(!shown.status.success());
}
