//! Task M9.15.1: the protocol-23 `history.jsonl` fixture, captured with milestone
//! 9.14's code before milestone 9.15 changes any history type.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::history::HistoryLine;

/// The repository's `history.jsonl` from a `fake-agent` run of 9.14's code: tier lines,
/// one `flaky` line, the task record of `t1` (one failed check, one rejected review
/// round, merged) and the accepted run's record. Paths are scrubbed to
/// `/tmp/anthrex-fixture`.
const M914_HISTORY: &str = include_str!("m914_history.jsonl");

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("a history line is an object")
        .keys()
        .cloned()
        .collect()
}

/// Pinning (task M9.15.1): every line decodes to today's `HistoryLine`, and a task
/// record encodes again with exactly the keys it was captured with.
#[test]
fn m914_history_decodes_today() {
    let (mut tasks, mut runs, mut flaky) = (0, 0, 0);
    for line in M914_HISTORY.lines() {
        let decoded: HistoryLine =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
        match &decoded {
            HistoryLine::Task(_) => {
                tasks += 1;
                let captured: Value = serde_json::from_str(line).unwrap();
                let again = serde_json::to_value(&decoded).unwrap();
                assert_eq!(keys(&again), keys(&captured), "{line}");
            }
            HistoryLine::Run(_) => runs += 1,
            HistoryLine::Flaky(_) => flaky += 1,
            _ => {}
        }
    }
    assert_eq!((tasks, runs, flaky), (1, 1, 1));
}

/// Task M9.15.2: every line of the protocol-23 and milestone-9.3 histories decodes,
/// and a task record from before signatures reads with `signatures` and `owns` empty
/// and writes neither key back.
#[test]
fn old_history_decodes_without_signatures() {
    let mut tasks = 0;
    for text in [M914_HISTORY, include_str!("m93_history.jsonl")] {
        for line in text.lines() {
            let decoded: HistoryLine =
                serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
            if let HistoryLine::Task(task) = &decoded {
                tasks += 1;
                assert!(task.signatures.is_empty(), "{line}");
                assert!(task.owns.is_empty(), "{line}");
                let again = serde_json::to_value(&decoded).unwrap();
                assert!(again.get("signatures").is_none(), "{again}");
                assert!(again.get("owns").is_none(), "{again}");
            }
        }
    }
    assert!(tasks >= 2, "{tasks} task lines");
}
