//! Decision 39's "a read clears too", through a real daemon socket (`orch_read_rig.rs`):
//! a `run_status` answer clears only the wake notes it showed. A digest over its cap
//! keeps the oldest [`NOTES_TRIMMED`](crate::run::orch::digest) notes, a seq-ordered
//! prefix, so the read's mark is the newest seq of that prefix, not of every note; and
//! only the orchestrator's own read clears anything.

use std::ops::RangeInclusive;

use proto::{AgentRole, BlockReason};
use serde_json::{Value, json};

use super::read_rig::{PLANNER, Rig};
use crate::run::model::Run;
use crate::run::orch::test_support::{block, run_of};
use crate::run::orch::{EpicRecord, PlannerPhase, PlannerSession};

/// The orchestrator's pending notes, `note 1` to `note <n>`, seqs 1 to `n`.
fn notes(run: &mut Run, n: u64) {
    let record = run.orch.orchestrator.as_mut().unwrap();
    record.notes = (1..=n).map(|s| format!("note {s}")).collect();
    record.note_seqs = (1..=n).collect();
    record.last_note_seq = n;
}

/// Blocked tasks with long titles and block texts, enough that the digest is still over
/// its cap once the block texts are cut: the trim reaches the notes cut.
fn over_the_cap(run: &mut Run) {
    for mut task in run_of(80).tasks.into_iter().skip(run.tasks.len()) {
        task.spec.title = "t".repeat(120);
        block(&mut task, BlockReason::Question, &"q".repeat(500));
        run.tasks.push(task);
    }
}

fn texts(seqs: RangeInclusive<u64>) -> Vec<String> {
    seqs.map(|s| format!("note {s}")).collect()
}

fn shown(digest: &Value) -> Vec<String> {
    (digest["notes"].as_array().unwrap().iter())
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

/// The seqs of the notes the engine still holds.
fn held(rig: &Rig) -> Vec<u64> {
    rig.run(|run| run.orch.orchestrator.as_ref().unwrap().note_seqs.clone())
}

/// A trimmed answer shows notes 1 to 5 of 20 and says 15 were cut. The read clears only
/// those 5: the engine keeps 6 to 20, the driver remembers 5 as read, and a wake-up
/// holding all 20 still pastes 6 to 20. Each later read shows the next five, so every
/// note reaches the orchestrator once.
#[tokio::test(flavor = "multi_thread")]
async fn a_trimmed_digest_read_keeps_the_notes_it_cut() {
    let rig = Rig::new(|run, _| {
        over_the_cap(run);
        notes(run, 20);
    })
    .await;
    let (ok, digest) = rig.orch("run_status", json!({})).await;
    assert!(ok, "{digest}");
    assert_eq!(shown(&digest), texts(1..=5));
    assert_eq!(digest["omitted_notes"], json!(15));
    rig.settled().await;
    assert_eq!(
        held(&rig),
        (6..=20).collect::<Vec<_>>(),
        "the cut notes went"
    );
    assert_eq!(rig.runs.wakes.remembered(&rig.run_id), Some(5));
    let all: Vec<(u64, String)> = (1..=20).map(|s| (s, format!("note {s}"))).collect();
    assert_eq!(
        rig.runs.wakes.unpasted_seqs(&rig.run_id, &all),
        Some((6..=20).collect::<Vec<_>>()),
        "the wake-up holding the cut notes would not paste them"
    );

    for (next, omitted) in [(6..=10, Some(10)), (11..=15, Some(5)), (16..=20, None)] {
        let last = *next.end();
        let (ok, digest) = rig.orch("run_status", json!({})).await;
        assert!(ok, "{digest}");
        assert_eq!(shown(&digest), texts(next));
        assert_eq!(digest.get("omitted_notes").and_then(Value::as_u64), omitted);
        rig.settled().await;
        assert_eq!(held(&rig), (last + 1..=20).collect::<Vec<_>>());
        assert_eq!(rig.runs.wakes.remembered(&rig.run_id), Some(last));
    }
    rig.stop().await;
}

/// A live sub-planner of epic `mail` in window [`PLANNER`].
fn with_planner(run: &mut Run) {
    let mut epic = EpicRecord::new("mail", PlannerPhase::Planning);
    epic.area = vec!["crates/m0/**".into()];
    epic.sessions.push(PlannerSession {
        session: 1,
        window_id: Some(PLANNER),
        op: Some(3),
        started_at: 1_000,
        ended_at: None,
        usage: Default::default(),
        rejections: 0,
    });
    run.orch.epics.push(epic);
}

/// Only the orchestrator's read clears its notes. `parse_call` and the MCP gate give no
/// other role `run_status` today; a planner's call made past both still leaves the notes
/// in the engine and marks none read, so a waiting wake-up is neither dropped nor cut
/// (`Wakes::read` is what does both, and it records the mark first).
#[tokio::test(flavor = "multi_thread")]
async fn a_planners_run_status_clears_no_notes() {
    let rig = Rig::new(|run, _| {
        with_planner(run);
        notes(run, 3);
    })
    .await;
    let call = proto::ToolCall {
        run_id: rig.run_id.clone(),
        task_id: None,
        role: AgentRole::Planner,
        window_id: PLANNER,
        tool: "run_status".into(),
        args: json!({}),
        scout_id: None,
        epic: Some("mail".into()),
        chain: None,
        lane: None,
    };
    let reply = rig.runs.run_status(&call).await;
    assert!(
        matches!(reply, proto::RunReply::ToolResult { ok: true, .. }),
        "{reply:?}"
    );
    rig.settled().await;
    assert_eq!(held(&rig), vec![1, 2, 3], "a planner's read cleared notes");
    assert_eq!(rig.runs.wakes.remembered(&rig.run_id), None);
    let all: Vec<(u64, String)> = (1..=3).map(|s| (s, format!("note {s}"))).collect();
    assert_eq!(
        rig.runs.wakes.unpasted_seqs(&rig.run_id, &all),
        Some(vec![1, 2, 3])
    );
    rig.stop().await;
}
