//! Milestone 9.1 task M9.1.17: `OpKind::Propagate` through `driver/stage_ops.rs`,
//! against the parent module's temporary repository: the merge of stage 1's head into
//! stage 2 with both parents, both refs moved at once when stage 2 is the highest, the
//! check judging the merged tree first, and a conflict whose tree is exactly what the
//! sync task's worktree holds after M8a's hand-back.

use std::collections::BTreeSet;

use super::{INTEGRATION, Rig, STAGE_1, STAGE_2, git};
use crate::run::engine::{OpKind, OpResult};
use crate::run::git::{RefreshedIn, hand_back, prepare_worktree, verify_done_spilling};
use crate::run::globs::{OwnsMatcher, ProtectedMatcher};
use crate::run::model::PropagateSpec;

impl Rig {
    /// Commits `file` with `content` on `branch`, from its head, and returns the commit.
    fn commit_on(&self, branch: &str, file: &str, content: &str) -> String {
        let wt = self
            .root
            .join(format!("../wt-{}", branch.replace('/', "-")));
        let at = wt.to_string_lossy().to_string();
        git(
            &self.root,
            &["worktree", "add", "-q", "--detach", &at, branch],
        );
        std::fs::create_dir_all(wt.join(file).parent().unwrap()).unwrap();
        std::fs::write(wt.join(file), content).unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["commit", "-q", "-m", &format!("{file} on {branch}")]);
        let head = git(&wt, &["rev-parse", "HEAD"]);
        git(&self.root, &["worktree", "remove", "--force", &at]);
        git(
            &self.root,
            &["update-ref", &format!("refs/heads/{branch}"), &head],
        );
        head
    }

    /// Stage 1 into stage 2 at the current heads, every run ref guarded where it is.
    fn propagate(&self, check: Option<&str>) -> OpKind {
        let guarded = [STAGE_1, STAGE_2, INTEGRATION]
            .map(|b| (b.to_string(), self.head(b)))
            .to_vec();
        OpKind::Propagate(Box::new(PropagateSpec {
            root: self.root.clone(),
            integration: self.integration.clone(),
            from: 1,
            to: 2,
            from_head: self.head(STAGE_1),
            to_branch: STAGE_2.into(),
            expected_to_head: self.head(STAGE_2),
            also_integration: true,
            base_branch: "main".into(),
            expected_base: self.base.clone(),
            guarded,
            message: "anthrex: propagate stage-1 into stage-2".into(),
            tier: None,
            check: check.map(str::to_string),
            timeout_secs: 30,
            env: Vec::new(),
            tasks: BTreeSet::from(["t1".to_string()]),
        }))
    }

    async fn run_propagate(&self, kind: OpKind) -> OpResult {
        super::super::propagate(&self.service, &self.ctx, 3, kind)
            .await
            .unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn propagate_merges_with_both_parents_and_moves_both_refs_at_once() {
    let rig = Rig::new();
    let one = rig.commit_on(STAGE_1, "s1.txt", "one\n");
    // `integration` is the alias of stage 2, the highest stage.
    let two = rig.commit_on(STAGE_2, "s2.txt", "two\n");
    git(
        &rig.root,
        &["update-ref", &format!("refs/heads/{INTEGRATION}"), &two],
    );
    git(
        &rig.integration,
        &["checkout", "-q", "--force", INTEGRATION],
    );
    // The check judges the merged tree: it passes only with both stages' files.
    let check = "test -f s1.txt && test -f s2.txt";
    let OpResult::Merged { commit, tier } = rig.run_propagate(rig.propagate(Some(check))).await
    else {
        panic!("not merged")
    };
    assert_eq!(tier, None);
    let parents = git(&rig.root, &["rev-list", "--parents", "-n", "1", &commit]);
    assert_eq!(parents, format!("{commit} {two} {one}"));
    assert_eq!(rig.head(STAGE_2), commit);
    assert_eq!(rig.head(INTEGRATION), commit);
    assert_eq!(rig.head(STAGE_1), one, "the lower stage never moves");
    rig.assert_on_integration();
    assert!(rig.integration.join("s1.txt").exists());

    // A red check moves nothing and leaves the integration worktree on its branch.
    let before = rig.head(STAGE_2);
    rig.commit_on(STAGE_1, "s1b.txt", "one more\n");
    let red = rig.run_propagate(rig.propagate(Some("false"))).await;
    assert!(matches!(red, OpResult::CandidateRed { .. }), "{red:?}");
    assert_eq!(rig.head(STAGE_2), before);
    assert_eq!(rig.head(INTEGRATION), before);
    rig.assert_on_integration();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_conflicted_propagate_gives_the_tree_its_sync_worktree_holds() {
    let rig = Rig::new();
    rig.commit_on(STAGE_1, "b.txt", "stage one's other work\n");
    let one = rig.commit_on(STAGE_1, "a.txt", "stage one\n");
    let two = rig.commit_on(STAGE_2, "a.txt", "stage two\n");
    let OpResult::Conflict { files, tree } = rig.run_propagate(rig.propagate(None)).await else {
        panic!("no conflict")
    };
    assert_eq!(files, ["a.txt"]);
    let tree = tree.expect("the conflicted tree");
    assert_eq!(rig.head(STAGE_2), two, "nothing landed");
    // The sync task's worktree starts at stage 2's head, and M8a's hand-back merges
    // stage 1's head into it: its first commit's tree is the conflicted tree.
    let path = rig.root.join("../wt/runs/r1/fix1");
    let git_bin = rig.service.git();
    let t = std::time::Duration::from_secs(30);
    prepare_worktree(&git_bin, &rig.root, "anthrex/r1/fix1", &two, &path, t).unwrap();
    let back = hand_back(&git_bin, &path, &one, t).unwrap();
    assert_eq!(back.files, ["a.txt"]);
    let text = std::fs::read_to_string(path.join("a.txt")).unwrap();
    assert!(
        text.contains("<<<<<<<") && text.contains(">>>>>>>"),
        "{text}"
    );
    git(&path, &["add", "-A"]);
    assert_eq!(git(&path, &["write-tree"]), tree);

    // The worker resolves `a.txt` and commits. Decision 51: its spill check is against
    // the conflicted tree, so stage 1's `b.txt`, merged in, is not the task's change.
    std::fs::write(path.join("a.txt"), "stage one\nstage two\n").unwrap();
    git(&path, &["add", "-A"]);
    git(
        &path,
        &[
            "-c",
            "user.name=W",
            "-c",
            "user.email=w@x",
            "commit",
            "-q",
            "--no-edit",
        ],
    );
    let owns = vec!["a.txt".to_string()];
    let (generated, protected) = (
        OwnsMatcher::new(&[]).unwrap(),
        ProtectedMatcher::new(&[]).unwrap(),
    );
    let done = |spill: Option<&str>| {
        let refreshed = RefreshedIn::default();
        let matchers = (&generated, &protected);
        verify_done_spilling(
            &git_bin,
            &path,
            &two,
            &two,
            &owns,
            matchers,
            None,
            (&refreshed, spill),
            t,
        )
        .unwrap()
    };
    let checked = done(Some(tree.as_str()));
    assert!(
        checked.outside_owns.is_empty(),
        "{:?}",
        checked.outside_owns
    );
    assert!(!checked.merge_in_progress);
    // Without it, the three-dot range counts stage 1's work as the task's.
    assert_eq!(done(None).outside_owns, ["b.txt"]);
}

/// Controller ruling C-22 (2): a propagate whose lower head stage 2 already holds (here
/// after a rebaseline) writes nothing: no `commit-tree -p X -p X`, no object, no ref.
#[tokio::test(flavor = "multi_thread")]
async fn a_propagate_stage_2_already_holds_writes_nothing() {
    let rig = Rig::new();
    let one = rig.commit_on(STAGE_1, "s1.txt", "one\n");
    let refname = format!("refs/heads/{STAGE_2}");
    // Equal heads first, then stage 2 ahead of stage 1.
    git(&rig.root, &["update-ref", &refname, &one]);
    for ahead in [false, true] {
        if ahead {
            rig.commit_on(STAGE_2, "s2.txt", "two\n");
        }
        // `integration` is the alias of stage 2, the highest stage.
        let alias = format!("refs/heads/{INTEGRATION}");
        git(&rig.root, &["update-ref", &alias, &rig.head(STAGE_2)]);
        git(
            &rig.integration,
            &["checkout", "-q", "--force", INTEGRATION],
        );
        let before = (rig.head(STAGE_2), rig.head(INTEGRATION));
        let objects = git(&rig.root, &["count-objects", "-v"]);
        let result = rig.run_propagate(rig.propagate(Some("true"))).await;
        assert_eq!(result, OpResult::AlreadyHeld, "ahead: {ahead}");
        assert_eq!((rig.head(STAGE_2), rig.head(INTEGRATION)), before);
        assert_eq!(git(&rig.root, &["count-objects", "-v"]), objects);
    }
}

// Controller ruling C-21: a sync task's claim and review, in a file of its own.
#[path = "stage_ops_sync_tests.rs"]
mod sync_tests;
