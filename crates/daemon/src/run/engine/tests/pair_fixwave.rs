//! The final fix wave's pair items in the reducer: a test writer's writer slot and cap
//! are its own runtime's (A-I3).

use proto::{Runtime, TaskState};

use super::*;
use crate::run::engine::concurrency::writers_busy_on;
use crate::run::engine::schedule::writer_slots;
use crate::run::model::RuntimeConcurrency;

/// Codex's cap brought down to 1 by a rate limit at the fixture's start.
fn codex_capped(run: &mut crate::run::model::Run) {
    let held = RuntimeConcurrency {
        cap: 1,
        last_rate_limit_at: Some(2_000),
        ..RuntimeConcurrency::new(run.limits.max_writers)
    };
    run.concurrency.insert("codex".into(), held);
}

/// A-I3: two paired Claude tasks whose test writers would both run on Codex, with
/// Codex's cap at 1. The first writer takes Codex's one slot, counted on Codex; the
/// second task waits for it, though Claude has room.
#[test]
fn the_peer_runtimes_cap_holds_back_a_second_test_writer() {
    let tasks = [task("t1", "S", "a", PAIRED), task("t2", "S", "b", PAIRED)];
    let plan = plan_with(&profile_with("max_writers = 3"), &tasks);
    let mut fx = Fixture::new(&plan);
    fx.start_with(true, codex_capped);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.launch_all();
    assert!(fx.run().limits.adaptive_concurrency);
    let t1 = fx.task("t1");
    assert_eq!(t1.route.runtime, Runtime::Claude);
    assert_eq!(
        writer_slots(t1),
        [Runtime::Codex],
        "the writer's slot is Codex's"
    );
    assert_eq!(writers_busy_on(fx.run(), Runtime::Codex), 1);
    assert_eq!(writers_busy_on(fx.run(), Runtime::Claude), 0);
    assert_eq!(fx.task("t2").state, TaskState::Queued, "Codex has no room");
    let writers: Vec<String> = (fx.ops("CreateWindow").into_iter())
        .map(|(_, kind)| launch_of(kind))
        .filter(|l| role_of(l) == proto::AgentRole::TestWriter)
        .map(|l| l.name)
        .collect();
    assert_eq!(writers, [format!("{H4}/t1.t1")]);
}
