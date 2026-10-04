//! Milestone 9.0.6 task 7 (review focus 1): for every fixture state, node and request
//! kind, `actions::check` is exactly what the engine's handler replies, and each node
//! lists decision 9's kinds.

use super::actions_fixtures::{self, FIXTURES, named};
use super::dispatch::replies;
use super::fixture::*;
use crate::run::engine::EventKind;
use crate::run::engine::actions::{self, ActionNode};
use crate::run::engine::stages::Rebaseline;
use crate::run::validate::EditScope;
use proto::{ActionKind, FinishAction, MessageKind, MessageTarget, PlanEdit};

/// The engine event the driver sends for `kind` on `node`, with a valid input. Resume
/// carries a rebaseline (preflight F31: a halt that is not retryable needs one), except
/// for a phase budget's halt (ruling T7-1).
fn event_for(fx: &mut Fixture, node: &ActionNode, kind: &ActionKind) -> EventKind {
    let (reply, run_id) = (fx.reply(), RUN_ID.to_string());
    let edit = |edits: Vec<PlanEdit>, submit| EventKind::Edit {
        reply,
        run_id: RUN_ID.to_string(),
        edits,
        scope: EditScope::Run,
        refusals: vec![],
        submit,
    };
    let task = || match node {
        ActionNode::Task(id) => id.to_string(),
        _ => unreachable!("a task kind on {node:?}"),
    };
    match kind {
        ActionKind::Approve => EventKind::Approve { reply, run_id },
        ActionKind::Reject => EventKind::Reject { reply, run_id },
        ActionKind::Submit => edit(vec![], true),
        ActionKind::Pause => edit(vec![PlanEdit::Pause], false),
        ActionKind::Unpause => edit(vec![PlanEdit::Resume], false),
        // Milestone 9.6 ruling T7-1: a phase budget's halt resumes without one.
        ActionKind::Resume => EventKind::Resume {
            reply,
            rebaseline: crate::run::engine::design::halted_phase(fx.run())
                .is_none()
                .then(|| Rebaseline::from((BASE.to_string(), HEAD.to_string()))),
            run_id,
        },
        ActionKind::Cancel => EventKind::Cancel { reply, run_id },
        ActionKind::Promote => EventKind::Promote {
            reply,
            run_id,
            orchestrator: None,
        },
        ActionKind::Accept => EventKind::Finish {
            reply,
            run_id,
            action: FinishAction::Accept,
        },
        ActionKind::Discard => EventKind::Finish {
            reply,
            run_id,
            action: FinishAction::Discard,
        },
        ActionKind::ApproveHold { hold } | ActionKind::RejectHold { hold } => {
            let approve = matches!(kind, ActionKind::ApproveHold { .. });
            fx.hold_event(reply, hold, approve)
        }
        ActionKind::MessageStage { stage } => edit(
            vec![PlanEdit::Message {
                to: MessageTarget::Stage(u32::from(*stage)),
                text: "m".into(),
                kind: MessageKind::Info,
            }],
            false,
        ),
        ActionKind::Answer => edit(
            vec![PlanEdit::Answer {
                task_id: task(),
                text: "a".into(),
            }],
            false,
        ),
        ActionKind::Message => edit(
            vec![PlanEdit::Message {
                to: MessageTarget::Tasks(vec![task()]),
                text: "m".into(),
                kind: MessageKind::Info,
            }],
            false,
        ),
        ActionKind::Refresh => edit(vec![PlanEdit::Refresh { task_id: task() }], false),
        ActionKind::Retry => EventKind::Retry {
            reply,
            run_id,
            task_id: task(),
        },
        ActionKind::Override => EventKind::Override {
            reply,
            run_id,
            task_id: task(),
            reason: "r".into(),
        },
        ActionKind::CancelTask => edit(vec![PlanEdit::CancelTask { task_id: task() }], false),
        ActionKind::ReviewPlan | ActionKind::Stats | ActionKind::OpenConversation => {
            unreachable!("client-only kinds are never request kinds")
        }
        // Milestone 9.3 decision 9: `run iterate`.
        ActionKind::Iterate => EventKind::Iterate {
            reply,
            run_id,
            goal: "more".into(),
        },
        // Milestone 9.6 decision 34: it opens the gate's screen; no request backs it.
        ActionKind::ReviewDoc => unreachable!("never a request kind"),
    }
}

/// Review focus 1: for every fixture state, node and request kind, `check` is exactly
/// what the handler replies: the same refusal text, or no refusal at all. A request
/// answered by an op (accept, discard, an override whose commits are counted) has no
/// reply in the same step: "not refused now" is then the empty reply list.
#[test]
fn every_check_is_the_handlers_own_reply() {
    let mut pairs = 0;
    for (name, build) in FIXTURES {
        let probe = build();
        for node in actions_fixtures::nodes(probe.run()) {
            for kind in actions::request_kinds(probe.run(), &node.as_node()) {
                let mut fx = build();
                let expected = actions::check(fx.run(), &node.as_node(), &kind);
                let event = event_for(&mut fx, &node.as_node(), &kind);
                let got = replies(&fx.next(event));
                match expected {
                    Err(text) => assert_eq!(got, vec![Err(text)], "{name}: {kind:?} on {node:?}"),
                    Ok(()) => assert!(
                        got.iter().all(Result::is_ok),
                        "{name}: {kind:?} on {node:?}: {got:?}"
                    ),
                }
                pairs += 1;
            }
        }
    }
    assert!(pairs > 300, "{pairs} pairs: the matrix lost its fixtures");
}

fn kinds(fx: &Fixture, node: ActionNode) -> Vec<ActionKind> {
    actions::available(fx.run(), &node)
        .into_iter()
        .map(|a| a.kind)
        .collect()
}

fn hold(id: &str, approve: bool) -> ActionKind {
    let hold = id.to_string();
    match approve {
        true => ActionKind::ApproveHold { hold },
        false => ActionKind::RejectHold { hold },
    }
}

#[test]
fn listed_kinds_follow_decision_9() {
    use ActionKind::*;
    let fx = named("running");
    let on = |id: &str| kinds(&fx, ActionNode::Task(id));
    assert_eq!(
        on("question"),
        vec![Answer, Message, Retry, Override, CancelTask]
    );
    assert!(on("merged").is_empty());
    assert!(on("doomed").is_empty(), "a cancelled task");
    assert_eq!(on("working"), vec![Answer, Message, Refresh, CancelTask]);
    assert_eq!(on("pending"), vec![Message, CancelTask]);
    assert_eq!(on("review"), vec![Message, Override, CancelTask]);
    assert_eq!(on("mergeq"), vec![Message, CancelTask]);
    assert_eq!(on("human"), vec![Message, Retry, Override, CancelTask]);
    assert_eq!(
        on("paused"),
        vec![Message, Refresh, Retry, Override, CancelTask]
    );
    assert_eq!(
        on("research"),
        vec![Answer, Refresh, CancelTask],
        "no message"
    );
    let run: Vec<_> = actions::available(fx.run(), &ActionNode::Run)
        .into_iter()
        .map(|a| (a.kind, a.refused_why))
        .collect();
    assert!(run.contains(&(
        Accept,
        Some(format!(
            "run {RUN_ID} is running; accept applies only to a complete run"
        ))
    )));
    assert_eq!(
        kinds(&fx, ActionNode::Run),
        vec![Pause, Cancel, Accept, Discard]
    );
    // The run rows.
    assert_eq!(
        kinds(&named("gate"), ActionNode::Run),
        vec![Approve, Reject]
    );
    assert_eq!(
        kinds(&named("gate_dormant"), ActionNode::Run),
        vec![Approve, Reject]
    );
    assert_eq!(
        kinds(&named("planning"), ActionNode::Run),
        vec![Approve, Reject, Submit]
    );
    assert_eq!(
        kinds(&named("planning_paused"), ActionNode::Run),
        vec![Approve, Reject, Unpause, Cancel, Accept, Discard]
    );
    assert_eq!(
        kinds(&named("paused"), ActionNode::Run),
        vec![Unpause, Cancel, Accept, Discard]
    );
    for halted in ["halted", "halted_retryable"] {
        assert_eq!(
            kinds(&named(halted), ActionNode::Run),
            vec![Resume, Cancel, Accept, Discard],
            "{halted}"
        );
    }
    // Preflight F32: a held tier 3 lists Resume on a running run.
    assert_eq!(
        kinds(&named("held_tier3"), ActionNode::Run),
        vec![Pause, Resume, Cancel, Accept, Discard]
    );
    assert_eq!(
        kinds(&named("with_awaiting_hold"), ActionNode::Run),
        vec![
            hold("epic:mail", true),
            hold("epic:mail", false),
            Pause,
            Cancel,
            Accept,
            Discard
        ]
    );
    assert_eq!(
        kinds(&named("with_approved_hold"), ActionNode::Run),
        vec![Pause, Cancel, Accept, Discard]
    );
    assert_eq!(
        kinds(&named("fast_path"), ActionNode::Run),
        vec![Pause, Cancel, Promote, Accept, Discard]
    );
    assert_eq!(
        kinds(&named("fast_path_promoted"), ActionNode::Run),
        vec![Submit, Pause, Cancel, Accept, Discard],
        "a promoted run is no longer on the fast path"
    );
    for complete in ["complete", "complete_with_moved_base", "finishing"] {
        assert_eq!(
            kinds(&named(complete), ActionNode::Run),
            vec![Accept, Discard],
            "{complete}"
        );
    }
    // Milestone 9.3 decision 9: a complete run with an orchestrator can be iterated.
    assert_eq!(
        kinds(&named("complete_orchestrated"), ActionNode::Run),
        vec![Accept, Discard, Iterate]
    );
    // The stage row: only a `Multi` run has stage nodes.
    let multi = named("multi_stage");
    for stage in [1, 2] {
        assert_eq!(
            kinds(&multi, ActionNode::Stage(stage)),
            vec![MessageStage { stage }]
        );
    }
    assert!(kinds(&fx, ActionNode::Stage(1)).is_empty());
}

#[test]
fn a_terminal_run_lists_nothing() {
    for name in ["accepted", "discarded", "failed"] {
        let fx = named(name);
        for node in actions_fixtures::nodes(fx.run()) {
            assert!(
                actions::available(fx.run(), &node.as_node()).is_empty(),
                "{name}: {node:?}"
            );
        }
    }
}

#[test]
fn a_finishing_run_refuses_with_being_accepted() {
    let fx = named("finishing");
    let refused: Vec<_> = actions::available(fx.run(), &ActionNode::Run)
        .into_iter()
        .map(|a| (a.kind, a.refused_why))
        .collect();
    let being = Some(format!("run {RUN_ID} is being accepted"));
    assert_eq!(
        refused,
        vec![
            (ActionKind::Accept, being.clone()),
            (ActionKind::Discard, being)
        ]
    );
}

#[test]
fn client_only_kinds_are_never_listed() {
    for (name, build) in FIXTURES {
        let fx = build();
        for node in actions_fixtures::nodes(fx.run()) {
            let listed = actions::available(fx.run(), &node.as_node());
            let requests = actions::request_kinds(fx.run(), &node.as_node());
            for action in listed {
                assert!(!action.kind.is_local(), "{name}: {node:?}");
                // Milestone 9.6 decision 34: the daemon lists `ReviewDoc`, which opens
                // the gate's screen and is backed by no request.
                let opens = action.kind == ActionKind::ReviewDoc;
                assert!(opens || requests.contains(&action.kind), "{name}: {node:?}");
                let confirm = action.needs == proto::ActionNeeds::Confirm;
                assert!(confirm || action.needs == action.kind.needs(), "{name}");
                assert_eq!(action.destructive, action.kind.destructive());
            }
            assert!(requests.iter().all(|k| !k.is_local()), "{name}");
        }
    }
}

/// Milestone 9.3 decisions 9 and 32: in every fixture state, `iterate` is refused in
/// exactly the handler's words, and listed (with no refusal) exactly when it is not.
#[test]
fn the_action_menu_refuses_iterate_in_the_handlers_words() {
    let mut accepted = 0;
    for (name, build) in FIXTURES {
        let mut fx = build();
        let kind = ActionKind::Iterate;
        let check = actions::check(fx.run(), &ActionNode::Run, &kind);
        let listed = actions::available(fx.run(), &ActionNode::Run)
            .into_iter()
            .find(|a| a.kind == kind);
        assert_eq!(listed.is_some(), check.is_ok(), "{name}: {check:?}");
        if let Some(action) = &listed {
            assert_eq!(action.refused_why, None, "{name}");
            assert_eq!(action.label, "iterate");
            assert_eq!(
                action.effect,
                "plan round 2 of this run with its orchestrator"
            );
            assert_eq!(action.needs, proto::ActionNeeds::Open);
        }
        let event = event_for(&mut fx, &ActionNode::Run, &kind);
        let got = replies(&fx.next(event));
        match check {
            Err(text) => assert_eq!(got, vec![Err(text)], "{name}"),
            Ok(()) => {
                assert_eq!(got.len(), 1, "{name}: {got:?}");
                assert!(got[0].is_ok(), "{name}: {got:?}");
                accepted += 1;
            }
        }
    }
    assert_eq!(accepted, 1, "only the complete run with an orchestrator");
}
