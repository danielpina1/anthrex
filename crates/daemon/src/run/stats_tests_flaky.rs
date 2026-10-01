//! Milestone 9.1 task M9.1.18: decision 34's quarantine proposals in `run stats`, and
//! history written before milestone 9.1 still aggregating as it did.

use std::path::Path;

use proto::{FlakyProposal, FlakyRecord, HISTORY_VERSION, HistoryLine, HistoryStats};

use super::{aggregate, flaky_proposals, render};

const DAY: u64 = 86_400;
const NOW: u64 = 1_800_000_000;

fn flaky(run: &str, op: u64, test: &str, at: u64) -> HistoryLine {
    HistoryLine::Flaky(FlakyRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run}/flaky/{op}/{test}"),
        at,
        run_id: run.into(),
        task_id: Some("t1".into()),
        tier: 1,
        test: test.into(),
    })
}

#[test]
fn third_flaky_run_is_proposed_for_quarantine() {
    let three = vec![
        flaky("r1", 4, "a::flaky", NOW - 10 * DAY),
        flaky("r2", 9, "a::flaky", NOW - 3 * DAY),
        // Twice in one run counts as one run.
        flaky("r2", 12, "a::flaky", NOW - 3 * DAY + 5),
        flaky("r3", 7, "a::flaky", NOW - DAY),
        flaky("r3", 7, "b::other", NOW - DAY),
    ];
    assert_eq!(
        flaky_proposals(&three, NOW, 14, 3),
        [FlakyProposal {
            test: "a::flaky".into(),
            runs: 3,
            last_at: NOW - DAY,
        }]
    );

    // Two runs: none.
    assert!(flaky_proposals(&three[1..], NOW, 14, 3).is_empty());

    // Three runs, one of them outside the window: none.
    let mut old = three.clone();
    old[0] = flaky("r1", 4, "a::flaky", NOW - 15 * DAY);
    assert!(flaky_proposals(&old, NOW, 14, 3).is_empty());
    // The edge of the window is inside it.
    old[0] = flaky("r1", 4, "a::flaky", NOW - 14 * DAY);
    assert_eq!(flaky_proposals(&old, NOW, 14, 3).len(), 1);
}

/// Decision 34: the most flaky test first; ties by name.
#[test]
fn proposals_are_ordered_by_runs_then_name() {
    let mut lines = Vec::new();
    for (k, run) in ["r1", "r2", "r3", "r4"].iter().enumerate() {
        let at = NOW - k as u64;
        lines.push(flaky(run, 1, "z::most", at));
        if k < 3 {
            lines.push(flaky(run, 1, "b::three", at));
            lines.push(flaky(run, 1, "a::three", at));
        }
    }
    let names: Vec<(String, u32)> = flaky_proposals(&lines, NOW, 14, 3)
        .into_iter()
        .map(|p| (p.test, p.runs))
        .collect();
    assert_eq!(
        names,
        [
            ("z::most".to_string(), 4),
            ("a::three".to_string(), 3),
            ("b::three".to_string(), 3)
        ]
    );
}

fn with_proposal() -> HistoryStats {
    let mut stats = aggregate(&[], Path::new("/tmp/h/history.jsonl"));
    stats.window_days = 14;
    stats.quarantine_after = 3;
    stats.flaky_proposals = vec![FlakyProposal {
        test: "a::flaky".into(),
        runs: 3,
        last_at: NOW,
    }];
    stats
}

#[test]
fn stats_prints_the_proposal_and_the_fix_command() {
    let stats = with_proposal();
    let text = render(&stats);
    let plain = render(&aggregate(&[], Path::new("/tmp/h/history.jsonl")));
    let block = text.strip_prefix(&plain).expect("after M8b's lines");
    assert_eq!(
        block,
        "Flaky tests (at least 3 runs in the last 14 days):\n\
         proposal: add a::flaky to slow_tests (flaky in 3 runs in the last 14 days)\n  \
         fix: anthrex run start --goal \"Make the test a::flaky deterministic; it failed and then passed on retry in 3 runs\"\n"
    );
    // No proposal, nothing printed.
    let mut none = stats.clone();
    none.flaky_proposals.clear();
    assert_eq!(render(&none), plain);
}

#[test]
fn stats_json_carries_flaky_proposals() {
    let json = serde_json::to_value(with_proposal()).unwrap();
    assert_eq!(
        json["flaky_proposals"],
        serde_json::json!([{"test": "a::flaky", "runs": 3, "last_at": NOW}])
    );
    assert_eq!(
        (
            json["window_days"].clone(),
            json["quarantine_after"].clone()
        ),
        (serde_json::json!(14), serde_json::json!(3))
    );

    // `summarise` reads the file and the window from `[testing]`.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let mut text = String::new();
    for run in ["r1", "r2", "r3"] {
        let line = flaky(run, 2, "a::flaky", NOW - DAY);
        text.push_str(&serde_json::to_string(&line).unwrap());
        text.push('\n');
    }
    std::fs::write(&path, text).unwrap();
    let testing = config::Testing {
        flaky_window_days: 7,
        flaky_quarantine_after: 3,
        ..config::Testing::default()
    };
    let stats = summarise_at(dir.path(), &path, &testing);
    assert_eq!((stats.window_days, stats.quarantine_after), (7, 3));
    assert_eq!(stats.flaky_proposals.len(), 1, "{stats:#?}");
    assert_eq!(stats.flaky_proposals[0].runs, 3);
    let json = serde_json::to_value(&stats).unwrap();
    assert_eq!(json["flaky_proposals"][0]["test"], "a::flaky");
}

/// `summarise` at `NOW` in `dir` with a `git` that does not exist.
fn summarise_at(dir: &Path, path: &Path, testing: &config::Testing) -> HistoryStats {
    let git = std::ffi::OsStr::new("/nonexistent/anthrex-test/git");
    let timeout = std::time::Duration::from_secs(5);
    crate::run::history_io::summarise(git, dir, path, NOW, timeout, testing)
}

/// Pinning: a history written by milestones 8b (version 1) and 9 (version 2) still
/// aggregates as it did, and milestone 9.1's lines change none of its aggregates.
#[test]
fn v2_history_still_aggregates() {
    let v2 = include_str!("../../../proto/src/m9_history_v2.jsonl");
    let task_v2 = v2.lines().next().expect("the fixture's task line");
    // The same task as milestone 8b wrote it: version 1, another run.
    let task_v1 = task_v2
        .replace("\"v\":2", "\"v\":1")
        .replace("run-a1b2", "run-old1");
    let revert_v1 = r#"{"type":"revert","v":1,"record_id":"revert/ffff","at":1700002000,"run_id":"run-old1","task_id":"t1","reverted":"aaaa","revert_commit":"ffff"}"#;
    let old = format!("{task_v1}\n{revert_v1}\n{v2}");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    std::fs::write(&path, &old).unwrap();
    let testing = config::Testing::default();
    let before = summarise_at(dir.path(), &path, &testing);
    assert!(before.problems.is_empty(), "{:?}", before.problems);
    assert_eq!((before.task_records, before.run_records), (2, 0));
    let m = before.rows.iter().find(|r| r.class == "M").unwrap();
    assert_eq!((m.tasks, m.merged, m.reverted), (2, 2, 1));
    assert!(before.flaky_proposals.is_empty());
    assert_eq!(
        (before.window_days, before.quarantine_after),
        (testing.flaky_window_days, testing.flaky_quarantine_after)
    );

    // The same file with version-3 lines appended: the M8b aggregates do not move.
    let mut newer = old.clone();
    for line in [
        flaky("run-new", 3, "a::flaky", NOW),
        serde_json::from_str(TIER_LINE).unwrap(),
        serde_json::from_str(BISECT_LINE).unwrap(),
    ] {
        newer.push_str(&serde_json::to_string(&line).unwrap());
        newer.push('\n');
    }
    std::fs::write(&path, newer).unwrap();
    let after = summarise_at(dir.path(), &path, &testing);
    assert!(after.problems.is_empty(), "{:?}", after.problems);
    assert_eq!(after.rows, before.rows);
    assert_eq!(
        (after.task_records, after.run_records, after.decider_calls),
        (
            before.task_records,
            before.run_records,
            before.decider_calls
        )
    );
    assert_eq!(
        (after.size_checked, after.size_raised),
        (before.size_checked, before.size_raised)
    );
}

const TIER_LINE: &str = r#"{"type":"tier","v":3,"record_id":"run-new/tier/9","at":1800000000,"run_id":"run-new","task_id":"t1","stage":1,"tier":2,"secs":4,"affected":1,"full_reason":null,"cache_hit":false,"cached_steps":0,"steps":2,"ok":true,"flaky":["a::flaky"]}"#;
const BISECT_LINE: &str = r#"{"type":"bisect","v":3,"record_id":"run-new/bisect/1/1","at":1800000000,"run_id":"run-new","stage":1,"head":"cccc","tests":["a::works"],"range":3,"probes":3,"culprit":"t2","reason":null,"fix_task":"fix1"}"#;

/// Review S2 and S5: `last_at` is the latest time whatever the lines' order, and `runs`
/// is the count itself, not the threshold.
#[test]
fn a_proposal_counts_every_run_and_keeps_the_latest_time() {
    let lines = vec![
        flaky("r3", 1, "a::flaky", NOW - DAY),
        flaky("r1", 1, "a::flaky", NOW - 2 * DAY),
        flaky("r4", 1, "a::flaky", NOW - 5 * DAY),
        flaky("r2", 1, "a::flaky", NOW - 3 * DAY),
    ];
    assert_eq!(
        flaky_proposals(&lines, NOW, 14, 3),
        [FlakyProposal {
            test: "a::flaky".into(),
            runs: 4,
            last_at: NOW - DAY,
        }]
    );
}

/// Ruling C-23: a line more than 300 s in the future is ignored; 300 s is kept.
#[test]
fn a_flaky_line_from_the_future_is_ignored() {
    let at = |ahead: u64| {
        vec![
            flaky("r1", 1, "a::flaky", NOW - DAY),
            flaky("r2", 1, "a::flaky", NOW - DAY),
            flaky("r3", 1, "a::flaky", NOW + ahead),
        ]
    };
    let kept = flaky_proposals(&at(300), NOW, 14, 3);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].last_at, NOW + 300);
    assert!(flaky_proposals(&at(301), NOW, 14, 3).is_empty());
}

/// Ruling C-23: a name a shell would read inside `"…"` is single-quoted instead; a
/// plain name keeps the brief's text.
#[test]
fn the_fix_command_quotes_a_name_the_shell_would_read() {
    let mut stats = with_proposal();
    stats.flaky_proposals[0].test = "a\"b$c'd".into();
    let text = render(&stats);
    let fix = text.lines().last().unwrap();
    assert_eq!(
        fix,
        "  fix: anthrex run start --goal 'Make the test a\"b$c'\\''d deterministic; it failed and then passed on retry in 3 runs'"
    );
    let mut stats = with_proposal();
    stats.flaky_proposals[0].test = "tests::a_b-c/d.e[1]".into();
    let fix = render(&stats).lines().last().unwrap().to_string();
    assert_eq!(
        fix,
        "  fix: anthrex run start --goal \"Make the test tests::a_b-c/d.e[1] deterministic; it failed and then passed on retry in 3 runs\""
    );
}

/// Ruling C-24: a name with a raw newline cannot break the `proposal:` line in two.
#[test]
fn a_proposal_line_is_one_line_whatever_the_name() {
    let mut stats = with_proposal();
    stats.flaky_proposals[0].test = "a\nb\rc".into();
    let text = render(&stats);
    let proposal = text.lines().find(|l| l.starts_with("proposal: ")).unwrap();
    assert_eq!(
        proposal,
        "proposal: add a b c to slow_tests (flaky in 3 runs in the last 14 days)"
    );
    // The fix command keeps its single quotes around the name as it is.
    assert!(
        text.contains("--goal 'Make the test a\nb\rc deterministic;"),
        "{text}"
    );
}
