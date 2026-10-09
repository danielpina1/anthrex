//! Milestone 9.9 task 9: `run status` prints what the orchestrator handled and the
//! question it waits on (OFA §4.6).

use proto::{AskInfo, HandledInfo, OrchestratorInfo};

use super::run_block;
use super::tests::example;

fn orchestrator() -> OrchestratorInfo {
    OrchestratorInfo {
        route: serde_json::from_value(serde_json::json!({
            "runtime": "claude", "model": "", "strength": "frontier", "effort": "high",
        }))
        .expect("a Route"),
        window_id: Some(12),
        live: true,
        started_at: 1,
        plan_submitted: true,
        summary: None,
        notes: vec![],
        wakes: 0,
        wake_held: false,
        stuck: None,
        ask: None,
        handled: vec![],
        handled_total: 0,
    }
}

#[test]
fn status_prints_handled_and_the_pending_question() {
    let mut run = example();
    run.orchestrator = Some(orchestrator());
    let quiet = run_block(&run, 0);
    assert!(!quiet.contains("orchestrator handled"), "{quiet}");
    assert!(!quiet.contains("orchestrator asks"), "{quiet}");

    let o = run.orchestrator.as_mut().unwrap();
    o.handled_total = 2;
    o.handled = vec![
        HandledInfo {
            at: 1,
            op: "retry".into(),
            target: "t2".into(),
            reason: "flaky".into(),
        },
        HandledInfo {
            at: 2,
            op: "override".into(),
            target: "t3".into(),
            reason: "ok".into(),
        },
    ];
    o.ask = Some(AskInfo {
        id: 1,
        question: "tabs or spaces?".into(),
        options: vec!["tabs".into(), "spaces".into()],
        context: String::new(),
        asked_at: 5,
    });
    let out = run_block(&run, 0);
    let lines: Vec<&str> = out.lines().collect();
    let at = lines
        .iter()
        .position(|l| *l == "  orchestrator handled 2")
        .unwrap_or_else(|| panic!("{out}"));
    assert_eq!(
        &lines[at + 1..at + 4],
        [
            "  orchestrator asks: tabs or spaces?",
            "    1. tabs",
            "    2. spaces"
        ],
        "{out}"
    );
}
