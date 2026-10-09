//! Milestone 9.9 task 9: `run status` prints what the orchestrator handled and the
//! question it waits on (OFA §4.6).

use proto::{AskInfo, HandledInfo, OrchestratorInfo};

use super::tests::example;
use super::{printable, run_block};

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

/// Task 9 M2: the ask's question and options are daemon-supplied text from a model; a
/// line break, an escape sequence, a bidi override and a zero-width joiner each stay
/// off the terminal, and the question and every option keep their one line, whether or
/// not `printable` runs after. Mutant: `one_line` removed from either, red.
#[test]
fn status_ask_lines_are_sanitised() {
    let mut run = example();
    let mut o = orchestrator();
    o.ask = Some(AskInfo {
        id: 1,
        question: "tabs\nor\x1b[2J spa\u{202E}ces\u{200D}?\x07".into(),
        options: vec![
            "ta\u{202E}bs\x1b]0;pwn\x07\nsecond".into(),
            "sp\u{200D}aces\r\x1b[31m".into(),
        ],
        context: String::new(),
        asked_at: 5,
    });
    run.orchestrator = Some(o);
    let raw = run_block(&run, 0);
    // The block alone: every control character a space, so the ask keeps its lines.
    assert!(!raw.contains(['\x1b', '\x07', '\r']), "{raw:?}");
    // What the terminal is given: the hidden format characters gone too.
    let shown = printable(&raw);
    assert!(
        !shown.contains(['\x1b', '\x07', '\r', '\u{202E}', '\u{200D}']),
        "{shown:?}"
    );
    for out in [raw, shown] {
        let lines: Vec<&str> = out.lines().collect();
        let at = lines
            .iter()
            .position(|l| l.starts_with("  orchestrator asks: "))
            .unwrap_or_else(|| panic!("{out:?}"));
        let plain = |l: &str| {
            l.replace(['\u{202E}', '\u{200D}'], "")
                .trim_end()
                .to_owned()
        };
        assert_eq!(
            plain(lines[at]),
            "  orchestrator asks: tabs or [2J spaces?",
            "{out:?}"
        );
        assert_eq!(
            plain(lines[at + 1]),
            "    1. tabs ]0;pwn  second",
            "{out:?}"
        );
        assert_eq!(plain(lines[at + 2]), "    2. spaces  [31m", "{out:?}");
        assert!(lines[at + 3].starts_with("  ID "), "{out:?}");
    }
}
