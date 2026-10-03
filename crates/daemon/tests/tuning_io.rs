//! Milestone 9.5 task 9: `tuning.toml` on disk (decision 10) and a start's tuning
//! (decision 12) against real temporary data directories: saving and loading, an absent
//! and an unparseable file, two tunings of one repository at once, and a run with
//! history off. No daemon, no agent, no git.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use daemon::run::driver::tuning::{TuningLocks, tune_for_start};
use daemon::run::refit_render::render;
use daemon::run::tuning_io::{Loaded, TUNING_FILE, load, save, thresholds};
use proto::{
    ClassBudget, ClassRoute, Effort, PathWeights, SizeThresholds, Strength, TuningFile,
    TuningReport,
};

/// M9.5.8's injected clock (review ruling I5).
const NOW: u64 = 1_790_500_000;

/// The recorded history `refit.jsonl` (task M9.5.8): S qualifies with 34 samples.
fn refit_history() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/history/refit.jsonl")
}

/// A repository data directory under a fresh temp dir, with `refit.jsonl` as its
/// `history.jsonl` when `history` is set.
fn repo_dir(history: bool) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repos").join("p-1234");
    std::fs::create_dir_all(&dir).unwrap();
    if history {
        std::fs::copy(refit_history(), dir.join("history.jsonl")).unwrap();
    }
    (tmp, dir)
}

fn every_section() -> TuningFile {
    let mut file = TuningFile::default();
    file.budgets.insert(
        "s".into(),
        ClassBudget {
            tool_calls: 55,
            minutes: 18,
            tokens: Some(2_500_000),
            samples: 34,
            at: NOW,
        },
    );
    file.budgets.insert(
        "hub".into(),
        ClassBudget {
            tool_calls: 160,
            minutes: 70,
            tokens: None,
            samples: 31,
            at: NOW,
        },
    );
    file.weights = Some(PathWeights {
        s_secs: 550,
        m_secs: 1650,
        hub_secs: 1650,
        derived: vec!["M".into(), "hub".into()],
        at: NOW,
    });
    file.thresholds = Some(SizeThresholds {
        s_lines: 35,
        m_lines: 140,
    });
    file.routes.insert(
        "s".into(),
        ClassRoute {
            strength: Strength::Standard,
            effort: Effort::Medium,
        },
    );
    file.dismissed
        .insert("route.m".into(), "frontier/medium".into());
    file
}

#[test]
fn save_then_load_round_trips() {
    let (_tmp, dir) = repo_dir(false);
    let file = every_section();
    save(&dir, &file).unwrap();
    assert_eq!(load(&dir, NOW).unwrap(), Loaded::File(file.clone()));
    // Written atomically and private, with no temp file left.
    let path = dir.join(TUNING_FILE);
    let mode = std::os::unix::fs::PermissionsExt::mode(&path.metadata().unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![TUNING_FILE.to_string()]);
    // Triage's read takes the thresholds without moving anything.
    assert_eq!(thresholds(&dir), file.thresholds.unwrap());
    // The defaults are written as almost nothing, and read back the same.
    save(&dir, &TuningFile::default()).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "v = 1");
    assert_eq!(
        load(&dir, NOW).unwrap(),
        Loaded::File(TuningFile::default())
    );
}

#[tokio::test]
async fn absent_is_default() {
    let (_tmp, dir) = repo_dir(false);
    assert_eq!(load(&dir, NOW).unwrap(), Loaded::Absent);
    assert_eq!(thresholds(&dir), SizeThresholds::default());
    let cfg = config::Orchestrator::default();
    let tuned = tune_for_start(&cfg, &TuningLocks::default(), &dir, NOW).await;
    assert_eq!(tuned.budget_s, None);
    assert_eq!(tuned.weights, None);
    assert_eq!(tuned.thresholds, SizeThresholds::default());
    assert_eq!(
        tuned.log,
        vec!["tuning: none (history has fewer than 30 samples per class)".to_string()]
    );
    assert!(
        !dir.join(TUNING_FILE).exists(),
        "nothing learned, nothing written"
    );
}

#[tokio::test]
async fn an_unparseable_file_is_moved_aside_and_tuning_restarts() {
    let (_tmp, dir) = repo_dir(true);
    std::fs::write(
        dir.join(TUNING_FILE),
        "v = 1\n[budgets.s]\ntool_calls = \"many\"\n",
    )
    .unwrap();
    // Triage's read never moves it.
    assert_eq!(thresholds(&dir), SizeThresholds::default());
    assert!(dir.join(TUNING_FILE).exists());

    let cfg = config::Orchestrator::default();
    let tuned = tune_for_start(&cfg, &TuningLocks::default(), &dir, NOW).await;
    let moved = dir.join(format!("{TUNING_FILE}.bad-{NOW}"));
    assert!(moved.exists(), "the bad file is kept beside, renamed");
    let first = &tuned.log[0];
    let prefix = "tuning: tuning.toml did not parse (";
    let suffix = format!(
        "); it was moved to {} and tuning starts again from history",
        moved.display()
    );
    assert!(
        first.starts_with(prefix) && first.ends_with(&suffix),
        "{first}"
    );
    // The parse error, on one line, names where it is.
    assert!(
        first.contains("(line 3: ") && !first.contains('\n'),
        "{first}"
    );
    // Tuning started again from history: the S refit is written and used.
    assert_eq!(
        tuned.log[1],
        "tuning: budget S 40 calls 15m → 55 calls 18m from 34 samples"
    );
    let Loaded::File(file) = load(&dir, NOW).unwrap() else {
        panic!("a fresh tuning.toml");
    };
    assert_eq!(file.budgets["s"].tool_calls, 55);
    assert_eq!(
        tuned.budget_s.map(|b| (b.tool_calls, b.minutes)),
        Some((55, 18))
    );

    // A second bad file in the same second does not overwrite the first.
    std::fs::write(dir.join(TUNING_FILE), "not toml at all [").unwrap();
    let Loaded::MovedBad { moved_to, error } = load(&dir, NOW).unwrap() else {
        panic!("moved");
    };
    assert_eq!(moved_to, dir.join(format!("{TUNING_FILE}.bad-{NOW}-1")));
    assert!(moved.exists() && !error.is_empty());

    // A file of another version is moved aside too.
    std::fs::write(dir.join(TUNING_FILE), "v = 2\n").unwrap();
    let Loaded::MovedBad { error, .. } = load(&dir, NOW + 1).unwrap() else {
        panic!("moved");
    };
    assert_eq!(error, "version 2 is not 1, the one this anthrex reads");

    // `run stats`' block opens with the same line (ruling T8-2).
    let report = TuningReport {
        path: dir.join(TUNING_FILE),
        min_samples: 30,
        refit_budgets: true,
        classes: Vec::new(),
        proposals: Vec::new(),
        moved_bad_file: Some(moved.clone()),
        applied: Vec::new(),
        dismissed: Vec::new(),
        orchestrator_list: None,
        parse_error: Some("expected an integer".into()),
    };
    let text = render(&report);
    let line = format!(
        "tuning: tuning.toml did not parse (expected an integer); it was moved to {} and tuning starts again from history\n",
        moved.display()
    );
    assert!(text.starts_with(&line), "{text}");
}

/// Two starts in one repository at once, one refitting the budgets and the other the
/// critical-path weights: the second reads what the first wrote, so the file keeps
/// both. Without the lock, each would load the same empty file and the later write
/// would drop the other's section. Repeated, so an interleaving that would lose one
/// has many chances to happen.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_tunings_of_one_repo_serialise() {
    let mut budgets = config::Orchestrator::default();
    budgets.tuning.table.path_weights = false;
    let mut weights = config::Orchestrator::default();
    weights.tuning.table.refit_budgets = false;
    let (budgets, weights) = (Arc::new(budgets), Arc::new(weights));
    let locks = Arc::new(TuningLocks::default());
    for _ in 0..20 {
        let (_tmp, dir) = repo_dir(true);
        let start = |cfg: Arc<config::Orchestrator>| {
            let (locks, dir) = (locks.clone(), dir.clone());
            tokio::spawn(async move { tune_for_start(&cfg, &locks, &dir, NOW).await })
        };
        let (a, b) = (start(budgets.clone()), start(weights.clone()));
        let (a, b) = (a.await.unwrap(), b.await.unwrap());
        assert!(a.budget_s.is_some() && b.weights.is_some());
        let Loaded::File(file) = load(&dir, NOW).unwrap() else {
            panic!("a tuning.toml");
        };
        assert_eq!(file.budgets["s"].tool_calls, 55, "{file:?}");
        assert_eq!(
            file.weights.as_ref().map(|w| w.s_secs),
            Some(550),
            "{file:?}"
        );
    }
}

#[tokio::test]
async fn a_run_with_history_off_touches_no_file() {
    // A run with history off has no repository data directory (`history::enabled`).
    let cwd = std::env::current_dir().unwrap();
    assert!(!cwd.join(TUNING_FILE).exists());
    let cfg = config::Orchestrator::default();
    let tuned = tune_for_start(&cfg, &TuningLocks::default(), Path::new(""), NOW).await;
    assert!(tuned.log.is_empty(), "{:?}", tuned.log);
    assert_eq!(tuned.budget_s, None);
    assert_eq!(tuned.weights, None);
    assert!(
        !cwd.join(TUNING_FILE).exists(),
        "nothing written beside the process"
    );
}
