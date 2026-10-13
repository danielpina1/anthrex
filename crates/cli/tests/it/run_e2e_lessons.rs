//! Milestone 9.15, task M9.15.9 (decisions 9 and 10): a recovered task's distill entry,
//! end to end through a real daemon with `fake-agent` as both runtimes. The test opts
//! into lessons (ruling C3) with its own `[lessons]` table, risk notes off.

use crate::support;

use proto::{LessonLine, TaskState};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// `check.sh`: exits 0 unless a file `broken` exists.
const CHECK_SH: &str =
    "if [ -e broken ]; then echo 'error: broken is present'; exit 1; fi\necho check ok\n";

const LESSONS_ON: &str = "[lessons]\nenabled = true\ndistill = true\nrisk_note = false\n";

/// Every `queued` line of the repository's `lessons.jsonl`, as the store reads it.
fn queued_lines(h: &RunHarness) -> Vec<proto::QueuedLine> {
    let path = h.repo_dir().join(daemon::lessons::LESSONS_FILE);
    let (lines, problems) = daemon::lessons::store::read_store(&path);
    assert!(problems.is_empty(), "{problems:?}");
    (lines.into_iter())
        .filter_map(|l| match l {
            LessonLine::Queued(q) => Some(q),
            _ => None,
        })
        .collect()
}

#[test]
fn a_recovered_task_queues_one_distill_entry() {
    let mut h = RunHarness::with_repo_and_config("", &[], &[("check.sh", CHECK_SH)], LESSONS_ON);
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            commit("broken", "x\n"),
            done("added a"),
            read("The check failed"),
            sh("git rm -q broken && git commit -qm 'remove broken'"),
            done("fixed the check"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let plan = format!(
        "goal = \"Lessons\"\n\n[profile]\ncheck = \"sh check.sh\"\n{}",
        task("t1", &["a.txt", "broken"], "")
    );
    let id = h.start(&plan, true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.bounces.check, 1, "{:?}", t1.bounces);

    let lines = queued_lines(&h);
    assert_eq!(lines.len(), 1, "{lines:#?}");
    let line = &lines[0];
    assert_eq!(line.record_id, format!("{id}/t1/distill"));
    assert_eq!(line.task_id.as_deref(), Some("t1"));
    assert_eq!(line.cause, proto::DistillCause::TaskRecovered);
    assert!(line.dossier.starts_with("## outcome"), "{}", line.dossier);
    assert!(!line.fps.is_empty(), "the failed check's signature");

    // A restart reconciles the run and writes no second entry.
    h.restart_daemon(&[]);
    let restored = h.wait_run(&id, |r| r.state == run.state, RUN_WAIT);
    assert_eq!(t(&restored, "t1").state, TaskState::Merged);
    assert_eq!(queued_lines(&h).len(), 1);
    let path = h.repo_dir().join(daemon::lessons::LESSONS_FILE);
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
}
