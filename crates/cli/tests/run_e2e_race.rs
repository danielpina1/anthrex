//! Milestone 9.5 task M9.5.22: racing end to end (decisions 18 to 23, spec §21),
//! through a real daemon with `fake-agent` as both runtimes. Lane a races on Claude,
//! lane b on Codex. A loser is stopped and salvaged; two lanes that pass together
//! crown exactly one; on a tiered profile each lane runs tier 1 and the crowned task
//! alone runs tier 2 (ruling RR-3).

mod support;

use proto::{AgentRole, LaneInfo, LaneState, RaceInfo, RaceLane, RunInfo, TaskInfo, TaskState};
use serde_json::Value;
use support::run_adapt::{argv_of, hang};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;
use support::run_tiers::*;

/// An M code task `t1` owning `owns`, with `race = true`.
fn racing(owns: &[&str]) -> String {
    task("t1", owns, "race = true").replace("size = \"S\"", "size = \"M\"")
}

/// A plan with no `[profile]`: the stored profile is the run's.
fn plan_of(tasks: &[String]) -> String {
    format!("goal = \"Race\"\n{}", tasks.concat())
}

fn race(task: &TaskInfo) -> &RaceInfo {
    task.race.as_ref().expect("the task raced")
}

fn lane(task: &TaskInfo, lane: RaceLane) -> &LaneInfo {
    (race(task).lanes.iter())
        .find(|l| l.lane == lane)
        .expect("the lane")
}

/// The run is complete and lane `l`'s salvage has come back.
fn complete_and_salvaged(l: RaceLane) -> impl Fn(&RunInfo) -> bool {
    move |run| {
        complete(run)
            && (t(run, "t1").race.as_ref())
                .and_then(|r| r.lanes.iter().find(|x| x.lane == l))
                .is_some_and(|x| x.salvage_ref.is_some())
    }
}

/// The parents of every merge commit on the run's integration branch.
fn merges(h: &RunHarness, id: &str) -> Vec<String> {
    let text = h.git(&[
        "log",
        "--merges",
        "--format=%P",
        &format!("anthrex/{id}/integration"),
    ]);
    text.lines().map(str::to_string).collect()
}

/// Two reviewers that meet before either answers: each marks its own file, waits for
/// the other's (at most `RELEASE_POLLS` polls), then approves. Both lanes are therefore
/// in review before either passes, and the two approvals land together (Review focus
/// 3).
fn meeting_reviewers(h: &RunHarness) {
    for (me, other) in [("r1", "r2"), ("r2", "r1")] {
        let n = &me[1..];
        h.script(
            &format!("reviewer-t1-{n}"),
            &[h.mark_release(me), h.wait_release(other), approve()],
        );
    }
}

/// Both meeting reviewers ran: each one's mark is there (a misnamed script would leave
/// its reviewer's mark missing, and the other would approve after its wait alone).
fn reviewers_met(h: &RunHarness) {
    for me in ["r1", "r2"] {
        assert!(h.release_path(me).exists(), "reviewer {me} never marked");
    }
}

/// Task `t1`'s rounds of `role` as `run.json` recorded them, with their lane.
fn rounds_json(h: &RunHarness, id: &str, role: &str) -> Vec<Value> {
    let path = h.data().join("runs").join(id).join("run.json");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    (json["tasks"][0]["rounds"].as_array().into_iter().flatten())
        .filter(|r| r["role"] == role)
        .cloned()
        .collect()
}

#[test]
fn e2e_race_loser_is_stopped_and_salvaged() {
    let h = RunHarness::tuning("", "", &[]);
    h.script(
        "racer-t1-a-1",
        &[commit("a.txt", "a\n"), done("added a in lane a")],
    );
    h.script(
        "racer-t1-b-1",
        &[
            commit("b-note.txt", "b\n"),
            sh("echo unfinished > wip.txt"),
            h.mark_release("b-wip"),
            hang(),
        ],
    );
    // Lane a's reviewer approves only once lane b has committed and written `wip.txt`,
    // so the salvage has both whatever the two racers' speeds (review m1).
    h.script("reviewer-t1-1", &[h.wait_release("b-wip"), approve()]);
    let base = h.git(&["rev-parse", "HEAD"]);
    let id = h.start(
        &plan_of(&[racing(&["a.txt", "b-note.txt", "wip.txt"])]),
        true,
    );
    let run = h.wait_run(&id, complete_and_salvaged(RaceLane::B), 2 * RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(race(t1).winner, Some(RaceLane::A));
    assert_eq!(lane(t1, RaceLane::A).state, LaneState::Won);
    let b = lane(t1, RaceLane::B);
    assert_eq!(b.state, LaneState::Lost);
    assert!(!b.kept, "lane b's racer exited when stopped: {b:?}");

    // The task merged with lane a's head as its merge parent.
    let head_a = lane(t1, RaceLane::A).head.clone().expect("lane a's head");
    assert_eq!(merges(&h, &id), [format!("{base} {head_a}")]);

    // Lane b's work, committed and not, is under its salvage ref.
    let salvage = b.salvage_ref.clone().unwrap();
    let prefix = format!("refs/anthrex/salvage/{id}/t1/");
    assert_eq!(salvage, format!("{prefix}1"), "the first salvage number");
    let files = h.git(&["ls-tree", "-r", "--name-only", &salvage]);
    for file in ["wip.txt", "b-note.txt"] {
        assert!(files.lines().any(|l| l == file), "{file} in {files}");
    }
    assert!(
        !files.lines().any(|l| l == "a.txt"),
        "lane a's work is not lane b's: {files}"
    );

    // Lane b's checkout and its private directory are gone.
    let checkout_b = t1.worktree.with_file_name("t1.b");
    assert_eq!(
        t1.worktree.file_name().unwrap(),
        "t1.a",
        "the crowned task's checkout is lane a's"
    );
    assert!(!checkout_b.exists(), "{}", checkout_b.display());
    let private_b = h.data().join("runs").join(&id).join("tasks").join("t1.b");
    assert!(!private_b.exists(), "{}", private_b.display());

    // No fake-agent runs with lane b's racer's recorded pid (that pid read only).
    let racers_b: Vec<Value> = (rounds_json(&h, &id, "racer").into_iter())
        .filter(|r| r["lane"] == "b")
        .collect();
    assert_eq!(racers_b.len(), 1, "{racers_b:?}");
    let racer = &racers_b[0];
    let pids: Vec<u32> = [&racer["pid"], &racer["closed_pid"], &racer["exited_pid"]]
        .iter()
        .filter_map(|p| p.as_u64())
        .map(|p| u32::try_from(p).unwrap())
        .collect();
    assert!(
        !pids.is_empty(),
        "lane b's racer recorded a pid: {racers_b:?}"
    );
    // Review m3: a recycled pid could be another test's `fake-agent`; lane b's racer
    // is the one whose argv names this run (its anthrex server's `--run <id>`, which
    // its recorded argv shows).
    let launched = h.io_lines("racer-t1-b-1", "args").concat();
    assert!(
        launched.contains(&id),
        "lane b's argv names the run: {launched}"
    );
    for pid in pids {
        if let Some(argv) = argv_of(pid) {
            let ours = argv.contains("fake-agent") && argv.contains(&id);
            assert!(!ours, "lane b's racer, pid {pid}, is alive: {argv}");
        }
    }

    // The report names the salvage ref on lane b's line.
    let report = report_with(&run, &salvage);
    assert!(
        (report.lines())
            .any(|l| l.starts_with("- racer b: ") && l.contains(&format!("; salvaged `{salvage}`"))),
        "lane b's line: {report}"
    );
}

#[test]
fn e2e_race_both_pass_crowns_one() {
    let h = RunHarness::tuning("", "", &[]);
    h.script(
        "racer-t1-a-1",
        &[commit("a.txt", "from a\n"), done("added a in lane a")],
    );
    h.script(
        "racer-t1-b-1",
        &[commit("a.txt", "from b\n"), done("added a in lane b")],
    );
    meeting_reviewers(&h);
    let base = h.git(&["rev-parse", "HEAD"]);
    let id = h.start(&plan_of(&[racing(&["a.txt"])]), true);
    let run = h.wait_run(
        &id,
        |r| {
            complete(r)
                && (t(r, "t1").race.as_ref()).is_some_and(|race| {
                    race.lanes
                        .iter()
                        .any(|l| l.state == LaneState::Lost && l.salvage_ref.is_some())
                })
        },
        2 * RUN_WAIT,
    );
    let t1 = t(&run, "t1");
    reviewers_met(&h);
    assert_eq!(t1.state, TaskState::Merged);
    let winner = race(t1).winner.expect("a winner");
    let loser = match winner {
        RaceLane::A => RaceLane::B,
        RaceLane::B => RaceLane::A,
    };
    assert!(!race(t1).adopted);
    assert_eq!(lane(t1, winner).state, LaneState::Won);
    assert_eq!(lane(t1, loser).state, LaneState::Lost);

    // Both lanes were in review: each had a reviewer round of its own.
    for l in [RaceLane::A, RaceLane::B] {
        assert!(
            (t1.rounds.iter()).any(|r| r.role == AgentRole::Reviewer && r.lane == Some(l)),
            "lane {l:?} had a reviewer: {:?}",
            t1.rounds
        );
    }

    // Exactly one merge, of the winner's head.
    let head_w = lane(t1, winner).head.clone().expect("the winner's head");
    assert_eq!(merges(&h, &id), [format!("{base} {head_w}")]);
    let content = h.git(&["show", &format!("anthrex/{id}/integration:a.txt")]);
    let expected = match winner {
        RaceLane::A => "from a",
        RaceLane::B => "from b",
    };
    assert_eq!(content, expected);

    // The loser's checkout was clean: its salvage ref is at its head (`keep_head`).
    let l = lane(t1, loser);
    let head_l = l.head.clone().expect("the loser's head");
    let salvage = l.salvage_ref.clone().unwrap();
    assert_eq!(h.git(&["rev-parse", &salvage]), head_l, "{salvage}");
}

#[test]
fn e2e_race_on_a_tiered_profile() {
    let h = RunHarness::with_config("", "", &tier_repo_files());
    h.script(
        "racer-t1-a-1",
        &[
            commit("mods/b/src.txt", "b from a\n"),
            done("changed b in a"),
        ],
    );
    h.script(
        "racer-t1-b-1",
        &[
            commit("mods/b/src.txt", "b from b\n"),
            done("changed b in b"),
        ],
    );
    meeting_reviewers(&h);
    let tasks = [racing(&["mods/b/src.txt"])];
    let id = h.start(&tier_plan(&h, "", &tasks), true);
    let run = h.wait_run(
        &id,
        |r| {
            complete(r)
                && (t(r, "t1").race.as_ref()).is_some_and(|race| {
                    race.lanes
                        .iter()
                        .all(|l| l.salvage_ref.is_some() || l.state == LaneState::Won)
                })
        },
        2 * TIER_WAIT,
    );
    let t1 = t(&run, "t1");
    reviewers_met(&h);
    assert_eq!(t1.state, TaskState::Merged);

    // Each lane ran tier 1 in its own proof checkout, each command on a slot grant.
    let log = tier_log(&h);
    for l in ["t1.a", "t1.b"] {
        let tier1: Vec<&Value> = (log.iter())
            .filter(|line| line["script"] != "graph.sh" && in_proof_of(line, l))
            .collect();
        assert!(!tier1.is_empty(), "{l} ran no tier 1: {log:#?}");
        for line in tier1 {
            let granted: u32 = (line["env"]["ANTHREX_TEST_SLOTS"].as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            assert!(granted >= 1, "{l}'s tier 1 ran outside the slots: {line}");
        }
    }

    // The task's check records: one tier-1 job per lane, and one tier-2 job, the
    // crowned task's.
    let path = h.data().join("runs").join(&id).join("run.json");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let checks: Vec<&Value> = (json["tasks"][0]["checks"].as_array().into_iter().flatten())
        .filter(|c| c["tier"].is_object())
        .collect();
    let of = |tier: u64| -> Vec<&Value> {
        (checks.iter().copied())
            .filter(|c| c["tier"]["tier"] == tier)
            .collect()
    };
    for l in ["a", "b"] {
        assert!(
            of(1).iter().any(|c| c["lane"] == l),
            "lane {l}'s tier 1: {checks:#?}"
        );
    }
    let tier2 = of(2);
    assert_eq!(tier2.len(), 1, "{checks:#?}");
    assert!(
        tier2[0]["lane"].is_null(),
        "tier 2 is the task's: {:?}",
        tier2[0]
    );
    // Its candidate's tree is the crowned lane's, which that lane's tier 1 proved: the
    // tier-2 job may find every step cached, so it is counted by its record.
    assert_eq!(tier2[0]["tier"]["ok"], true, "{:?}", tier2[0]);
}
