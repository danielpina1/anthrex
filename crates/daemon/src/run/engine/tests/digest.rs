//! Task M9.6: `Run.orch.digest_rev` moves only when the digest's fingerprint does
//! (decision 16): a reducer step sequence on a running run.

use serde_json::json;

use super::turns::working;
use crate::run::engine::AgentSignal;
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
