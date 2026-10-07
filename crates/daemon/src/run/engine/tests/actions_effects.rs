//! Milestone 9.0.6 task 7: each listed action's label and effect line, exactly as
//! Interfaces "Exact user-visible text" has them (decision 11, preflight F1 and F32),
//! and review focus 5's sanitising and cap.

use proto::{ActionKind, ActionNeeds, InputKind, PlanEdit};

use super::actions_fixtures::{MOVED_TO, gate, named};
use super::control::blocked;
use super::dispatch::{edit, replies};
use super::fixture::*;
use crate::run::engine::actions::{self, ActionNode};

/// The listed action `kind` on `node`.
fn action(fx: &Fixture, node: ActionNode, kind: &ActionKind) -> proto::ActionInfo {
    actions::available(fx.run(), &node)
        .into_iter()
        .find(|a| a.kind == *kind)
        .unwrap_or_else(|| panic!("{kind:?} is not listed on {node:?}"))
}

fn effect(fx: &Fixture, node: ActionNode, kind: ActionKind) -> String {
    action(fx, node, &kind).effect
}

/// `t1` (routed at medium effort) working, `t2` waiting for it, `--yes`; `t1`'s window.
fn t1_working() -> (Fixture, u32) {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", "route = { effort = \"medium\" }"),
            task("t2", "S", "b", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let windows = fx.launch_all();
    (fx, windows[0].1)
}

#[test]
fn approve_names_the_tasks_and_the_base() {
    assert_eq!(
        effect(&gate(), ActionNode::Run, ActionKind::Approve),
        "approve: 3 tasks start, each in its own worktree off main@b0b0b0b"
    );
    assert_eq!(
        effect(&named("planning"), ActionNode::Run, ActionKind::Approve),
        "approve: 1 task starts, each in its own worktree off main@b0b0b0b"
    );
}

#[test]
fn reject_and_discard_name_the_branches_and_keep_the_base() {
    assert_eq!(
        effect(&gate(), ActionNode::Run, ActionKind::Reject),
        format!(
            "reject: discard run {RUN_ID}: its worktrees and anthrex/{RUN_ID}/* branches go; main is unchanged"
        )
    );
    // Preflight F1: only uncommitted work is salvaged.
    assert_eq!(
        effect(&named("complete"), ActionNode::Run, ActionKind::Discard),
        format!(
            "discard: remove the run's worktrees and anthrex/{RUN_ID}/* branches (uncommitted work is kept under refs/anthrex/salvage/{RUN_ID}/); main is unchanged"
        )
    );
}

#[test]
fn submit_names_the_plan() {
    assert_eq!(
        effect(&named("planning"), ActionNode::Run, ActionKind::Submit),
        "submit: the plan of 1 task goes to the plan gate"
    );
}

#[test]
fn hold_verdicts_name_their_tasks() {
    let fx = named("with_awaiting_hold");
    let hold = "epic:mail".to_string();
    let approve = ActionKind::ApproveHold { hold: hold.clone() };
    let reject = ActionKind::RejectHold { hold };
    assert_eq!(
        action(&fx, ActionNode::Run, &approve).label,
        "approve hold epic:mail"
    );
    assert_eq!(
        effect(&fx, ActionNode::Run, approve),
        "approve hold epic:mail: 1 task may start"
    );
    assert_eq!(
        effect(&fx, ActionNode::Run, reject),
        "reject hold epic:mail: its 1 task is cancelled; none has started"
    );
}

#[test]
fn pause_and_unpause() {
    assert_eq!(
        effect(&named("running"), ActionNode::Run, ActionKind::Pause),
        "pause: no new task, gate or delivery starts; open turns finish"
    );
    let unpause = action(&named("paused"), ActionNode::Run, &ActionKind::Unpause);
    assert_eq!(unpause.label, "resume");
    assert_eq!(
        unpause.effect,
        "resume: dispatch, gates and deliveries start again"
    );
}

#[test]
fn resume_names_the_halt_or_the_held_stage() {
    let mut fx = named("halted");
    let resume = action(&fx, ActionNode::Run, &ActionKind::Resume);
    assert_eq!(resume.label, "resume halted run");
    assert_eq!(resume.needs, ActionNeeds::Input(InputKind::Resume));
    assert_eq!(
        resume.effect,
        format!(
            "resume: refs/heads/anthrex/{RUN_ID}/integration moved from 1eeeeee to 9999999; rebaseline reads main and the run head again"
        )
    );
    fx.run_mut().halted_reason = Some("\n  \nfirst line\nsecond line".into());
    assert_eq!(
        effect(&fx, ActionNode::Run, ActionKind::Resume),
        "resume: first line; rebaseline reads main and the run head again"
    );
    fx.run_mut().halted_reason = None;
    assert_eq!(
        effect(&fx, ActionNode::Run, ActionKind::Resume),
        "resume: run halted; rebaseline reads main and the run head again"
    );
    // Preflight F32: no form, a plain resume retries the held tier 3.
    let held = action(&named("held_tier3"), ActionNode::Run, &ActionKind::Resume);
    assert_eq!(held.label, "retry tier 3");
    assert_eq!(held.effect, "resume: tier 3 retries on stage 1");
    assert_eq!(held.needs, ActionNeeds::Confirm);
    assert_eq!(held.refused_why, None);
}

#[test]
fn cancel_counts_workers_and_unmerged_tasks() {
    let (fx, _) = t1_working();
    let cancel = action(&fx, ActionNode::Run, &ActionKind::Cancel);
    assert_eq!(cancel.label, "cancel run");
    assert_eq!(
        cancel.effect,
        "cancel: stop 1 worker and cancel 2 unmerged tasks; the run then completes"
    );
    assert_eq!(
        effect(&named("running"), ActionNode::Run, ActionKind::Cancel),
        "cancel: stop 7 workers and cancel 10 unmerged tasks; the run then completes"
    );
}

#[test]
fn promote_names_the_hold() {
    assert_eq!(
        effect(&named("fast_path"), ActionNode::Run, ActionKind::Promote),
        "promote: give this fast-path run an orchestrator; what it adds waits under hold promotion"
    );
}

#[test]
fn accept_names_the_merged_tasks_and_the_base() {
    assert_eq!(
        effect(&named("complete"), ActionNode::Run, ActionKind::Accept),
        "accept: merge 2 tasks into main@b0b0b0b"
    );
    assert_eq!(
        effect(
            &named("complete_with_moved_base"),
            ActionNode::Run,
            ActionKind::Accept
        ),
        format!(
            "accept: merge 2 tasks into main@{} (moved from b0b0b0b)",
            &MOVED_TO[..7]
        )
    );
}

#[test]
fn message_stage_counts_its_unfinished_tasks() {
    let fx = named("multi_stage");
    let kind = ActionKind::MessageStage { stage: 2 };
    assert_eq!(
        action(&fx, ActionNode::Stage(2), &kind).label,
        "message stage 2"
    );
    assert_eq!(
        effect(&fx, ActionNode::Stage(2), kind),
        "message stage 2: 1 unfinished task gets it at its next turn"
    );
    let mut fx = fx;
    fx.task_mut("t1").spec.stage = 2;
    assert_eq!(
        effect(
            &fx,
            ActionNode::Stage(2),
            ActionKind::MessageStage { stage: 2 }
        ),
        "message stage 2: 2 unfinished tasks get it at their next turn"
    );
}

#[test]
fn task_effects_name_the_task() {
    let (mut fx, window) = t1_working();
    // Milestone 9.8 decision 29: Sonnet reports `low`, `medium`, `high`.
    fx.with_efforts();
    let t1 = || ActionNode::Task("t1");
    assert_eq!(
        effect(&fx, t1(), ActionKind::Answer),
        "answer t1: its worker gets the answer in the same session"
    );
    assert_eq!(
        effect(&fx, t1(), ActionKind::Message),
        "message t1: its worker gets it at its next turn"
    );
    assert_eq!(
        effect(&fx, t1(), ActionKind::Refresh),
        "refresh t1: merge the run's latest merged work (b0b0b0b) into its branch at its next turn boundary"
    );
    blocked(&mut fx, window, "question", "which table?");
    assert_eq!(
        effect(&fx, t1(), ActionKind::Retry),
        "retry t1: a fresh session at rung 2 on claude claude-sonnet-5 (high effort)"
    );
    assert_eq!(
        effect(&fx, t1(), ActionKind::Override),
        "override t1: to the merge queue without review"
    );
}

#[test]
fn cancel_task_counts_its_dependents() {
    let mut fx = gate();
    let cancel = |fx: &Fixture, id| effect(fx, ActionNode::Task(id), ActionKind::CancelTask);
    assert_eq!(
        cancel(&fx, "t1"),
        "cancel t1: its worker stops; 1 dependent becomes blocked(dep_cancelled)"
    );
    assert_eq!(
        cancel(&fx, "t3"),
        "cancel t3: its worker stops; nothing depends on it"
    );
    let add_dep = PlanEdit::AddDep {
        task_id: "t3".into(),
        dep: "t1".into(),
    };
    assert!(replies(&edit(&mut fx, vec![add_dep]))[0].is_ok());
    assert_eq!(
        cancel(&fx, "t1"),
        "cancel t1: its worker stops; 2 dependents become blocked(dep_cancelled)"
    );
}

#[test]
fn every_label_is_the_spec_label() {
    let labels = [
        (ActionKind::Approve, "approve"),
        (ActionKind::Reject, "reject"),
        (ActionKind::Submit, "submit"),
        (ActionKind::Pause, "pause"),
        (ActionKind::Unpause, "resume"),
        (ActionKind::Resume, "resume halted run"),
        (ActionKind::Cancel, "cancel run"),
        (ActionKind::Promote, "promote"),
        (ActionKind::Accept, "accept"),
        (ActionKind::Discard, "discard"),
        (ActionKind::Answer, "answer"),
        (ActionKind::Message, "message"),
        (ActionKind::Refresh, "refresh"),
        (ActionKind::Retry, "retry"),
        (ActionKind::Override, "override"),
        (ActionKind::CancelTask, "cancel task"),
        (ActionKind::ReviewPlan, "review plan"),
        (ActionKind::Stats, "stats"),
        (ActionKind::OpenConversation, "open conversation"),
        (
            ActionKind::RejectHold {
                hold: "promotion".into(),
            },
            "reject hold promotion",
        ),
    ];
    for (kind, label) in labels {
        assert_eq!(actions::label(&kind), label, "{kind:?}");
    }
}

/// Review focus 5: a halted reason is git's or an agent's words; the effect line
/// carries no control or bidi character and is cut to `ACTION_TEXT_MAX` with `…`.
#[test]
fn action_text_is_sanitised_and_capped() {
    let mut fx = named("halted");
    fx.run_mut().halted_reason = Some(format!("\x1b[2Jbase\u{202e} moved {}", "x".repeat(400)));
    let resume = actions::available(fx.run(), &ActionNode::Run)
        .into_iter()
        .find(|a| a.kind == ActionKind::Resume)
        .unwrap();
    assert!(!resume.effect.contains('\x1b') && !resume.effect.contains('\u{202e}'));
    assert!(resume.effect.chars().count() <= proto::ACTION_TEXT_MAX);
    assert!(resume.effect.ends_with('…'));
    // A refusal carries the run's own words too: a hostile run id in accept's refusal
    // (the run is halted) is sanitised and capped the same way.
    let mut run = fx.run().clone();
    run.id = format!("\x1b[2J\u{202e}run{}", "r".repeat(400));
    let accept = actions::available(&run, &ActionNode::Run)
        .into_iter()
        .find(|a| a.kind == ActionKind::Accept)
        .unwrap();
    let why = accept
        .refused_why
        .expect("a halted run's accept is refused");
    assert!(!why.contains('\x1b') && !why.contains('\u{202e}'), "{why}");
    assert!(why.chars().count() <= proto::ACTION_TEXT_MAX, "{why}");
    assert!(
        why.starts_with("run  [2Jrun") && why.ends_with('…'),
        "{why}"
    );
}
