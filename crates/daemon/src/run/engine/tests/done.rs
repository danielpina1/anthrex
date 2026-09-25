//! M8a.12: the done gate (decisions 32, 55 and 56). `done_tools.rs` has the tools'
//! acceptance, `task_blocked` and decision 54's unavailable sandbox.

use proto::{BlockReason, DoneSignal, Size, TaskState};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use crate::run::contract::{DONE_ACCEPTED, generated_files_message, protected_file_message};
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::OpId;

/// `PROFILE` without its `check` line.
fn no_check() -> String {
    PROFILE.replace("check = \"cargo test\"\n", "")
}

/// `PROFILE` with a `generated` list.
fn generated_profile() -> String {
    PROFILE.replace("setup = ", "generated = [\"Cargo.lock\"]\nsetup = ")
}

/// A working `t1` (S) owning `owns` (TOML array text), with `extra` lines; its window.
fn working_owning(profile: &str, owns: &str, extra: &str) -> (Fixture, u32) {
    let mut fx = Fixture::new(&plan_with(profile, &[task_toml("t1", "S", owns, extra)]));
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t1").state, TaskState::Working);
    (fx, window)
}

pub(super) fn working() -> (Fixture, u32) {
    working_owning(PROFILE, "[\"crates/a/**\"]", "")
}

pub(super) fn done_args() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

/// `task_done` from `window`: exactly one `VerifyDone` and no reply yet.
fn claim(fx: &mut Fixture, window: u32) -> OpId {
    claim_with(fx, window, done_args())
}

fn claim_with(fx: &mut Fixture, window: u32, args: serde_json::Value) -> OpId {
    let effects = fx.tool(window, "task_done", args);
    assert!(replies(&effects).is_empty(), "no reply yet: {effects:#?}");
    let ops = ops_in(&effects, "VerifyDone");
    assert_eq!(ops.len(), 1, "{effects:#?}");
    ops[0].0
}

/// `DoneChecked` for t1: the clean result, edited.
fn verify(fx: &mut Fixture, op: OpId, edit: impl FnOnce(&mut OpResult)) -> Vec<Effect> {
    let mut result = fx.clean_check("t1");
    edit(&mut result);
    fx.done(op, result)
}

pub(super) fn one_reply(effects: &[Effect]) -> Result<String, String> {
    let replies = replies(effects);
    assert_eq!(replies.len(), 1, "{effects:#?}");
    replies[0].clone()
}

pub(super) fn killed(effects: &[Effect], window: u32) -> bool {
    effects.contains(&Effect::KillWindow { window_id: window })
}

pub(super) fn block_of(fx: &Fixture) -> (BlockReason, String) {
    let block = fx.task("t1").block.clone().expect("t1 is blocked");
    (block.reason, block.text)
}

fn outside(files: &[&str]) -> impl FnOnce(&mut OpResult) {
    let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
    move |r| {
        if let OpResult::DoneChecked { outside_owns, .. } = r {
            *outside_owns = files;
        }
    }
}

fn generated(files: &[&str]) -> impl FnOnce(&mut OpResult) {
    let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
    move |r| {
        if let OpResult::DoneChecked {
            generated_outside_owns,
            ..
        } = r
        {
            *generated_outside_owns = files;
        }
    }
}

fn protected(files: &[&str]) -> impl FnOnce(&mut OpResult) {
    let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
    move |r| {
        if let OpResult::DoneChecked {
            protected_changed, ..
        } = r
        {
            *protected_changed = files;
        }
    }
}

fn resolution_only(only: Option<bool>) -> impl FnOnce(&mut OpResult) {
    move |r| {
        if let OpResult::DoneChecked {
            resolution_only, ..
        } = r
        {
            *resolution_only = only;
        }
    }
}

/// The claim is accepted: `Ok(DONE_ACCEPTED)` and the task moves to `state`.
fn assert_accepted(fx: &Fixture, effects: &[Effect], state: TaskState) {
    assert_eq!(one_reply(effects), Ok(DONE_ACCEPTED.to_string()));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, state, "{:#?}", t1.history);
    let done = t1.done.as_ref().expect("the claim is recorded");
    assert_eq!(done.signal, DoneSignal::TaskDone);
    assert_eq!(t1.head.as_deref(), Some(HEAD));
}

#[test]
fn task_done_runs_verify_done_and_replies_after_it() {
    // tdd → proof.
    let (mut fx, window) = working();
    let op = claim(&mut fx, window);
    let (_, kind) = fx.op("VerifyDone");
    let OpKind::VerifyDone {
        worktree,
        start,
        run_head,
        owns,
        spill_exempt,
        red,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(worktree, super::dispatch::task_path("t1"));
    assert_eq!(start, BASE);
    assert_eq!(run_head, BASE);
    assert_eq!(owns, vec!["crates/a/**".to_string()]);
    assert!(!spill_exempt);
    assert_eq!(red.as_deref(), Some("abcdef1"));
    let effects = verify(&mut fx, op, |_| {});
    assert_accepted(&fx, &effects, TaskState::Proof);
    let done = fx.task("t1").done.clone().unwrap();
    assert_eq!(done.test.as_deref(), Some("a::works"));
    assert_eq!(done.red.as_deref(), Some("abcdef1"));
    assert_eq!(done.summary, "did it");

    // check mode, with a check → check.
    let extra = "test_mode = \"check\"\ntest_mode_reason = \"glue code\"";
    let (mut fx, window) = working_owning(PROFILE, "[\"crates/a/**\"]", extra);
    let op = claim_with(&mut fx, window, json!({"summary": "s"}));
    let effects = verify(&mut fx, op, |_| {});
    assert_accepted(&fx, &effects, TaskState::Check);

    // No check in the profile, not tdd → review.
    let extra = "test_mode = \"none\"\ntest_mode_reason = \"docs only\"";
    let (mut fx, window) = working_owning(&no_check(), "[\"docs/**\"]", extra);
    let op = claim_with(&mut fx, window, json!({"summary": "s"}));
    let effects = verify(&mut fx, op, |_| {});
    assert_accepted(&fx, &effects, TaskState::Review);

    // A handed-back task whose claim is only the resolution goes straight back to the
    // merge queue (decision 36, ruling T14-R2)...
    let (mut fx, window) = working();
    fx.task_mut("t1").handed_back = true;
    let op = claim(&mut fx, window);
    let effects = verify(&mut fx, op, resolution_only(Some(true)));
    assert_accepted(&fx, &effects, TaskState::MergeQueue);
    assert_eq!(fx.run().merge_queue, vec!["t1".to_string()]);

    // ...and one that carries more passes the gates.
    for only in [Some(false), None] {
        let (mut fx, window) = working();
        fx.task_mut("t1").handed_back = true;
        let op = claim(&mut fx, window);
        let effects = verify(&mut fx, op, resolution_only(only));
        assert_accepted(&fx, &effects, TaskState::Proof);
        assert!(fx.run().merge_queue.is_empty());
    }
}

#[test]
fn task_done_rejections_leave_the_task_working() {
    type Case = (
        &'static str,
        serde_json::Value,
        Box<dyn FnOnce(&mut OpResult)>,
        String,
    );
    let cases: Vec<Case> = vec![
        (
            "no commit",
            done_args(),
            Box::new(|r| {
                if let OpResult::DoneChecked { commits, .. } = r {
                    *commits = 0;
                }
            }),
            "task_done rejected: the branch has no commit since the task started; commit your work first".into(),
        ),
        (
            "dirty",
            done_args(),
            Box::new(|r| {
                if let OpResult::DoneChecked { dirty_tracked, .. } = r {
                    *dirty_tracked = 3;
                }
            }),
            "task_done rejected: the tracked tree has uncommitted changes (3 files); commit or revert them first".into(),
        ),
        (
            "merge",
            done_args(),
            Box::new(|r| {
                if let OpResult::DoneChecked {
                    merge_in_progress, ..
                } = r
                {
                    *merge_in_progress = true;
                }
            }),
            "task_done rejected: a merge is in progress; finish it with git commit first".into(),
        ),
        (
            "untracked",
            done_args(),
            Box::new(|r| {
                if let OpResult::DoneChecked {
                    untracked_in_owns, ..
                } = r
                {
                    *untracked_in_owns = vec!["crates/a/new.rs".into(), "crates/a/b.rs".into()];
                }
            }),
            "task_done rejected: untracked files inside this task's owns are not committed: crates/a/new.rs, crates/a/b.rs".into(),
        ),
        (
            "tdd without test and red",
            json!({"summary": "s"}),
            Box::new(|r| {
                if let OpResult::DoneChecked { red_ok, .. } = r {
                    *red_ok = None;
                }
            }),
            "task_done rejected: this is a tdd task; name the test (test) and the commit where it was added and failed (red)".into(),
        ),
        (
            "tdd without red",
            json!({"summary": "s", "test": "a::works"}),
            Box::new(|r| {
                if let OpResult::DoneChecked { red_ok, .. } = r {
                    *red_ok = None;
                }
            }),
            "task_done rejected: this is a tdd task; name the test (test) and the commit where it was added and failed (red)".into(),
        ),
        (
            "bad red",
            done_args(),
            Box::new(|r| {
                if let OpResult::DoneChecked { red_ok, .. } = r {
                    *red_ok = Some(false);
                }
            }),
            "task_done rejected: red abcdef1 is not a commit on this task's branch after its start commit".into(),
        ),
        (
            "off the branch (carry T8)",
            done_args(),
            Box::new(|r| {
                if let OpResult::DoneChecked { head_branch, .. } = r {
                    *head_branch = Some("main".into());
                }
            }),
            "task_done rejected: this worktree's HEAD must be a detached commit with no rebase in progress; run git checkout --detach (or finish the rebase), commit, and call task_done again".to_string(),
        ),
        (
            "not a detached commit (F1b: a branch checked out, a rebase stopped)",
            done_args(),
            Box::new(|r| {
                if let OpResult::DoneChecked { head_branch, .. } = r {
                    *head_branch = None;
                }
            }),
            "task_done rejected: this worktree's HEAD must be a detached commit with no rebase in progress; run git checkout --detach (or finish the rebase), commit, and call task_done again".to_string(),
        ),
    ];
    for (name, args, edit, text) in cases {
        let (mut fx, window) = working();
        let op = claim_with(&mut fx, window, args);
        let effects = verify(&mut fx, op, edit);
        assert_eq!(one_reply(&effects), Err(text), "{name}");
        let t1 = fx.task("t1");
        assert_eq!(t1.state, TaskState::Working, "{name}");
        assert_eq!(t1.failures, 0, "{name}: nothing is counted");
        assert_eq!(t1.bounces, Default::default(), "{name}");
        assert!(t1.done.is_none(), "{name}");
        assert!(!killed(&effects, window), "{name}");
        // The worker may claim again.
        claim(&mut fx, window);
    }
}

#[test]
fn task_done_with_files_outside_owns_goes_to_rung_3() {
    let (mut fx, window) = working();
    let op = claim(&mut fx, window);
    let effects = verify(&mut fx, op, outside(&["x.rs"]));
    assert!(one_reply(&effects).is_err());
    assert_eq!(
        block_of(&fx),
        (
            BlockReason::MisSized,
            "changed files outside owns: x.rs".to_string()
        )
    );
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.size, t1.raised_size, t1.rung),
        (Size::M, Some(Size::M), 3)
    );
    assert!(killed(&effects, window), "rung 3 kills the worker");
}

#[test]
fn a_generated_file_outside_owns_bounces_at_rung_1() {
    let (mut fx, window) = working_owning(&generated_profile(), "[\"crates/a/**\"]", "");
    let op = claim(&mut fx, window);
    let (_, kind) = fx.op("VerifyDone");
    let OpKind::VerifyDone { generated: g, .. } = kind else {
        unreachable!()
    };
    assert_eq!(g, vec!["Cargo.lock".to_string()]);
    let effects = verify(&mut fx, op, generated(&["Cargo.lock"]));
    let text = generated_files_message(&["Cargo.lock".to_string()]);
    assert_eq!(one_reply(&effects), Err(text));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!((t1.bounces.done, t1.failures), (1, 1));
    assert_eq!(t1.rounds.len(), 1, "the same session");
    assert!(!killed(&effects, window));
    assert!(fx.run().outbox.is_empty(), "rung 1 queues nothing more");

    // A second such claim is rung 2: a fresh session.
    let op = claim(&mut fx, window);
    let effects = verify(&mut fx, op, generated(&["Cargo.lock"]));
    assert!(one_reply(&effects).is_err());
    assert!(killed(&effects, window));
    let t1 = fx.task("t1");
    assert_eq!((t1.bounces.done, t1.failures, t1.rung), (2, 2, 2));
    assert_eq!(t1.state, TaskState::Working);
}

#[test]
fn a_non_generated_path_outside_owns_still_goes_to_rung_3() {
    let (mut fx, window) = working_owning(&generated_profile(), "[\"crates/a/**\"]", "");
    let op = claim(&mut fx, window);
    let effects = verify(&mut fx, op, |r| {
        outside(&["x.rs"])(r);
        generated(&["Cargo.lock"])(r);
    });
    assert_eq!(block_of(&fx).0, BlockReason::MisSized);
    assert_eq!(fx.task("t1").bounces.done, 0);
    assert!(killed(&effects, window));
}

#[test]
fn a_task_that_owns_the_generated_file_passes() {
    let owns = "[\"crates/a/**\", \"Cargo.lock\"]";
    let (mut fx, window) = working_owning(&generated_profile(), owns, "");
    let op = claim(&mut fx, window);
    let (_, kind) = fx.op("VerifyDone");
    let OpKind::VerifyDone { owns, .. } = kind else {
        unreachable!()
    };
    assert!(owns.contains(&"Cargo.lock".to_string()));
    let effects = verify(&mut fx, op, |_| {});
    assert_accepted(&fx, &effects, TaskState::Proof);
}

fn assert_protected_bounce(fx: &Fixture, effects: &[Effect], files: &[&str], window: u32) {
    let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
    assert_eq!(one_reply(effects), Err(protected_file_message(&files)));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!((t1.bounces.done, t1.failures), (1, 1));
    assert!(!killed(effects, window));
    assert!(fx.run().outbox.is_empty());
}

#[test]
fn a_protected_file_changed_through_a_wildcard_bounces_at_rung_1() {
    let (mut fx, window) = working_owning(PROFILE, "[\"**\"]", "");
    let op = claim(&mut fx, window);
    let effects = verify(&mut fx, op, protected(&["AGENTS.md"]));
    assert_protected_bounce(&fx, &effects, &["AGENTS.md"], window);
    let Err(text) = one_reply(&effects) else {
        unreachable!()
    };
    assert!(text.contains("AGENTS.md configures or instructs future agents"));
}

#[test]
fn a_protected_file_named_exactly_passes() {
    let (mut fx, window) = working_owning(PROFILE, "[\"src/**\", \"AGENTS.md\"]", "");
    let op = claim(&mut fx, window);
    // Even reported, a path `owns` names literally is allowed (decision 56).
    let effects = verify(&mut fx, op, protected(&["AGENTS.md"]));
    assert_accepted(&fx, &effects, TaskState::Proof);
    assert_eq!(fx.task("t1").bounces.done, 0);
}

#[test]
fn a_directory_glob_over_claude_settings_still_bounces() {
    let (mut fx, window) = working_owning(PROFILE, "[\".claude/**\"]", "");
    let op = claim(&mut fx, window);
    let effects = verify(&mut fx, op, protected(&[".claude/settings.json"]));
    assert_protected_bounce(&fx, &effects, &[".claude/settings.json"], window);
}

#[test]
fn a_nested_agents_md_is_protected() {
    let (mut fx, window) = working_owning(PROFILE, "[\"docs/**\"]", "");
    let op = claim(&mut fx, window);
    let effects = verify(&mut fx, op, protected(&["docs/AGENTS.md"]));
    assert_protected_bounce(&fx, &effects, &["docs/AGENTS.md"], window);
}

#[test]
fn protected_is_checked_before_the_spill_split() {
    let (mut fx, window) = working_owning(&generated_profile(), "[\"crates/a/**\"]", "");
    let op = claim(&mut fx, window);
    let effects = verify(&mut fx, op, |r| {
        protected(&["AGENTS.md"])(r);
        outside(&["x.rs"])(r);
        generated(&["Cargo.lock"])(r);
    });
    assert_protected_bounce(&fx, &effects, &["AGENTS.md"], window);
}

#[test]
fn an_overridden_task_is_exempt_from_protected() {
    let (mut fx, window) = working_owning(&generated_profile(), "[\"crates/a/**\"]", "");
    fx.task_mut("t1").merged_without_approval = Some("the user accepted it".into());
    let op = claim(&mut fx, window);
    let (_, kind) = fx.op("VerifyDone");
    let OpKind::VerifyDone { spill_exempt, .. } = kind else {
        unreachable!()
    };
    assert!(spill_exempt);
    let effects = verify(&mut fx, op, |r| {
        protected(&["AGENTS.md"])(r);
        outside(&["x.rs"])(r);
        generated(&["Cargo.lock"])(r);
    });
    assert_accepted(&fx, &effects, TaskState::Proof);
    assert_eq!(fx.task("t1").failures, 0);
}
