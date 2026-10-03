//! Milestone 9.5 task 11: `anthrex run stats` prints the tuning block (decision 11),
//! applies or dismisses a proposal on confirmation, and a read-only `Stats` (decision
//! 48, what the Settings screen sends) records and writes nothing. The history is the
//! recorded `refit.jsonl` (task M9.5.8), copied into the harness's repository data
//! directory; the daemon is the harness's own, and no agent session ever starts.

mod support;

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

use daemon::run::engine::HISTORY_FILE;
use daemon::run::tuning_io::{Loaded, TUNING_FILE, load, save};
use proto::{
    Budget, ClassBudget, FlakyRecord, HISTORY_VERSION, HistoryLine, HistoryStats, RefitState,
    RunRecord, RunReply, RunRequest, SizeThresholds, TuningFile,
};
use support::run_harness::{RunHarness, git_in};

/// The recorded history of task M9.5.8: S qualifies with 34 samples.
fn refit_history() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../daemon/tests/fixtures/history/refit.jsonl")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        stdout(out),
        stderr(out)
    );
}

/// The harness's repository, as the daemon names it (the main checkout, canonical).
fn project(h: &RunHarness) -> PathBuf {
    h.repo.canonicalize().unwrap()
}

/// The repository's data directory, created.
fn repo_dir(h: &RunHarness) -> PathBuf {
    let dir = daemon::profile::repo_dir(&h.data(), &project(h));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `refit.jsonl` as the repository's history, then `extra` lines.
fn seed(h: &RunHarness, extra: &[HistoryLine]) -> PathBuf {
    let path = repo_dir(h).join(HISTORY_FILE);
    let mut text = std::fs::read_to_string(refit_history()).unwrap();
    for line in extra {
        text.push_str(&serde_json::to_string(line).unwrap());
        text.push('\n');
    }
    std::fs::write(&path, text).unwrap();
    path
}

/// The test failed and then passed in three runs, within the flaky window.
fn flaky_lines() -> Vec<HistoryLine> {
    (1..=3)
        .map(|i| {
            HistoryLine::Flaky(FlakyRecord {
                v: HISTORY_VERSION,
                record_id: format!("f{i}/flaky/1"),
                at: now() - 100 * i,
                run_id: format!("f{i}"),
                task_id: Some("t1".into()),
                tier: 1,
                test: "suite::flaky_one".into(),
            })
        })
        .collect()
}

/// `tuning.toml` as the daemon wrote it.
fn tuning(h: &RunHarness) -> TuningFile {
    match load(&repo_dir(h), now()).unwrap() {
        Loaded::File(file) => file,
        other => panic!("no tuning.toml: {other:?}"),
    }
}

fn budget(tool_calls: u32, minutes: u32) -> Budget {
    Budget {
        tool_calls,
        minutes,
        tokens: None,
    }
}

/// `yyyy-mm-dd hh:mm` (the brief's `\d{4}-\d{2}-\d{2} \d{2}:\d{2}`).
fn is_minute(text: &str) -> bool {
    let shape = "dddd-dd-dd dd:dd";
    text.len() == shape.len()
        && text
            .chars()
            .zip(shape.chars())
            .all(|(c, s)| if s == 'd' { c.is_ascii_digit() } else { c == s })
}

fn stats(h: &RunHarness, extra: &[&str], input: &str) -> Output {
    let repo = h.repo.display().to_string();
    let mut args = vec!["run", "stats", "--dir", &repo];
    args.extend_from_slice(extra);
    h.anthrex_input(&args, input)
}

/// Whether `text` lists the proposal `id` under `tuning proposals:`.
fn lists(text: &str, id: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with(&format!("  {id}  ")) || l.starts_with(&format!("  {id} ")))
}

#[test]
fn stats_prints_the_tuning_block_for_recorded_history() {
    let h = RunHarness::new("");
    seed(&h, &flaky_lines());
    let out = stats(&h, &[], "");
    ok(&out);
    let text = stdout(&out);
    // M8b's table, the flaky block, then the tuning block.
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle:?} in:\n{text}"))
    };
    let table = at("\nCLASS  TASKS");
    let flaky = at("\nFlaky tests (at least 3 runs");
    let block = at("\ntuning: ");
    assert!(table < flaky && flaky < block, "{text}");
    let tuning_path = repo_dir(&h).join(TUNING_FILE);
    let rows: Vec<&str> = text[block + 1..].lines().collect();
    assert_eq!(
        rows[0],
        format!(
            "tuning: {}  (refit after 30 samples per class)",
            tuning_path.display()
        )
    );
    assert_eq!(rows[1], "  CLASS  SAMPLES  BUDGET          WEIGHT  REFIT");
    let s = rows[2]
        .strip_prefix("  S      34       55 calls 18m    9m      ")
        .unwrap_or_else(|| panic!("the S row: {text}"));
    assert!(is_minute(s), "the S row's refit date: {s:?}");
    assert_eq!(
        rows[3..].join("\n"),
        "  M      12/30    150 calls 60m   27m*    not yet
  hub    3/30     150 calls 60m   27m*    not yet
  * derived from another class's median
tuning proposals:
  thresholds.s  S line threshold 20 → 35 (p90 of 34 merged S tasks)
  route.s       S route standard/low → standard/medium (14 of 34 S tasks, 41%, reached rung 2 or higher)
apply with anthrex run stats --apply <id>; dismiss with anthrex run stats --dismiss <id>"
    );
    let file = tuning(&h);
    let s = &file.budgets["s"];
    assert_eq!((s.tool_calls, s.minutes), (55, 18));
}

#[test]
fn apply_asks_and_applies_only_on_yes() {
    let h = RunHarness::new("");
    seed(&h, &[]);
    let question =
        "apply thresholds.s: S line threshold 20 → 35 (p90 of 34 merged S tasks)? [y/N] ";
    let no = stats(&h, &["--apply", "thresholds.s"], "n\n");
    assert!(stderr(&no).contains(question), "{}", stderr(&no));
    assert!(!no.status.success(), "a no applies nothing");
    assert!(!stdout(&no).contains("applied"), "{}", stdout(&no));
    assert_eq!(tuning(&h).thresholds, None);

    let yes = stats(&h, &["--apply", "thresholds.s"], "y\n");
    ok(&yes);
    assert!(stderr(&yes).contains(question), "{}", stderr(&yes));
    let text = stdout(&yes);
    let applied = format!(
        "applied thresholds.s: new runs in {} use it\n",
        project(&h).display()
    );
    assert!(text.starts_with(&applied), "before the stats:\n{text}");
    assert!(!lists(&text, "thresholds.s"), "{text}");
    assert_eq!(
        tuning(&h).thresholds,
        Some(SizeThresholds {
            s_lines: 35,
            m_lines: 100
        })
    );
    let next = stats(&h, &[], "");
    ok(&next);
    assert!(!lists(&stdout(&next), "thresholds.s"), "{}", stdout(&next));
    assert!(lists(&stdout(&next), "route.s"), "{}", stdout(&next));
}

#[test]
fn apply_yes_skips_the_question() {
    let h = RunHarness::new("");
    seed(&h, &[]);
    let out = stats(&h, &["--apply", "route.s", "--yes"], "");
    ok(&out);
    assert!(!stderr(&out).contains("[y/N]"), "{}", stderr(&out));
    assert!(
        stdout(&out).starts_with(&format!(
            "applied route.s: new runs in {} use it\n",
            project(&h).display()
        )),
        "{}",
        stdout(&out)
    );
    let route = tuning(&h).routes["s"];
    assert_eq!(
        (route.strength, route.effort),
        (proto::Strength::Standard, proto::Effort::Medium)
    );
}

#[test]
fn dismiss_hides_a_proposal() {
    let h = RunHarness::new("");
    seed(&h, &[]);
    let out = stats(&h, &["--dismiss", "route.s"], "");
    ok(&out);
    assert!(!stderr(&out).contains("[y/N]"), "never asks");
    let text = stdout(&out);
    assert!(
        text.starts_with(
            "dismissed route.s: it is not proposed again while it would propose standard/medium\n"
        ),
        "{text}"
    );
    assert!(!lists(&text, "route.s"), "{text}");
    assert!(lists(&text, "thresholds.s"), "{text}");
    assert_eq!(tuning(&h).dismissed["route.s"], "standard/medium");
    let next = stats(&h, &[], "");
    assert!(!lists(&stdout(&next), "route.s"), "{}", stdout(&next));
}

#[test]
fn an_unknown_id_exits_1_with_the_message() {
    let h = RunHarness::new("");
    seed(&h, &[]);
    for args in [
        &["--apply", "thresholds.x", "--yes"][..],
        &[
            "--apply",
            "thresholds.s",
            "--apply",
            "thresholds.x",
            "--yes",
        ],
        &["--dismiss", "thresholds.x"],
        &[
            "--apply",
            "thresholds.s",
            "--dismiss",
            "thresholds.x",
            "--yes",
        ],
    ] {
        let out = stats(&h, args, "");
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains(
                "no current proposal thresholds.x; run anthrex run stats to see the proposals"
            ),
            "{args:?}: {}",
            stderr(&out)
        );
        // The whole request is refused: the known id is not applied either.
        assert_eq!(tuning(&h).thresholds, None, "{args:?}");
    }
}

#[test]
fn stats_json_carries_tuning() {
    let h = RunHarness::new("");
    seed(&h, &[]);
    let out = stats(&h, &["--json"], "");
    ok(&out);
    let stats: HistoryStats = serde_json::from_str(&stdout(&out)).expect("HistoryStats");
    let tuning = stats.tuning.expect("the tuning report");
    assert_eq!(tuning.path, repo_dir(&h).join(TUNING_FILE));
    let s = &tuning.classes[0];
    assert_eq!((s.class.as_str(), s.samples), ("S", 34));
    assert!(matches!(s.refit, RefitState::Written { .. }), "{s:?}");
    assert_eq!(s.refit_budget, Some(budget(55, 18)));
    let ids: Vec<&str> = tuning.proposals.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["thresholds.s", "route.s"]);
}

/// Decision 48: what the Settings screen sends on opening records no revert and writes
/// no file, yet reports the refit it would write.
#[test]
fn a_read_only_stats_writes_nothing() {
    let h = RunHarness::new("");
    // An accepted run, merged and then reverted: a plain stats would record the revert.
    let repo = &h.repo;
    git_in(repo, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("b.txt"), "b\n").unwrap();
    git_in(repo, &["add", "b.txt"]);
    git_in(repo, &["commit", "-q", "-m", "the run's work"]);
    git_in(repo, &["checkout", "-q", "main"]);
    git_in(repo, &["merge", "-q", "--no-ff", "--no-edit", "feature"]);
    let accept = git_in(repo, &["rev-parse", "HEAD"]);
    git_in(repo, &["revert", "-m", "1", "--no-edit", &accept]);
    let run = HistoryLine::Run(RunRecord {
        v: HISTORY_VERSION,
        record_id: "x1".into(),
        at: now(),
        run_id: "x1".into(),
        goal: "g".into(),
        path: None,
        triage: None,
        profile_source: None,
        outcome: "accepted".into(),
        base_branch: "main".into(),
        accepted_commit: Some(accept),
        tasks: 1,
        usage: None,
    });
    let history = seed(&h, &[run]);
    // An older refit of S, which history would now move to 55/18.
    let mut file = TuningFile::default();
    file.budgets.insert(
        "s".into(),
        ClassBudget {
            tool_calls: 40,
            minutes: 15,
            tokens: None,
            samples: 30,
            at: 1_790_000_000,
        },
    );
    save(&repo_dir(&h), &file).unwrap();
    let tuning_path = repo_dir(&h).join(TUNING_FILE);
    let before = (
        std::fs::read(&history).unwrap(),
        std::fs::read(&tuning_path).unwrap(),
    );

    let reply = h.request(RunRequest::Stats {
        dir: h.repo.clone(),
        apply: Vec::new(),
        dismiss: Vec::new(),
        read_only: true,
    });
    let RunReply::Stats { stats, .. } = reply else {
        panic!("not stats: {reply:?}");
    };
    let after = (
        std::fs::read(&history).unwrap(),
        std::fs::read(&tuning_path).unwrap(),
    );
    assert!(
        before == after,
        "history.jsonl and tuning.toml are untouched"
    );
    let s = &stats.tuning.expect("the tuning report").classes[0];
    assert_eq!(s.refit, RefitState::Written { at: 1_790_000_000 });
    assert_eq!(s.refit_budget, Some(budget(55, 18)));

    // The contrast: a plain stats records the revert and writes the refit.
    ok(&stats_plain(&h));
    assert_ne!(std::fs::read(&history).unwrap(), before.0);
    assert_eq!(tuning(&h).budgets["s"].tool_calls, 55);
}

fn stats_plain(h: &RunHarness) -> Output {
    stats(h, &[], "")
}

#[test]
fn a_read_only_stats_reports_the_orchestrator_list() {
    let h = RunHarness::with_config(
        "",
        "[orchestrator.routes.orchestrator]\ncandidates = [{ runtime = \"claude\", model = \"claude-opus-5-5\", effort = \"high\" }]\n",
        &[],
    );
    let reply = h.request(RunRequest::Stats {
        dir: h.repo.clone(),
        apply: Vec::new(),
        dismiss: Vec::new(),
        read_only: true,
    });
    let RunReply::Stats { stats, .. } = reply else {
        panic!("not stats: {reply:?}");
    };
    let tuning = stats.tuning.expect("a report with no history too");
    assert_eq!(
        tuning.orchestrator_list.as_deref(),
        Some("claude/claude-opus-5-5 high")
    );
    assert!(
        !repo_dir(&h).join(TUNING_FILE).exists(),
        "read-only writes no file"
    );
}

/// Decision 10 and ruling T8-2: a `tuning.toml` that does not parse is moved aside by a
/// plain stats, whose block says why first; a read-only one leaves it where it is.
#[test]
fn a_moved_bad_file_says_why_in_stats() {
    let h = RunHarness::new("");
    seed(&h, &[]);
    let bad = repo_dir(&h).join(TUNING_FILE);
    std::fs::write(&bad, "v = 1\nbudgets = 3\n").unwrap();
    let reply = h.request(RunRequest::Stats {
        dir: h.repo.clone(),
        apply: Vec::new(),
        dismiss: Vec::new(),
        read_only: true,
    });
    let RunReply::Stats { stats, .. } = reply else {
        panic!("not stats: {reply:?}");
    };
    let report = stats.tuning.expect("a report");
    assert_eq!(report.moved_bad_file, None);
    assert!(report.parse_error.is_some(), "{report:?}");
    assert_eq!(
        std::fs::read_to_string(&bad).unwrap(),
        "v = 1\nbudgets = 3\n"
    );

    let out = stats_plain(&h);
    ok(&out);
    let text = stdout(&out);
    let line = text
        .lines()
        .find(|l| l.starts_with("tuning: tuning.toml did not parse ("))
        .unwrap_or_else(|| panic!("the moved file's line:\n{text}"));
    let moved = std::fs::read_dir(repo_dir(&h))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().contains("tuning.toml.bad-"))
        .expect("the bad file moved aside");
    assert!(
        line.starts_with("tuning: tuning.toml did not parse (line 2: ")
            && line.ends_with(&format!(
                "); it was moved to {} and tuning starts again from history",
                moved.display()
            )),
        "{line}"
    );
}
