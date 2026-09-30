//! Milestone 9 decision 2: every run reply that answers a request echoes its id.

use std::path::PathBuf;

use super::both_ways;
use crate::adapt::{DeciderSource, RunPath, Scale, TriageInfo};
use crate::history::HistoryStats;
use crate::messages::DaemonMsg;
use crate::run::{RunState, TaskKind};
use crate::run_info::{BaseMovedInfo, RunsSnapshot};
use crate::run_wire::{ProfileReply, RunReply, request};

fn a_triage() -> TriageInfo {
    TriageInfo {
        kinds: vec![TaskKind::Code],
        scale: Scale::Single,
        path: RunPath::Fast,
        reason: "one file".into(),
        source: DeciderSource::Fallback,
        fallback_reason: Some("deciders are off".into()),
        at: 1_700_000_200,
    }
}

fn a_stats() -> HistoryStats {
    HistoryStats {
        path: PathBuf::from("/tmp/data/history.jsonl"),
        task_records: 1,
        run_records: 1,
        rows: Vec::new(),
        decider_calls: 0,
        decider_fallbacks: 0,
        size_checked: 0,
        size_raised: 0,
        problems: Vec::new(),
    }
}

/// One of every reply that answers a `RunRequest`, untagged.
fn every_answer() -> Vec<RunReply> {
    vec![
        RunReply::Started {
            run_id: "r1".into(),
            state: RunState::AwaitingApproval,
            request_id: None,
        },
        RunReply::done(request::APPROVE, "hold h1 approved"),
        RunReply::refused(request::START_GOAL, "not a Git repository"),
        RunReply::ConfirmNeeded {
            run_id: "r1".into(),
            prompt: "type r1@abc to confirm".into(),
            base_moved: Some(BaseMovedInfo {
                from: "aaa".into(),
                to: "abc".into(),
                commits: vec!["abc1234 Ann: fix".into()],
                total: 1,
            }),
            request_id: None,
        },
        RunReply::tool_result(true, "recorded"),
        RunReply::Triaged {
            triage: a_triage(),
            run_id: Some("r2".into()),
            message: "fast path".into(),
            request_id: None,
        },
        RunReply::profile(ProfileReply::Done {
            message: "stored".into(),
        }),
        RunReply::stats(a_stats()),
    ]
}

#[test]
fn every_run_reply_round_trips_its_request_id() {
    for reply in every_answer() {
        assert_eq!(reply.request_id(), None, "{reply:?}");
        let tagged = reply.clone().tagged(Some(7));
        assert_eq!(tagged.request_id(), Some(7), "{reply:?}");
        both_ways(&DaemonMsg::Run(tagged.clone()));
        // Untagging restores the original reply exactly: only the id changed.
        assert_eq!(tagged.tagged(None), reply);
    }
    // A snapshot is state, not an answer: `tagged` leaves it as it is.
    let snapshot = RunReply::Snapshot(RunsSnapshot {
        revision: 3,
        runs: Vec::new(),
        now: 1_700_000_000,
        proposals: Vec::new(),
    });
    assert_eq!(snapshot.clone().tagged(Some(7)), snapshot);
    assert_eq!(snapshot.request_id(), None);
}

/// The brief's `request_id_round_trips`, widened by the M9.2 review: a milestone-8c
/// reply had no `request_id`; each struct-shaped one still decodes, as `None`. (`Profile` and `Stats` were newtype variants and change shape with
/// `PROTO_VERSION` 10, which the handshake enforces.)
#[test]
fn request_id_round_trips() {
    let triage = serde_json::to_value(a_triage()).unwrap();
    let old = [
        serde_json::json!({"Started": {"run_id": "r1", "state": "running"}}),
        serde_json::json!({"Done": {"request": "run edit", "message": "m"}}),
        serde_json::json!({"Refused": {"request": "run edit", "message": "m"}}),
        serde_json::json!({"ConfirmNeeded": {"run_id": "r1", "prompt": "p", "base_moved": null}}),
        serde_json::json!({"ToolResult": {"ok": true, "text": "t"}}),
        serde_json::json!({"Triaged": {"triage": triage, "run_id": null, "message": "m"}}),
    ];
    for value in old {
        let reply: RunReply = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(reply.request_id(), None, "{value}");
    }
}
