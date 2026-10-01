use crate::actions::{ACTION_TEXT_MAX, ActionInfo, ActionKind, ActionNeeds, InputKind};

fn every_kind() -> Vec<ActionKind> {
    use ActionKind::*;
    vec![
        ReviewPlan,
        Approve,
        Reject,
        Submit,
        ApproveHold { hold: "e1".into() },
        RejectHold { hold: "e1".into() },
        Pause,
        Unpause,
        Resume,
        Cancel,
        Promote,
        Accept,
        Discard,
        Stats,
        MessageStage { stage: 2 },
        Answer,
        Message,
        Refresh,
        Retry,
        Override,
        CancelTask,
        OpenConversation,
    ]
}

fn an_info(kind: ActionKind) -> ActionInfo {
    ActionInfo {
        needs: kind.needs(),
        destructive: kind.destructive(),
        label: "l".into(),
        effect: "e".into(),
        refused_why: Some("run r is running".into()),
        kind,
    }
}

/// Ruling F2: `encode` prepends a 4-byte length, so the round trip goes through
/// `rmp_serde` directly, as `conversation_tests.rs` does.
#[test]
fn action_info_round_trips_through_messagepack_and_json() {
    for kind in every_kind() {
        let info = an_info(kind);
        let bytes = rmp_serde::to_vec_named(&info).unwrap();
        let back: ActionInfo = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(back, info);
        let json: ActionInfo =
            serde_json::from_str(&serde_json::to_string(&info).unwrap()).unwrap();
        assert_eq!(json, info);
        // `refused_why` is `#[serde(default)]`: absent decodes as `None`.
        let mut value = serde_json::to_value(&info).unwrap();
        value.as_object_mut().unwrap().remove("refused_why");
        let none: ActionInfo = serde_json::from_value(value).unwrap();
        assert_eq!(none.refused_why, None);
    }
}

#[test]
fn needs_and_destructive_follow_the_table() {
    use ActionKind::*;
    assert_eq!(Answer.needs(), ActionNeeds::Input(InputKind::Answer));
    assert_eq!(Message.needs(), ActionNeeds::Input(InputKind::Message));
    assert_eq!(Override.needs(), ActionNeeds::Input(InputKind::Reason));
    assert_eq!(Resume.needs(), ActionNeeds::Input(InputKind::Resume));
    assert_eq!(Promote.needs(), ActionNeeds::Input(InputKind::Promote));
    assert_eq!(
        MessageStage { stage: 1 }.needs(),
        ActionNeeds::Input(InputKind::Message)
    );
    assert_eq!(ReviewPlan.needs(), ActionNeeds::Open);
    assert_eq!(Stats.needs(), ActionNeeds::Open);
    assert_eq!(OpenConversation.needs(), ActionNeeds::Open);
    assert_eq!(Accept.needs(), ActionNeeds::Confirm);
    let destructive: Vec<_> = every_kind()
        .into_iter()
        .filter(ActionKind::destructive)
        .collect();
    assert_eq!(
        destructive,
        vec![
            Reject,
            RejectHold { hold: "e1".into() },
            Cancel,
            Discard,
            CancelTask
        ]
    );
}

#[test]
fn only_three_kinds_are_local() {
    let local: Vec<_> = every_kind()
        .into_iter()
        .filter(ActionKind::is_local)
        .collect();
    assert_eq!(
        local,
        vec![
            ActionKind::ReviewPlan,
            ActionKind::Stats,
            ActionKind::OpenConversation
        ]
    );
    assert_eq!(ACTION_TEXT_MAX, 200);
}

#[test]
fn a_12_era_run_info_decodes_with_no_actions() {
    let info: crate::RunInfo = serde_json::from_str(include_str!("m912_run_info.json")).unwrap();
    assert!(info.actions.is_empty());
    assert!(!info.tasks.is_empty() && !info.stages.is_empty());
    assert!(info.tasks.iter().all(|t| t.actions.is_empty()));
    assert!(info.stages.iter().all(|s| s.actions.is_empty()));
}

#[test]
fn actions_round_trip_on_run_task_and_stage() {
    let mut info: crate::RunInfo =
        serde_json::from_str(include_str!("m912_run_info.json")).unwrap();
    info.actions = vec![an_info(ActionKind::Accept)];
    info.tasks[0].actions = vec![an_info(ActionKind::Answer)];
    info.stages[0].actions = vec![an_info(ActionKind::MessageStage { stage: 1 })];
    let back: crate::RunInfo =
        rmp_serde::from_slice(&rmp_serde::to_vec_named(&info).unwrap()).unwrap();
    assert_eq!(back, info);
    let json: crate::RunInfo =
        serde_json::from_str(&serde_json::to_string(&info).unwrap()).unwrap();
    assert_eq!(json, info);
}

#[test]
fn empty_actions_cost_nothing_on_the_wire() {
    let info: crate::RunInfo = serde_json::from_str(include_str!("m912_run_info.json")).unwrap();
    let bytes = rmp_serde::to_vec_named(&info).unwrap();
    assert!(!bytes.windows(7).any(|w| w == b"actions"));
    assert!(!serde_json::to_string(&info).unwrap().contains("actions"));
}
