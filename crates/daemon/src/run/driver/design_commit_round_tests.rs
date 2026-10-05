//! Milestone 9.6 task M9.6.15 (decision 24, DF §8.1): a later round's documents commit
//! through the driver's executor, against a temporary repository with a
//! repository-local identity. Round 1's commit is made first; then round 2's appends
//! its amendment to the committed spec under `## Round 2 amendment`, adds the round's
//! plan as `…-round2.md`, and is the round's first new stage: its stage branch is
//! created at the commit, `integration` follows it, stage 1 stays where it was. Sent
//! again (a restart lost the reply), it finds its own commit.

use super::tests::{PLAN, PLAN_PATH, RUN, Rig, SPEC, SPEC_PATH, branch, git};
use crate::run::design::commit::DocsCommitSpec;
use crate::run::engine::OpResult;

const AMENDMENT: &str = "# Password reset on mobile\n\n## Requirements\nR2 Single use, in the app too.\nR3 The app opens links.\n";
const ROUND_PLAN: &str = "# Plan: Reset passwords (round 2)\n\n## Stage 2\n";
const ROUND_PLAN_PATH: &str = "docs/anthrex/plans/2026-10-05-password-reset-round2.md";
const MESSAGE: &str = "docs: round 2 spec amendment and plan for Reset passwords";

fn stage(n: u16) -> String {
    format!("anthrex/{RUN}/stage-{n}")
}

/// Round 1 committed, and stage 1 created at its head (the widened run's first pass):
/// that head.
fn round_one(rig: &Rig) -> String {
    let head = match rig.run(rig.spec_and_plan()) {
        OpResult::DocsCommitted { head, .. } => head,
        other => panic!("{other:?}"),
    };
    git(&rig.root, &["branch", &stage(1), &head]);
    head
}

/// Round 2's commit spec: the stored amendment appended to round 1's spec, the stored
/// round plan at its own path, as stage 2 from stage 1's head.
fn round_two(rig: &Rig, from: &str) -> DocsCommitSpec {
    let mut amendment = rig.stored("specs", "spec-v2.md", AMENDMENT);
    amendment.repo_path = Some(SPEC_PATH.into());
    amendment.append = Some("## Round 2 amendment".into());
    let mut plan = rig.stored("plans", "plan-v2.md", ROUND_PLAN);
    plan.repo_path = Some(ROUND_PLAN_PATH.into());
    let mut spec = rig.spec(vec![amendment, plan]);
    spec.expected_head = from.to_string();
    spec.stage_branch = Some(stage(2));
    spec.message = MESSAGE.into();
    spec
}

fn head_of(rig: &Rig, branch: &str) -> String {
    git(&rig.root, &["rev-parse", &format!("refs/heads/{branch}")])
}

#[test]
fn the_round_commit_appends_the_amendment_and_a_round_plan() {
    let rig = Rig::new(None);
    let one = round_one(&rig);
    let head = match rig.run(round_two(&rig, &one)) {
        OpResult::DocsCommitted { head, spec } => {
            assert_eq!(spec, SPEC_PATH);
            head
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(head_of(&rig, &stage(2)), head);
    assert_eq!(
        head_of(&rig, &branch()),
        head,
        "integration follows the top stage"
    );
    assert_eq!(head_of(&rig, &stage(1)), one, "stage 1 never moves");
    let parents = git(&rig.root, &["rev-list", "--parents", "-n", "1", &head]);
    assert_eq!(parents, format!("{head} {one}"));
    let spec = format!("{SPEC}\n## Round 2 amendment\n\n{AMENDMENT}");
    assert_eq!(rig.show(&head, SPEC_PATH), spec);
    assert_eq!(rig.show(&head, ROUND_PLAN_PATH), ROUND_PLAN);
    assert_eq!(rig.show(&head, PLAN_PATH), PLAN, "round 1's plan stays");
    let message = git(&rig.root, &["log", "-1", "--format=%B", &head]);
    assert_eq!(message, MESSAGE);
    let on_disk = std::fs::read_to_string(rig.integration.join(SPEC_PATH)).unwrap();
    assert_eq!(on_disk, spec);
    assert!(
        !rig.ctx.data_dir.join("design/commit.index").exists(),
        "the driver's index is removed"
    );
}

/// A restart lost the reply: the same commit sent again finds its own stage, and moves
/// nothing.
#[test]
fn a_round_commit_sent_again_finds_its_own_stage() {
    let rig = Rig::new(None);
    let one = round_one(&rig);
    let first = rig.run(round_two(&rig, &one));
    let again = rig.run(round_two(&rig, &one));
    assert_eq!(first, again);
    let OpResult::DocsCommitted { head, .. } = again else {
        panic!("{again:?}");
    };
    assert_eq!(head_of(&rig, &stage(2)), head);
    assert_eq!(head_of(&rig, &branch()), head);
}

/// `integration` moved under the run (not at the stage below's head): nothing is made,
/// and the stage branch is not created.
#[test]
fn a_moved_integration_branch_fails_the_round_commit() {
    let rig = Rig::new(None);
    let one = round_one(&rig);
    git(
        &rig.root,
        &["update-ref", &format!("refs/heads/{}", branch()), &rig.base],
    );
    match rig.run(round_two(&rig, &one)) {
        OpResult::Failed { message } => {
            assert!(message.contains("moved from"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let missing = std::process::Command::new("git")
        .arg("-C")
        .arg(&rig.root)
        .args([
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{}", stage(2)),
        ])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap();
    assert!(!missing.status.success(), "no stage 2");
}

/// Hard rule 11 and the addendum for a later round's commit too: every git call it
/// makes (the spec's `ls-tree` and `cat-file`, the stage's `update-ref --stdin`)
/// carries `--no-optional-locks` and the protect flags, and only the index commands see
/// `GIT_INDEX_FILE`.
#[test]
fn a_round_commit_uses_no_optional_locks_and_the_protect_flags() {
    let scripts = tempfile::tempdir().unwrap();
    let program = super::queue_tests::recording(scripts.path());
    let rig = Rig::new(Some(program));
    let one = round_one(&rig);
    let log = scripts.path().join("git.log");
    std::fs::remove_file(&log).unwrap();
    let result = rig.run(round_two(&rig, &one));
    assert!(
        matches!(result, OpResult::DocsCommitted { .. }),
        "{result:?}"
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let mut subs = Vec::new();
    let mut env: Vec<String> = Vec::new();
    let mut argv: Vec<String> = Vec::new();
    let check = |argv: &[String], env: &[String], subs: &mut Vec<String>| {
        if argv.is_empty() {
            return;
        }
        assert_eq!(
            (argv[0].as_str(), argv[2].as_str()),
            ("-C", "--no-optional-locks")
        );
        let joined = argv.join(" ");
        assert!(
            joined.contains("-c core.protectHFS=true -c core.protectNTFS=true"),
            "{argv:?}"
        );
        let indexed = ["read-tree", "update-index", "write-tree"];
        let index = argv.iter().any(|a| indexed.contains(&a.as_str()));
        let has_index = env.iter().any(|e| e.starts_with("GIT_INDEX_FILE="));
        assert_eq!(index, has_index, "{argv:?}: {env:?}");
        subs.push(joined);
    };
    for line in text.lines() {
        let mut fields = line.split('\t');
        match fields.next() {
            Some("argv") => {
                check(&argv, &env, &mut subs);
                argv = fields.map(String::from).collect();
                env.clear();
            }
            Some("env") => env.push(fields.collect()),
            other => panic!("{other:?}"),
        }
    }
    check(&argv, &env, &mut subs);
    for wanted in [
        "cat-file blob",
        "update-ref --no-deref --stdin",
        "commit-tree",
    ] {
        assert!(
            subs.iter().any(|s| s.contains(wanted)),
            "{wanted}: {subs:#?}"
        );
    }
}
