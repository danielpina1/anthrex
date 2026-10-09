//! Milestone 9.5 task 20b (ruling T20-1 (d)): the run snapshot a real daemon publishes
//! fills a race's and a pair's fields, with `fake-agent` as both runtimes. A racing
//! task (lane a wins, lane b's racer hangs until it is stopped) and a paired task (the
//! test writer's red check, then the implementer), each through to `complete`.

mod support;

use proto::{AgentRole, LaneState, PairPhase, RaceLane, RunInfo, Runtime, TaskInfo, TaskState};
use support::run_adapt::{hang, seen_task};
use support::run_harness::{RUN_WAIT, RunHarness, RunWatcher};
use support::run_plans::*;

/// `t1` of every snapshot `watcher` received, oldest first.
fn seen(watcher: &RunWatcher, id: &str) -> Vec<TaskInfo> {
    seen_task(watcher, id, "t1")
}

fn lane_rounds(task: &TaskInfo) -> Vec<(AgentRole, Option<RaceLane>)> {
    task.rounds.iter().map(|r| (r.role, r.lane)).collect()
}

/// The built-in medium row (Claude Sonnet, `medium`) with a Codex fallback.
const MEDIUM_FALLS_BACK_TO_CODEX: &str = "[models.implementer.medium]\nmodel = \"claude:claude-sonnet-5\"\neffort = \"medium\"\nfallback = \"codex:default\"\n";

#[test]
fn a_racing_tasks_snapshot_names_its_lanes_and_winner() {
    // Milestone 9.8 (MR §3.1, decision 28): lane b takes the medium row's fallback, so
    // the row falls back to Codex to keep the race across runtimes.
    let h = RunHarness::with_config("", MEDIUM_FALLS_BACK_TO_CODEX, &[]);
    let watcher = h.subscribe();
    h.script(
        "racer-t1-a-1",
        &[commit("a.txt", "a\n"), done("added a in lane a")],
    );
    h.script("racer-t1-b-1", &[hang()]);
    h.script("reviewer-t1-1", &[approve()]);
    let racing = task("t1", &["a.txt"], "race = true").replace("size = \"S\"", "size = \"M\"");
    let id = h.start(&plan("", &[racing]), true);

    // While both lanes race, the snapshot shows both, and each racer its lane.
    let live = support::run_plans::until("both racers", RUN_WAIT, || {
        seen(&watcher, &id).into_iter().find(|t| {
            let racers = (t.rounds.iter()).filter(|r| r.role == AgentRole::Racer);
            racers.count() == 2 && t.race.as_ref().is_some_and(|r| r.winner.is_none())
        })
    });
    let race = live.race.as_ref().unwrap();
    let lanes: Vec<_> = (race.lanes.iter())
        .map(|l| (l.lane, l.checkout.as_str(), l.route.runtime))
        .collect();
    assert_eq!(
        lanes,
        [
            (RaceLane::A, "t1.a", Runtime::Claude),
            (RaceLane::B, "t1.b", Runtime::Codex),
        ]
    );
    let racers: Vec<_> = (live.rounds.iter())
        .filter(|r| r.role == AgentRole::Racer)
        .map(|r| r.lane)
        .collect();
    assert_eq!(racers, [Some(RaceLane::A), Some(RaceLane::B)]);

    let done = |run: &RunInfo| {
        let lost = (t(run, "t1").race.as_ref())
            .and_then(|r| r.lanes.iter().find(|l| l.lane == RaceLane::B))
            .is_some_and(|l| l.state == LaneState::Lost);
        complete(run) && lost
    };
    // Whole-branch review C, I-1: the race rows' bound (`run_e2e_race.rs`): the lanes
    // side by side, then the crown and merge, plus the loser's exit wait and salvage.
    let run = h.wait_run(&id, done, 2 * RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    let race = t1.race.as_ref().expect("the race is still shown");
    assert_eq!((race.winner, race.adopted), (Some(RaceLane::A), false));
    let states: Vec<_> = race.lanes.iter().map(|l| (l.lane, l.state)).collect();
    assert_eq!(
        states,
        [
            (RaceLane::A, LaneState::Won),
            (RaceLane::B, LaneState::Lost)
        ]
    );
    assert!(race.lanes[0].head.is_some(), "{race:?}");
    assert!(!race.lanes[1].kept, "lane b's racer exited when stopped");
    assert_eq!(t1.pair, None);
    // Lane a's reviewer and its review carry lane a.
    assert!(
        lane_rounds(t1).contains(&(AgentRole::Reviewer, Some(RaceLane::A))),
        "{:?}",
        lane_rounds(t1)
    );
    let reviews: Vec<_> = t1.reviews.iter().map(|r| (r.round, r.lane)).collect();
    assert_eq!(reviews, [(1, Some(RaceLane::A))]);
}

/// The profile of a `tdd` task's proof (as `run_e2e_gates.rs`).
fn pair_plan(task: String) -> String {
    format!(
        "goal = \"Pair\"\n\n[profile]\ncheck = \"true\"\nsingle_test = \"sh tests/{{test}}.sh\"\ntest_passed = \"PASS {{test}}\"\n{task}"
    )
}

#[test]
fn a_paired_tasks_snapshot_shows_the_writer_then_the_implementer() {
    let h = RunHarness::new("");
    let watcher = h.subscribe();
    let failing = "mkdir -p tests && printf '%s' 'test -f feature.txt || exit 1\necho PASS t_feat\n' > tests/t_feat.sh && git add -A && git commit -qm 'add t_feat'";
    h.script(
        "test_writer-t1-1",
        &[
            sh(failing),
            capture("red", "git rev-parse HEAD"),
            done_tdd("t_feat", "{{red}}"),
        ],
    );
    h.script(
        "worker-t1-1",
        &[
            commit("feature.txt", "feature\n"),
            done("added the feature"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let paired = task(
        "t1",
        &["tests/**", "feature.txt"],
        "pair = true\ntest_to_write = \"t_feat\"",
    )
    .replace(
        "test_mode = \"check\"\ntest_mode_reason = \"smoke\"\n",
        "test_mode = \"tdd\"\n",
    );
    let id = h.start(&pair_plan(paired), true);
    // Whole-branch review C, I-1: the pair rows' bound (`run_e2e_pair.rs`): the
    // writer's path, then the implementer's (k = 2).
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    let pair = t1.pair.as_ref().expect("the pair");
    assert_eq!(pair.phase, PairPhase::Implementing);
    assert_eq!(pair.test.as_deref(), Some("t_feat"));
    assert_eq!(pair.red_checked, Some(true));
    assert_eq!(pair.writer_failures, 0);
    let writer = (t1.rounds.iter())
        .find(|r| r.role == AgentRole::TestWriter)
        .expect("the test writer's round");
    assert_eq!(writer.route, pair.writer_route);
    assert_eq!(pair.writer_route.runtime, Runtime::Codex);
    let red = pair.red.clone().expect("the red commit");
    assert_eq!(red.len(), 40, "{red}");
    // Nothing of a pair carries a lane, and it is no race.
    assert!(t1.rounds.iter().all(|r| r.lane.is_none()));
    assert_eq!(t1.race, None);

    // While the test writer worked, the snapshot said so, with no red yet.
    let writing: Vec<_> = (seen(&watcher, &id).into_iter())
        .filter_map(|t| t.pair)
        .filter(|p| p.phase == PairPhase::Writing)
        .collect();
    assert!(!writing.is_empty(), "no snapshot showed the writing phase");
    assert!(writing.iter().all(|p| p.red_checked.is_none()));
}
