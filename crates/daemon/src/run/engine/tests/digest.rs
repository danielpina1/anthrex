//! Task M9.6: `Run.orch.digest_rev` moves only when the digest's fingerprint does
//! (decision 16): a reducer step sequence on a running run.

use proto::HoldState;
use serde_json::json;

use super::fixture::RUN_ID;
use super::turns::working;
use crate::run::engine::{AgentSignal, EventKind, OrchEvent, notes_seq, read_at};
use crate::run::orch::digest::fingerprint;

#[test]
fn digest_revision_moves_only_on_fingerprint_change() {
    let (mut fx, window) = working();
    let rev = fx.run().orch.digest_rev;
    // The start, the approval and the launch changed the digest.
    assert!(rev > 0, "{rev}");
    assert_eq!(fx.run().orch.digest_fp, fingerprint(fx.run()));

    // Counters only: tool calls, activity and a quiet tick move the run's revision,
    // never the digest's.
    let revision = fx.run().revision;
    fx.signal(
        window,
        AgentSignal::ToolUse {
            name: "Read".into(),
            target: None,
        },
    );
    fx.signal(window, AgentSignal::Activity);
    fx.tick();
    assert!(fx.run().revision > revision);
    assert_eq!(fx.run().orch.digest_rev, rev);

    // A block is a state change the orchestrator must see: one bump.
    fx.tool(
        window,
        "task_blocked",
        json!({"kind": "question", "reason": "which endpoint?"}),
    );
    assert_eq!(fx.task("t1").state, proto::TaskState::Blocked);
    assert_eq!(fx.run().orch.digest_rev, rev + 1);
    assert_eq!(fx.run().orch.digest_fp, fingerprint(fx.run()));

    // Nothing new: no bump.
    fx.tick();
    assert_eq!(fx.run().orch.digest_rev, rev + 1);
}

/// M9.6 review fix I-1: a restore that changes a run (a running run is paused) moves
/// its digest too, so an orchestrator's wait sees the change.
#[test]
fn a_restore_that_changes_the_run_moves_the_digest() {
    let (mut fx, _) = working();
    let rev = fx.run().orch.digest_rev;
    super::control_restore::restart(&mut fx, vec![]);
    assert_eq!(fx.run().orch.digest_fp, fingerprint(fx.run()));
    assert!(
        fx.run().orch.digest_rev > rev,
        "{}",
        fx.run().orch.digest_rev
    );
}

/// Milestone 9.7 decision 11 (DH §2.2): `run_status` clones the run at `t0` and its
/// `DigestRead` arrives later. A hold approved between the two was in no answer, so
/// the read is recorded at the clone's time and the next read still shows it.
#[test]
fn a_hold_approved_between_the_clone_and_its_read_is_shown_by_the_next_read() {
    let mut fx = super::gate_holds::held(false);
    super::gate_holds::awaiting(&mut fx);
    let t0 = fx.now;
    let (revision, seq) = (fx.run().orch.digest_rev, notes_seq(fx.run()));

    let reply = fx.reply();
    fx.send(
        t0 + 5,
        EventKind::Orch(OrchEvent::ApproveHold {
            reply,
            run_id: RUN_ID.into(),
            hold: "epic:mail".into(),
        }),
    );
    let hold = &fx.run().orch.gate_holds[0];
    assert_eq!(hold.state, HoldState::Approved);
    assert_eq!(hold.decided_at, Some(t0 + 5));

    fx.send(
        t0 + 9,
        EventKind::Orch(OrchEvent::DigestRead {
            run_id: RUN_ID.into(),
            digest_revision: revision,
            notes_seq: seq,
            at: t0,
        }),
    );

    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    let holds = &digest["gate"]["holds"];
    assert_eq!(
        holds,
        &json!([{"id": "epic:mail", "state": "approved", "tasks": 1}]),
        "{digest}"
    );
    assert_eq!(fx.run().orch.digest_read_at, Some(t0));
}

/// Ruling T3-1: the engine stamps an approval at `s` before it takes the lock; it waits
/// while `run_status` takes its `now` (`s + 1`) and clones the run, and is applied after
/// the clone. The answer showed the hold still awaiting, so the next read must show it.
#[test]
fn an_approval_stamped_before_the_clone_and_applied_after_it_is_shown_by_the_next_read() {
    let mut fx = super::gate_holds::held(false);
    super::gate_holds::awaiting(&mut fx);
    let s = fx.now + 5;
    // run_status: `now` first, then the clone, which misses the approval.
    let now_before_clone = s + 1;
    let (revision, seq) = (fx.run().orch.digest_rev, notes_seq(fx.run()));
    let answer = crate::run::orch::digest::digest(fx.run(), now_before_clone);
    assert_eq!(answer["gate"]["holds"][0]["state"], "awaiting", "{answer}");

    // The approval, stamped `s`, is applied after the clone.
    let reply = fx.reply();
    fx.send(
        s,
        EventKind::Orch(OrchEvent::ApproveHold {
            reply,
            run_id: RUN_ID.into(),
            hold: "epic:mail".into(),
        }),
    );
    assert_eq!(fx.run().orch.gate_holds[0].decided_at, Some(s));

    fx.send(
        s + 2,
        EventKind::Orch(OrchEvent::DigestRead {
            run_id: RUN_ID.into(),
            digest_revision: revision,
            notes_seq: seq,
            at: read_at(now_before_clone),
        }),
    );
    let digest = crate::run::orch::digest::digest(fx.run(), fx.now);
    assert_eq!(
        digest["gate"]["holds"],
        json!([{"id": "epic:mail", "state": "approved", "tasks": 1}]),
        "{digest}"
    );
    assert_eq!(read_at(3), 0, "saturates at the epoch");
}
