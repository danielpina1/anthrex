//! Task M9.15.10 (decisions 12 to 16): a recovered task's lesson, end to end. A real
//! daemon with `fake-agent` as both runtimes and as the decider queues the task's
//! distill entry, its worker asks the `distill` decider, and `anthrex lessons list`
//! shows the episodic lesson on trial. The test opts into lessons (ruling C3), risk
//! notes off.

use crate::support;

use std::time::{Duration, Instant};

use proto::{LessonInfo, LessonLayer, LessonStatus, TaskState};
use serde_json::json;
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_plans::*;

/// `check.sh`: exits 0 unless a file `broken` exists.
const CHECK_SH: &str =
    "if [ -e broken ]; then echo 'error: broken is present'; exit 1; fi\necho check ok\n";

const LESSONS_ON: &str = "[lessons]\nenabled = true\ndistill = true\nrisk_note = false\n";

const TITLE: &str = "Remove the broken marker before claiming done";

/// The worker's call, once the run is complete: the routing probe
/// (`INSTALLED_PROBE_TIMEOUT`, 5 s), the harness's `deciders.timeout_secs` (5 s), the
/// wait for the decider's exit and its kill grace (`EXIT_WAIT` and `KILL_GRACE`, 2 s
/// each) and a spawn on a loaded machine (5 s): 19 s; then one `lessons list`
/// request's own bound (`REQUEST_WAIT`). The polls between are 200 ms.
fn distill_wait() -> Duration {
    daemon::run::driver::INSTALLED_PROBE_TIMEOUT
        + Duration::from_secs(5)
        + daemon::decider::call::EXIT_WAIT
        + daemon::decider::call::KILL_GRACE
        + Duration::from_secs(5)
        + REQUEST_WAIT
}

fn list(h: &RunHarness, args: &[&str]) -> String {
    let repo = h.repo.display().to_string();
    let mut all = vec!["--dir", repo.as_str(), "lessons", "list"];
    all.extend_from_slice(args);
    let output = h.anthrex(&all);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn recovered_task_gives_an_on_trial_lesson_listed_by_the_cli() {
    let h = RunHarness::adapt_with_config("claude", "", &[], &[("check.sh", CHECK_SH)], LESSONS_ON);
    h.decider(
        "distill",
        1,
        json!({"answer": {"items": [{
            "layer": "episodic",
            "title": TITLE,
            "body": "The check fails while the file broken exists.",
            "files": ["broken"],
        }]}}),
    );
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
    assert_eq!(t(&run, "t1").state, TaskState::Merged);

    // The worker runs in the background: wait for the lesson with a deadline loop.
    let within = distill_wait();
    let deadline = Instant::now() + within;
    let lessons = loop {
        let lessons: Vec<LessonInfo> = serde_json::from_str(&list(&h, &["--json"])).unwrap();
        if !lessons.is_empty() {
            break lessons;
        }
        assert!(
            Instant::now() < deadline,
            "no lesson within {within:?}\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(lessons.len(), 1, "{lessons:#?}");
    let lesson = &lessons[0];
    assert_eq!(lesson.title, TITLE);
    assert_eq!(lesson.layer, LessonLayer::Episodic);
    assert_eq!(lesson.status, LessonStatus::OnTrial);
    assert_eq!(lesson.runs, vec![id.clone()]);
    assert_eq!(lesson.files, vec!["broken".to_string()]);

    let text = list(&h, &[]);
    let row = (text.lines())
        .find(|l| l.contains(TITLE))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(row.contains("on_trial"), "{text}");
    assert!(row.contains("episodic"), "{text}");

    // One distill call, whose prompt carries the task's dossier.
    let text = std::fs::read_to_string(h.decider_dir().join("calls.jsonl")).unwrap();
    let calls: Vec<serde_json::Value> = (text.lines())
        .map(|l| serde_json::from_str(l).unwrap())
        .filter(|c: &serde_json::Value| c["kind"] == "distill")
        .collect();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("--- DOSSIER ---\n## outcome\ncause: task_recovered"),
        "{prompt}"
    );
    assert!(prompt.contains("error: broken is present"), "{prompt}");
}
