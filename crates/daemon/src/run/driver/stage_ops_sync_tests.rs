//! Controller ruling C-21 on task M9.1.17, on real git: a sync task's claim and review
//! are measured from the conflicted tree its worktree started with. Its signals do not
//! count stage 1's deletions (item 2), a later hand-back of stage 2 is not its spill
//! (item 3), a head that dropped the merge is not kept (item 5), and its reviewer's
//! diff holds only the resolution (item 6).

use std::path::PathBuf;

use super::super::{Rig, STAGE_1, STAGE_2, git};
use crate::run::engine::{OpKind, OpResult};
use crate::run::git::{hand_back, prepare_worktree};
use crate::run::model::SyncCheck;
use crate::run::tiers::{Signal, SignalsSpec};

const T: std::time::Duration = std::time::Duration::from_secs(30);

/// A conflicted sync: stage 1's head, stage 2's head, the conflicted tree, and `fix1`'s
/// worktree at stage 2's head with stage 1's head handed back into it.
struct Sync {
    one: String,
    two: String,
    tree: String,
    path: PathBuf,
}

impl Rig {
    /// Removes `file` on `branch`, from its head.
    fn remove_on(&self, branch: &str, file: &str) -> String {
        let wt = self.root.join("../wt-remove");
        let at = wt.to_string_lossy().to_string();
        git(
            &self.root,
            &["worktree", "add", "-q", "--detach", &at, branch],
        );
        git(&wt, &["rm", "-q", file]);
        git(&wt, &["commit", "-q", "-m", &format!("remove {file}")]);
        let head = git(&wt, &["rev-parse", "HEAD"]);
        git(&self.root, &["worktree", "remove", "--force", &at]);
        let refname = format!("refs/heads/{branch}");
        git(&self.root, &["update-ref", &refname, &head]);
        head
    }

    /// Stage 1 and stage 2 both change `a.txt`, after `before`; the propagate conflicts,
    /// and `fix1`'s worktree holds the merge.
    async fn sync(&self, before: impl FnOnce(&Rig)) -> Sync {
        before(self);
        let one = self.commit_on(STAGE_1, "a.txt", "stage one\n");
        let two = self.commit_on(STAGE_2, "a.txt", "stage two\n");
        let OpResult::Conflict { tree, .. } = self.run_propagate(self.propagate(None)).await else {
            panic!("no conflict")
        };
        let tree = tree.expect("the conflicted tree");
        let path = self.root.join("../wt/runs/r1/fix1");
        let g = self.service.git();
        prepare_worktree(&g, &self.root, "anthrex/r1/fix1", &two, &path, T).unwrap();
        hand_back(&g, &path, &one, T).unwrap();
        Sync {
            one,
            two,
            tree,
            path,
        }
    }

    async fn op(&self, kind: OpKind) -> OpResult {
        crate::run::driver::ops::run(&self.service, &self.ctx, 7, kind).await
    }
}

impl Sync {
    /// The worker resolves `a.txt` keeping both stages, and commits the merge.
    fn resolve(&self) {
        std::fs::write(self.path.join("a.txt"), "stage one\nstage two\n").unwrap();
        commit(&self.path, &["--no-edit"]);
    }

    /// `fix1`'s `VerifyDone`, owning `a.txt`, against `run_head`, its sync check with
    /// `upper`, and `signals`.
    fn verify(&self, run_head: &str, upper: Option<&str>, signals: Option<SignalsSpec>) -> OpKind {
        OpKind::VerifyDone {
            worktree: self.path.clone(),
            start: self.two.clone(),
            run_head: run_head.into(),
            owns: vec!["a.txt".into()],
            generated: Vec::new(),
            protected: Vec::new(),
            spill_exempt: false,
            red: None,
            resolution: None,
            not_own: Vec::new(),
            not_run: Vec::new(),
            signals,
            spill_base: Some(self.tree.clone()),
            sync: Some(Box::new(SyncCheck {
                onto: self.one.clone(),
                to_head: self.two.clone(),
                upper: upper.map(str::to_string),
            })),
        }
    }
}

fn commit(dir: &std::path::Path, extra: &[&str]) {
    git(dir, &["add", "-A"]);
    let mut args = vec!["-c", "user.name=W", "-c", "user.email=w@x", "commit", "-q"];
    args.extend_from_slice(extra);
    git(dir, &args);
}

fn checked(result: &OpResult) -> (Vec<String>, Option<bool>, Vec<Signal>) {
    let OpResult::DoneChecked {
        outside_owns,
        sync_kept,
        signals,
        ..
    } = result
    else {
        panic!("not checked: {result:?}")
    };
    let list = signals.as_ref().map(|s| s.list.clone()).unwrap_or_default();
    (outside_owns.clone(), *sync_kept, list)
}

/// Item 2, the reviewer's `old_test.rs` scenario: stage 1 deleted a test file that
/// stage 2 still has. The sync task's merge carries the deletion, which is stage 1's,
/// not the task's: no `DeletedTestFile`, and the claim passes.
#[tokio::test(flavor = "multi_thread")]
async fn a_sync_claims_signals_start_at_its_conflicted_tree() {
    let rig = Rig::new();
    let sync = rig
        .sync(|rig| {
            let with = rig.commit_on(STAGE_1, "tests/old_test.rs", "fn t() {}\n");
            let refname = format!("refs/heads/{STAGE_2}");
            git(&rig.root, &["update-ref", &refname, &with]);
            rig.remove_on(STAGE_1, "tests/old_test.rs");
        })
        .await;
    sync.resolve();
    let spec = SignalsSpec {
        test_paths: vec!["tests/**".into()],
        skip_markers: Vec::new(),
    };
    let result = rig.op(sync.verify(&sync.two, None, Some(spec))).await;
    let (outside, kept, signals) = checked(&result);
    assert!(outside.is_empty(), "{outside:?}");
    assert_eq!(kept, Some(true));
    assert!(
        !signals
            .iter()
            .any(|s| matches!(s, Signal::DeletedTestFile { .. })),
        "{signals:?}"
    );
}

/// Item 3, the reviewer's `c.txt` scenario: stage 2 moves on after the sync task
/// resolved, and the merge queue hands its head back. `c.txt`, which stage 2's commit
/// changed, is not the task's spill.
#[tokio::test(flavor = "multi_thread")]
async fn a_later_hand_back_of_stage_2_is_not_the_sync_tasks_spill() {
    let rig = Rig::new();
    let sync = rig.sync(|_| {}).await;
    sync.resolve();
    let h2 = rig.commit_on(STAGE_2, "c.txt", "stage two again\n");
    let back = hand_back(&rig.service.git(), &sync.path, &h2, T).unwrap();
    assert!(back.files.is_empty(), "{back:?}");
    let result = rig.op(sync.verify(&h2, Some(&h2), None)).await;
    let (outside, kept, _) = checked(&result);
    assert!(outside.is_empty(), "{outside:?}");
    assert_eq!(kept, Some(true));
    // Without the hand-back recorded, stage 2's `c.txt` would be the task's.
    let result = rig.op(sync.verify(&h2, None, None)).await;
    assert_eq!(checked(&result).0, ["c.txt"]);
}

/// Item 5: a worker that ran `git merge --abort` and committed by hand dropped stage
/// 1's merge; the claim says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_sync_head_without_its_merge_is_not_kept() {
    let rig = Rig::new();
    let sync = rig.sync(|_| {}).await;
    git(&sync.path, &["merge", "--abort"]);
    std::fs::write(sync.path.join("a.txt"), "stage one\nstage two\n").unwrap();
    commit(&sync.path, &["-m", "resolved by hand"]);
    let result = rig.op(sync.verify(&sync.two, None, None)).await;
    assert_eq!(checked(&result).1, Some(false));
}

/// Item 6: the sync reviewer's diff runs from the conflicted tree to the head, so it
/// holds the resolution of `a.txt` and none of stage 1's own `b.txt`.
#[tokio::test(flavor = "multi_thread")]
async fn the_sync_review_diff_starts_at_the_conflicted_tree() {
    let rig = Rig::new();
    let sync = rig
        .sync(|rig| {
            rig.commit_on(STAGE_1, "b.txt", "stage one's other work\n");
        })
        .await;
    sync.resolve();
    rig.op(sync.verify(&sync.two, None, None)).await;
    let review = |base_tree: Option<String>| OpKind::PrepareReview {
        root: rig.root.clone(),
        head_ref: "anthrex/r1/fix1".into(),
        base_ref: sync.two.clone(),
        path: rig.root.join("../wt/runs/r1/fix1.review"),
        base_tree,
    };
    let OpResult::Review { base, patch, .. } = rig.op(review(Some(sync.tree.clone()))).await else {
        panic!("no review")
    };
    assert_eq!(base, sync.tree);
    assert!(patch.contains("a.txt"), "{patch}");
    assert!(!patch.contains("b.txt"), "{patch}");
    // The merge base would show stage 1's work too.
    let OpResult::Review { patch, .. } = rig.op(review(None)).await else {
        panic!("no review")
    };
    assert!(patch.contains("b.txt"), "{patch}");
}
