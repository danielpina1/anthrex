//! Milestone 9.1 task M9.1.21: tiers, the result cache, flaky tests, slots and
//! isolation end to end, through the real binary and a real daemon on `/tmp` paths,
//! with `fake-agent` workers (`support/run_tiers.rs`'s repository and scripts).

mod support;

use support::run_harness::RunHarness;
use support::run_plans::*;
use support::run_tiers::*;

fn harness() -> RunHarness {
    RunHarness::with_config("", "", &tier_repo_files())
}

/// A worker for `task` that commits `file` with `content`, then calls `task_done`; and
/// a reviewer that approves.
fn green_task(h: &RunHarness, task: &str, file: &str, content: &str) {
    h.script(
        &format!("worker-{task}-1"),
        &[commit(file, content), done(&format!("changed {file}"))],
    );
    h.script(&format!("reviewer-{task}-1"), &[approve()]);
}

#[test]
fn e2e_unknown_graph_falls_back_to_check() {
    let h = harness();
    green_task(&h, "t1", "mods/b/src.txt", "b2\n");
    let plan = tier_plan(&h, "", &[task("t1", &["mods/b/src.txt"], "")]).replace(
        "module_graph = \"sh graph.sh\"",
        "module_graph = \"sh -c 'exit 3'\"",
    );
    let id = h.start(&plan, true);
    let run = h.wait_run(&id, complete, TIER_WAIT);
    assert_eq!(t(&run, "t1").state, proto::TaskState::Merged);

    let log = tier_log(&h);
    assert!(
        runs_of(&log, "check.sh")
            .iter()
            .any(|l| in_proof_of(l, "t1")),
        "tier 1 ran check.sh: {log:#?}"
    );
    assert!(runs_of(&log, "test.sh").is_empty(), "{log:#?}");
    let report = report_with(&run, "## Log");
    let log_section = &report[report.find("## Log").unwrap()..];
    assert_eq!(
        log_section.matches("module graph unknown: ").count(),
        1,
        "{report}"
    );
}

#[test]
fn e2e_tier2_is_skipped_on_a_cached_tree() {
    let h = harness();
    green_task(&h, "t1", "mods/b/src.txt", "b2\n");
    // t2 changes the file and changes it back: its merge adds nothing to the stage, so
    // its candidate's tree is the one t1's tier 2 already proved green.
    h.script(
        "worker-t2-1",
        &[
            commit("mods/b/src.txt", "b3\n"),
            commit("mods/b/src.txt", "b2\n"),
            done("changed it back"),
        ],
    );
    h.script("reviewer-t2-1", &[approve()]);
    let tasks = [
        task("t1", &["mods/b/src.txt"], ""),
        task("t2", &["mods/b/src.txt"], "deps = [\"t1\"]"),
    ];
    let id = h.start(&tier_plan(&h, "", &tasks), true);
    // Two task paths one after another (`k = 2`).
    let run = h.wait_run(&id, complete, 2 * TIER_WAIT);
    assert_eq!(t(&run, "t2").state, proto::TaskState::Merged);

    // Every tree a merge made was already proved green by a tier-1 run on the same
    // tree (t1's, at its head): no tier-2 command ran in the integration worktree, for
    // t1 or for t2, whose tree is t1's again.
    let log = tier_log(&h);
    assert!(
        runs_of(&log, "build.sh")
            .iter()
            .any(|l| in_proof_of(l, "t1")),
        "{log:#?}"
    );
    let integration: Vec<_> = log.iter().filter(|l| in_integration(l)).collect();
    assert!(integration.is_empty(), "{integration:#?}");
    let t2_history: Vec<&str> = t(&run, "t2")
        .history
        .iter()
        .map(|e| e.text.as_str())
        .collect();
    assert!(
        t2_history.iter().any(|l| l.starts_with("tier 2: cached (")),
        "{t2_history:?}"
    );
    report_with(&run, "tier 2: cached (");
}

#[test]
fn e2e_flaky_test_costs_no_bounce_and_is_recorded() {
    let h = harness();
    green_task(&h, "t1", "mods/b/FLAKY", "once\n");
    let id = h.start(
        &tier_plan(&h, "", &[task("t1", &["mods/b/FLAKY"], "")]),
        true,
    );
    let run = h.wait_run(&id, complete, TIER_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, proto::TaskState::Merged);
    let workers: Vec<_> = t1
        .rounds
        .iter()
        .filter(|r| r.role == proto::AgentRole::Worker)
        .collect();
    assert_eq!(workers.len(), 1);
    assert!(workers[0].sent_back_at.is_empty(), "{workers:?}");
    assert_eq!(t1.rung, 0);
    // The snapshot keeps the newest 10 history entries; the report has them all.
    report_with(&run, "tier 1: flaky b::flaky passed on retry");

    let flaky = until("the flaky history line", TIER_WAIT, || {
        let lines = flaky_lines(&h);
        (!lines.is_empty()).then_some(lines)
    });
    assert_eq!(flaky.len(), 1, "{flaky:?}");
    assert_eq!(
        (
            flaky[0].run_id.as_str(),
            flaky[0].tier,
            flaky[0].test.as_str()
        ),
        (id.as_str(), 1, "b::flaky")
    );
}

/// Every `flaky` line of the repository's `history.jsonl`.
fn flaky_lines(h: &RunHarness) -> Vec<proto::FlakyRecord> {
    let path = h.repo_dir().join(daemon::run::engine::HISTORY_FILE);
    let (lines, _) = daemon::run::history_io::read_history(&path);
    lines
        .into_iter()
        .filter_map(|l| match l {
            proto::HistoryLine::Flaky(record) => Some(record),
            _ => None,
        })
        .collect()
}

#[test]
fn e2e_run_stats_proposes_a_thrice_flaky_test() {
    let h = harness();
    let plan = tier_plan(&h, "", &[task("t1", &["mods/b/FLAKY"], "")]);
    for n in 1..=3 {
        // A session claims the unclaimed script with the smallest number: each run
        // gets its own. Each run's tree differs, so no result is cached from the last
        // run, and each run's first call of b's tests fails again.
        h.script(
            &format!("worker-t1-{n}"),
            &[commit("mods/b/FLAKY", &format!("run {n}\n")), done("flaky")],
        );
        h.script(&format!("reviewer-t1-{n}"), &[approve()]);
        let _ = std::fs::remove_file(h.cache_dir().join("flaky-b"));
        let id = h.start(&plan, true);
        h.wait_run(&id, complete, TIER_WAIT);
        until("the run's flaky line", TIER_WAIT, || {
            (flaky_lines(&h).len() == n).then_some(())
        });
    }
    let repo = h.repo.display().to_string();
    let out = h.anthrex(&["run", "stats", "--dir", &repo]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.ends_with(
            "Flaky tests (at least 3 runs in the last 14 days):\n\
             proposal: add b::flaky to slow_tests (flaky in 3 runs in the last 14 days)\n  \
             fix: anthrex run start --goal \"Make the test b::flaky deterministic; it failed and then passed on retry in 3 runs\"\n"
        ),
        "{text}"
    );
}

/// Whether `line`'s arguments hold `--filter <expr>` with the timing group's own
/// filter, `(timing)`, rather than a filter that leaves it out (`not (timing)`).
fn is_timing_step(line: &serde_json::Value) -> bool {
    let a = args(line);
    a.windows(2)
        .any(|w| w[0] == "--filter" && w[1].starts_with("(timing)"))
}

fn span(line: &serde_json::Value) -> (f64, f64) {
    (
        line["start"].as_f64().unwrap(),
        line["end"].as_f64().unwrap(),
    )
}

#[test]
fn e2e_timing_group_runs_alone() {
    let h = RunHarness::with_config("", "[testing]\ntest_slots = 4\n", &tier_repo_files());
    // Two tasks in different modules, so their tier-1 jobs can run side by side.
    green_task(&h, "t1", "mods/a/src.txt", "a2\n");
    green_task(&h, "t2", "mods/c/src.txt", "c2\n");
    let tasks = [
        task("t1", &["mods/a/src.txt"], ""),
        task("t2", &["mods/c/src.txt"], ""),
    ];
    let id = h.start(&tier_plan(&h, "timing_tests = \"timing\"", &tasks), true);
    let run = h.wait_run(&id, complete, TIER_WAIT);
    assert!(
        run.tasks
            .iter()
            .all(|t| t.state == proto::TaskState::Merged)
    );

    // The graph command is not a scheduled step; every other line is one.
    let log: Vec<_> = tier_log(&h)
        .into_iter()
        .filter(|l| l["script"] != "graph.sh")
        .collect();
    let timing: Vec<_> = log.iter().filter(|l| is_timing_step(l)).collect();
    assert!(!timing.is_empty(), "no timing step ran: {log:#?}");
    for step in &timing {
        let (start, end) = span(step);
        for other in log.iter().filter(|l| !std::ptr::eq(*l, *step)) {
            let (s, e) = span(other);
            assert!(
                e <= start || end <= s,
                "the timing step {step} overlaps {other}"
            );
        }
    }
    // Every other test step leaves the timing tests out.
    for step in log.iter().filter(|l| !is_timing_step(l)) {
        let a = args(step);
        if a.contains(&"--filter") {
            assert!(a.contains(&"not (timing)"), "{step}");
        }
    }
}

const SLOT_VARS: [&str; 4] = [
    "CARGO_BUILD_JOBS",
    "NEXTEST_TEST_THREADS",
    "RUST_TEST_THREADS",
    "ANTHREX_TEST_SLOTS",
];

#[test]
fn e2e_tier_commands_and_workers_get_the_slot_variables() {
    let h = RunHarness::with_config("", "[testing]\ntest_slots = 4\n", &tier_repo_files());
    let worker_env = h.cache_dir().join("worker-env.txt");
    h.script(
        "worker-t1-1",
        &[
            sh(&format!("env > '{}'", worker_env.display())),
            commit("mods/b/src.txt", "b2\n"),
            done("changed b"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let plan = format!(
        "max_writers = 2\n{}",
        tier_plan(&h, "", &[task("t1", &["mods/b/src.txt"], "")])
    );
    let id = h.start(&plan, true);
    h.wait_run(&id, complete, TIER_WAIT);

    let log: Vec<_> = tier_log(&h)
        .into_iter()
        .filter(|l| l["script"] != "graph.sh")
        .collect();
    assert!(!log.is_empty());
    for step in &log {
        let env = &step["env"];
        let granted: u32 = env["ANTHREX_TEST_SLOTS"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("no slot count: {step}"));
        assert!((1..=4).contains(&granted), "{step}");
        for var in SLOT_VARS {
            assert_eq!(env[var], granted.to_string(), "{var}: {step}");
        }
        let load = env["ANTHREX_TEST_LOAD"].as_str().unwrap_or_default();
        let parsed: f64 = load.parse().unwrap_or_else(|_| panic!("load {load:?}"));
        assert!((0.0..=1.0).contains(&parsed), "{step}");
        assert_eq!(load.split('.').nth(1).map(str::len), Some(1), "{step}");
    }

    // Decision 27: the worker is capped at test_slots / max_writers = 2.
    let env = std::fs::read_to_string(&worker_env).expect("the worker recorded its env");
    for var in SLOT_VARS {
        assert!(
            env.lines().any(|l| l == format!("{var}=2")),
            "{var}:\n{env}"
        );
    }
}

/// Controller ruling 1: an untiered profile runs M8a's gates exactly (no tier op, no
/// tier command), and its check still gets the slot and isolation variables.
#[test]
fn e2e_untiered_profile_runs_m8a_gates() {
    let h = harness();
    green_task(&h, "t1", "a.txt", "a\n");
    let check_env = h.cache_dir().join("check-env.txt");
    let plan = format!(
        "goal = \"Add a\"\n\n[profile]\ncheck = \"env >> '{}'\"\n{}",
        check_env.display(),
        task("t1", &["a.txt"], "")
    );
    let id = h.start(&plan, true);
    let run = h.wait_run(&id, complete, TIER_WAIT);
    assert_eq!(t(&run, "t1").state, proto::TaskState::Merged);
    assert!(!tier_log_path(&h).exists(), "no tier command ran");

    // A one-task M8a run's op kinds (M8b's history and diff ops included): M8a's
    // `Check` for the check gate and the final check, and no `Tier`, `TestAt`,
    // `Propagate` or stage op.
    let kinds = op_kinds(&run.report_path.with_file_name("journal.jsonl"));
    assert_eq!(
        kinds,
        [
            "AppendHistory",
            "Check",
            "CreateRunBranch",
            "CreateWindow",
            "MeasureDiff",
            "MergeCandidate",
            "PrepareReview",
            "PrepareWorktree",
            "RemoveWorktree",
            "VerifyDone",
            "VerifyRefs",
        ]
    );

    let env = std::fs::read_to_string(&check_env).expect("the check recorded its env");
    for var in SLOT_VARS.iter().chain(&["ANTHREX_TEST_LOAD"]) {
        assert!(
            env.lines().any(|l| l.starts_with(&format!("{var}="))),
            "{var}:\n{env}"
        );
    }
    let value = |var: &str| {
        env.lines()
            .find_map(|l| l.strip_prefix(&format!("{var}=")))
            .unwrap_or_else(|| panic!("{var}:\n{env}"))
            .to_string()
    };
    let tmp = value("TMPDIR");
    let tmp = tmp.trim_end_matches('/');
    assert_eq!(value("ANTHREX_SOCKET"), format!("{tmp}/d.sock"));
    assert_eq!(value("ANTHREX_DATA_DIR"), format!("{tmp}/data"));
}

/// The kinds of every op intent the run's journal holds, sorted and deduplicated.
fn op_kinds(journal: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(journal).unwrap_or_default();
    let mut kinds: Vec<String> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| match v.get("intent")? {
            serde_json::Value::Object(map) => map.keys().next().cloned(),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    kinds.sort();
    kinds.dedup();
    kinds
}
