//! Task M9.5.11's carries (a) and (b): `run stats`' tuning once its write has begun,
//! forced deterministically with [`Gates`]: the request's bound passes only after the
//! write is committed, and the write is held as long as the test says. Real temporary
//! data directories, the recorded `refit.jsonl`; no daemon, no agent, no git.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use proto::ProposalValue;

use super::*;

const NOW: u64 = 1_790_500_000;

/// A repository data directory with `refit.jsonl` as its history.
fn repo_dir() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repos").join("p-1");
    std::fs::create_dir_all(&dir).unwrap();
    let history = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/history/refit.jsonl");
    std::fs::copy(history, dir.join(HISTORY_FILE)).unwrap();
    (tmp, dir)
}

/// Gates that tell `committed` once the write is committed and hold it until `release`
/// is sent; the request, past its bound, waits for the commit before it looks.
fn gates(committed: mpsc::Sender<()>, release: mpsc::Receiver<()>) -> Gates {
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel::<()>();
    Gates {
        committed: Some(Box::new(move || {
            let _ = seen_tx.send(());
            let _ = committed.send(());
            let _ = release.recv_timeout(Duration::from_secs(30));
        })),
        overdue: Some(Box::pin(async move {
            let _ = seen_rx.await;
        })),
    }
}

fn applied(dir: &Path) -> bool {
    matches!(tuning_io::load(dir, NOW).unwrap(), Loaded::File(f) if f.thresholds.is_some())
}

/// Carry (a): past its bound, a request whose write had begun answers with the write's
/// own result once it lands within the write's bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_begun_before_the_bound_is_answered_with_its_result() {
    let (_tmp, dir) = repo_dir();
    let (locks, cfg) = (TuningLocks::default(), config::Orchestrator::default());
    let apply = [ProposalValue {
        id: "thresholds.s".into(),
        value: "35".into(),
    }];
    let ask = StatsAsk {
        apply: &apply,
        dismiss: &[],
        read_only: false,
    };
    let (committed_tx, committed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let at = (dir.as_path(), Path::new("/r/demo"));
    let bounds = (Duration::ZERO, Duration::from_secs(30));
    let request = stats_gated(
        &cfg,
        &locks,
        at,
        ask,
        NOW,
        bounds,
        gates(committed_tx, release_rx),
    );
    let release = async {
        let committed =
            tokio::task::spawn_blocking(move || committed_rx.recv_timeout(Duration::from_secs(30)));
        committed.await.unwrap().expect("the write is committed");
        release_tx.send(()).unwrap();
    };
    let (answer, ()) = tokio::join!(request, release);
    let report = answer.expect("the write's own result");
    assert_eq!(report.applied, ["thresholds.s"]);
    assert!(applied(&dir));
}

/// Carry (b): a begun write held past the write's bound answers
/// [`TUNING_WRITE_FINISHING`], and the write still lands under the tuning lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_begun_write_past_its_bound_says_it_is_still_finishing() {
    assert_eq!(
        TUNING_WRITE_FINISHING,
        "the tuning write is still finishing; run anthrex run stats again"
    );
    let (_tmp, dir) = repo_dir();
    let (locks, cfg) = (TuningLocks::default(), config::Orchestrator::default());
    let apply = [ProposalValue {
        id: "thresholds.s".into(),
        value: "35".into(),
    }];
    let ask = StatsAsk {
        apply: &apply,
        dismiss: &[],
        read_only: false,
    };
    let (committed_tx, _committed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let at = (dir.as_path(), Path::new("/r/demo"));
    let bounds = (Duration::ZERO, Duration::from_millis(50));
    let gated = gates(committed_tx, release_rx);
    let answer = stats_gated(&cfg, &locks, at, ask, NOW, bounds, gated).await;
    assert_eq!(answer, Err(TUNING_WRITE_FINISHING.to_string()));
    assert!(!applied(&dir), "held: not written yet");
    release_tx.send(()).unwrap();
    // The work holds the lock until its write ends.
    let lock = tokio::time::timeout(Duration::from_secs(30), locks.lock(&dir));
    drop(lock.await.expect("the work let go of the lock"));
    assert!(applied(&dir), "the write landed");
}

/// Milestone 9.6 ruling T13-5 (m5): a start that refits the design classes writes
/// them to `tuning.toml`, but their write and start lines are kept apart for a design
/// run (`Tuned.design_lines`); the start's own lines are those of a history without
/// them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_design_refit_at_a_start_keeps_its_lines_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let (plain, design) = (tmp.path().join("plain"), tmp.path().join("design"));
    for dir in [&plain, &design] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let agent = proto::PhaseAgent {
        role: proto::AgentRole::Brainstormer,
        route: proto::Route {
            runtime: proto::Runtime::Codex,
            model: "gpt-6".into(),
            strength: proto::Strength::Frontier,
            effort: proto::Effort::High,
        },
        calls: 30,
        tokens: 0,
        outcome: "ok".into(),
        secs: 900,
        sessions: 1,
    };
    for run in 0..32u32 {
        let line = proto::HistoryLine::Phase(proto::PhaseRecord {
            v: proto::HISTORY_VERSION,
            record_id: format!("r{run}/phase/1/brainstorming/v1"),
            at: u64::from(run),
            run_id: format!("r{run}"),
            round: 1,
            phase: "brainstorming".into(),
            secs: 60,
            agents: vec![agent.clone()],
            gate_versions: 1,
            disputed: 0,
        });
        crate::run::history_io::append_line(&design.join(HISTORY_FILE), &line).unwrap();
    }
    let (locks, cfg) = (TuningLocks::default(), config::Orchestrator::default());
    let without = tune_for_start(&cfg, &locks, &plain, NOW).await;
    let with = tune_for_start(&cfg, &locks, &design, NOW).await;
    assert_eq!(with.log, without.log);
    assert_eq!(
        with.design_lines,
        [
            "tuning: budget brainstorm 40 calls 15m → 75 calls 38m from 32 samples",
            "tuning: budget brainstorm 75 calls 38m from 32 samples",
        ]
    );
    assert!(matches!(
        tuning_io::load(&design, NOW).unwrap(),
        Loaded::File(f) if f.budgets.contains_key("brainstorm")
    ));
}
