use super::*;
use crate::headless::{FailureKind, TurnOutcome};
use serde_json::json;

fn run(events: &[SessionEvent]) -> HeadlessStatus {
    events
        .iter()
        .fold(HeadlessStatus::default(), |state, event| {
            next(&state, event)
        })
}

fn init() -> SessionEvent {
    SessionEvent::Init {
        session_id: "s-1".into(),
        model: None,
        mcp_ok: Some(true),
    }
}

fn tool(name: &str, parent: Option<&str>) -> SessionEvent {
    SessionEvent::ToolUse {
        id: format!("id-{name}"),
        name: name.into(),
        input: json!({}),
        parent: parent.map(str::to_owned),
    }
}

fn ended(outcome: TurnOutcome) -> SessionEvent {
    SessionEvent::TurnEnded {
        outcome,
        usage: None,
        denials: vec![],
    }
}

fn retry() -> SessionEvent {
    SessionEvent::ApiRetry {
        error: "rate_limit".into(),
        attempt: 1,
        delay_ms: 1000,
    }
}

#[test]
fn headless_status_transitions() {
    // Starting until the first event that opens a turn; inert lines keep it.
    let other = SessionEvent::Other {
        kind: "system/hook_started".into(),
    };
    assert_eq!(run(&[]).status, Status::Starting);
    assert_eq!(run(std::slice::from_ref(&other)).status, Status::Starting);
    let working = run(&[other, init()]);
    assert_eq!(working.status, Status::Working);
    assert!(working.turn_open);
    assert_eq!(run(&[SessionEvent::TurnStarted]).status, Status::Working);

    // A rate-limit retry needs attention until the next event of any kind.
    let limited = run(&[init(), retry()]);
    assert_eq!(limited.status, Status::Attention);
    assert!(limited.rate_limited);
    let resumed = next(
        &limited,
        &SessionEvent::AssistantText {
            text: "back".into(),
            parent: None,
        },
    );
    assert_eq!(resumed.status, Status::Working);
    assert!(!resumed.rate_limited);

    // Turn ends.
    let idle = run(&[init(), ended(TurnOutcome::Completed)]);
    assert_eq!(idle.status, Status::Idle);
    assert!(!idle.turn_open);
    let failed = run(&[
        init(),
        ended(TurnOutcome::Failed {
            error: "Not logged in".into(),
            kind: FailureKind::Authentication,
        }),
    ]);
    assert_eq!(failed.status, Status::Attention);
    assert!(!failed.turn_open);
    // A stray line after a failure does not hide it; a new turn does.
    let still = next(
        &failed,
        &SessionEvent::StderrLine {
            line: "warning".into(),
        },
    );
    assert_eq!(still.status, Status::Attention);
    assert_eq!(next(&failed, &init()).status, Status::Working);
    assert_eq!(
        run(&[init(), ended(TurnOutcome::Interrupted)]).status,
        Status::Idle
    );

    // A later `Init` (Claude repeats it every turn) is a new turn, never `Starting`.
    let second = run(&[init(), ended(TurnOutcome::Completed), init()]);
    assert_eq!(second.status, Status::Working);
    assert!(second.turn_open);

    // The process ending ends the session, whatever came before.
    for before in [
        vec![],
        vec![init()],
        vec![init(), retry()],
        vec![init(), ended(TurnOutcome::Completed)],
    ] {
        let mut events = before;
        events.push(SessionEvent::ProcessExited {
            code: Some(1),
            signal: None,
        });
        let exited = run(&events);
        assert_eq!(exited.status, Status::Exited);
        assert!(!exited.turn_open);
        assert!(!exited.rate_limited);
    }
}

#[test]
fn tool_follows_the_latest_top_level_tool_use() {
    let state = run(&[init(), tool("Bash", None), tool("Read", None)]);
    assert_eq!(state.tool.as_deref(), Some("Read"));
    // A sub-agent's tool is not the window's.
    let state = next(&state, &tool("Grep", Some("toolu_parent")));
    assert_eq!(state.tool.as_deref(), Some("Read"));
    // Other events keep it; the turn's end clears it.
    let state = next(
        &state,
        &SessionEvent::ToolResult {
            id: "id-Read".into(),
            text: String::new(),
            ok: true,
            parent: None,
        },
    );
    assert_eq!(state.tool.as_deref(), Some("Read"));
    assert_eq!(next(&state, &ended(TurnOutcome::Completed)).tool, None);
}
