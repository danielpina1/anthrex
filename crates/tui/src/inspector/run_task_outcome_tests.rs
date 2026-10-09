//! M9.0.7.8: the task panel, outcome first (decisions 12–15): the pipeline, the
//! criteria ticked only by an approving review, the review row, the four sections in
//! their order with nothing the 9.0.5 panel listed dropped, the footer and the names.

use super::run_task_outcome::{criteria_rows, pipeline_text, review_row};
use super::run_tests::{app_of, inspect_node};
use super::{Inspection, Section};
use crate::app::App;
use crate::app::task_detail::{DetailState, TaskDetailCache, detail_key};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{GEMINI_NOW, gemini_fixture};
use crate::tree::stage_fixtures::stage;
use proto::{
    CheckInfo, Finding, ReviewInfo, RunInfo, RunsSnapshot, Severity, SummarySource, TaskDetailInfo,
    TaskInfo, TaskOrigin, TaskState, TestMode, Verdict, WindowInfo,
};

pub(crate) const CRITERIA: [&str; 2] = [
    "Stop marks the window idle",
    "SubagentStop pairs with Start",
];

pub(crate) fn t2_key() -> NodeKey {
    NodeKey::Task {
        run: "r1".into(),
        id: "t2".into(),
    }
}

fn t2(run: &mut RunInfo) -> &mut TaskInfo {
    run.tasks.iter_mut().find(|t| t.id == "t2").expect("t2")
}

/// The in-review fixture: the Gemini run in two stages, `t2` (stage 1 of 2, M, tdd,
/// `cx gpt-6-sol`) in review round 2 after a passed proof and a passed check
/// (`cargo test -p gemini 4.1s`), its diff `+142 −18 · 3 files`.
pub(crate) fn in_review_snapshot() -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snapshot, windows) = gemini_fixture();
    let run = &mut snapshot.runs[0];
    run.stages = vec![stage(1, Some(&"1".repeat(40)), 7, 5), stage(2, None, 2, 0)];
    for task in &mut run.tasks {
        task.stage = if matches!(task.id.as_str(), "t3" | "t7") {
            2
        } else {
            1
        };
    }
    let task = t2(run);
    task.route.model = "gpt-6-sol".into();
    task.diff = Some(proto::DiffStats {
        files: 3,
        hunks: 5,
        added: 142,
        removed: 18,
    });
    task.test = None;
    task.red = None;
    if let Some(check) = task.last_check.as_mut() {
        check.summary = "cargo test -p gemini 4.1s".into();
    }
    (snapshot, windows)
}

/// `t2`'s detail, landed: a five-line brief, the two criteria and `summary`.
pub(crate) fn land_detail(app: &mut App, summary: Option<&str>) {
    let task = app.runs.runs[0]
        .tasks
        .iter()
        .find(|t| t.id == "t2")
        .expect("t2");
    app.task_detail = Some(TaskDetailCache {
        run_id: "r1".into(),
        task_id: "t2".into(),
        key: detail_key(task),
        state: DetailState::Ready(Box::new(TaskDetailInfo {
            run_id: "r1".into(),
            task_id: "t2".into(),
            brief: "Map each Gemini hook event to a status.\nSubagentStop pairs with its start.\nStop marks the agent idle.".into(),
            acceptance: CRITERIA.map(String::from).to_vec(),
            worker_summary: summary.map(str::to_owned),
            summary_source: summary.map(|_| SummarySource::TaskDone),
        })),
    });
}

pub(crate) fn in_review() -> App {
    let mut app = app_of(in_review_snapshot());
    land_detail(&mut app, None);
    app
}

fn in_review_task() -> (RunInfo, TaskInfo) {
    let (snapshot, _) = in_review_snapshot();
    let run = snapshot.runs[0].clone();
    let task = run
        .tasks
        .iter()
        .find(|t| t.id == "t2")
        .cloned()
        .expect("t2");
    (run, task)
}

/// A check-mode task the check bounced: re-queued for a worker after its done signal,
/// its check failed, no review yet.
fn check_mode_task_bounced_by_the_check() -> (RunInfo, TaskInfo) {
    let (run, mut task) = in_review_task();
    task.test_mode = TestMode::Check;
    task.state = TaskState::Queued;
    task.reviews.clear();
    let check: &mut CheckInfo = task.last_check.as_mut().expect("a check");
    check.ok = false;
    (run, task)
}

fn review(verdict: Verdict, blocking: bool) -> ReviewInfo {
    let (_, task) = in_review_task();
    ReviewInfo {
        round: 2,
        route: task.review_route.clone().expect("a review route"),
        verdict: Some(verdict),
        summary: "\n  looks right  \nsecond line".into(),
        findings: Vec::new(),
        blocking,
        lane: None,
    }
}

fn approving() -> ReviewInfo {
    review(Verdict::Approve, false)
}

fn changes() -> ReviewInfo {
    review(Verdict::Changes, true)
}

#[test]
fn the_pipeline_marks_each_step() {
    // tdd, verified, a review route, in review after a passed proof and check.
    let (run, task) = in_review_task();
    assert_eq!(
        pipeline_text(&run, &task, false),
        "done ✓ › proof ✓ › check ✓ › review › merge ◌"
    );
    let (run, task) = check_mode_task_bounced_by_the_check();
    assert_eq!(
        pipeline_text(&run, &task, false),
        "done ✓ › check ✗ › review ◌ › merge ◌"
    );
    assert_eq!(
        pipeline_text(&run, &task, true),
        "done + > check x > review . > merge ."
    );
    // A step that does not apply is left out, never drawn `–`.
    let (mut run, mut task) = in_review_task();
    run.unverified = true;
    task.review_route = None;
    task.state = TaskState::MergeQueue;
    assert_eq!(
        pipeline_text(&run, &task, false),
        "done ✓ › proof ✓ › merge"
    );
    task.state = TaskState::Merged;
    assert_eq!(
        pipeline_text(&run, &task, false),
        "done ✓ › proof ✓ › merge ✓"
    );
    // The worker at work: `done` is the current step, the rest not reached.
    let (run, mut task) = in_review_task();
    task.state = TaskState::Working;
    task.done_signal = None;
    task.last_proof = None;
    task.last_check = None;
    task.reviews.clear();
    assert_eq!(
        pipeline_text(&run, &task, false),
        "done › proof ◌ › check ◌ › review ◌ › merge ◌"
    );
    // Fix round 1 (m1): a reported task, or a research or review task at any state,
    // is never merged: no merge step.
    let (run, mut task) = in_review_task();
    task.state = TaskState::Reported;
    assert_eq!(
        pipeline_text(&run, &task, false),
        "done ✓ › proof ✓ › check ✓ › review ✗"
    );
    task.state = TaskState::Working;
    task.kind = proto::TaskKind::Research;
    assert!(!pipeline_text(&run, &task, false).contains("merge"));
    // Final fix wave (task 8's minor): a review task is never merged either.
    task.kind = proto::TaskKind::Review;
    assert!(!pipeline_text(&run, &task, false).contains("merge"));
    task.kind = proto::TaskKind::Code;
    assert!(pipeline_text(&run, &task, false).contains("merge"));
}

/// Review focus 4: nothing ticks a criterion but an approving review.
#[test]
fn criteria_tick_only_on_an_approving_review() {
    let criteria = CRITERIA;
    assert_eq!(
        criteria_rows(&criteria, Some(&approving()), None),
        vec![
            "✓ Stop marks the window idle",
            "✓ SubagentStop pairs with Start"
        ]
    );
    assert_eq!(
        criteria_rows(&criteria, Some(&changes()), None),
        vec![
            "◌ Stop marks the window idle",
            "◌ SubagentStop pairs with Start"
        ]
    );
    assert_eq!(
        criteria_rows(&criteria, None, None)[0],
        "◌ Stop marks the window idle"
    );
    // Merged by override: the approval is not what merged it.
    assert_eq!(
        criteria_rows(&criteria, Some(&approving()), Some("trusted"))[0],
        "◌ Stop marks the window idle"
    );
    // An approval that still blocks, or a review still running, proves nothing.
    assert_eq!(
        criteria_rows(&criteria, Some(&review(Verdict::Approve, true)), None)[0],
        "◌ Stop marks the window idle"
    );
    let mut running = approving();
    running.verdict = None;
    assert_eq!(
        criteria_rows(&criteria, Some(&running), None)[0],
        "◌ Stop marks the window idle"
    );
    assert!(
        criteria_rows(&criteria, Some(&changes()), None)
            .iter()
            .all(|r| !r.contains('✗'))
    );
    // The panel asks the task's latest review: round 2 is still running, so nothing is
    // ticked though round 1 is a verdict.
    let app = in_review();
    assert_eq!(
        value(&inspect_node(&app, &t2_key()), "OUTCOME", "accept"),
        "◌ Stop marks the window idle\n◌ SubagentStop pairs with Start"
    );
    // Until the detail lands the row reads `loading…`; no criteria, no row.
    let app = app_of(in_review_snapshot());
    assert_eq!(
        value(&inspect_node(&app, &t2_key()), "OUTCOME", "accept"),
        "loading…"
    );
    let mut app = in_review();
    if let Some(DetailState::Ready(detail)) = app.task_detail.as_mut().map(|c| &mut c.state) {
        detail.acceptance.clear();
    }
    assert!(field(&inspect_node(&app, &t2_key()), "OUTCOME", "accept").is_none());
}

#[test]
fn review_rows_read_decision_12() {
    let (_, mut task) = in_review_task();
    assert_eq!(review_row(&task), "in review · r2");
    task.state = TaskState::MergeQueue;
    task.reviews[1] = approving();
    assert_eq!(review_row(&task), "r2 ✓ approve · looks right");
    task.reviews[1].summary = String::new();
    assert_eq!(review_row(&task), "r2 ✓ approve");
    // Round 1's changes, with its worst finding.
    task.reviews.truncate(1);
    task.state = TaskState::Working;
    // Fix round 1 (m2): the summary line the 9.0.5 `verdict` showed, on the next row.
    assert_eq!(
        review_row(&task),
        "r1 ✗ changes · 1 critical, 2 minor: status.rs:118 \"SubagentStop not paired\"\none pairing bug"
    );
    // Changes without a finding read the summary's first line.
    task.reviews[0].findings.clear();
    assert_eq!(review_row(&task), "r1 ✗ changes · one pairing bug");
    // A non-blocking review with findings is an approval with them.
    task.reviews[0] = approving();
    task.reviews[0].findings = vec![Finding {
        severity: Severity::Minor,
        file: Some("src/a.rs".into()),
        line: Some(3),
        input: None,
        text: "naming".into(),
    }];
    assert_eq!(review_row(&task), "r2 ✓ approve · looks right");
    // A review route and nothing judged yet; no review route at all.
    task.reviews.clear();
    assert_eq!(review_row(&task), "not yet");
    task.review_route = None;
    assert_eq!(review_row(&task), "none");
}

fn titles(sections: &[Section]) -> Vec<&'static str> {
    sections.iter().map(|s| s.title).collect()
}

fn labels(sections: &[Section], title: &str) -> Vec<&'static str> {
    sections
        .iter()
        .find(|s| s.title == title)
        .unwrap_or_else(|| panic!("no section {title}"))
        .fields
        .iter()
        .map(|f| f.label)
        .collect()
}

fn field<'a>(inspection: &'a Inspection, title: &str, label: &str) -> Option<&'a str> {
    inspection
        .sections
        .iter()
        .find(|s| s.title == title)
        .unwrap_or_else(|| panic!("no section {title}"))
        .fields
        .iter()
        .find(|f| f.label == label)
        .map(|f| f.value.as_str())
}

fn value<'a>(inspection: &'a Inspection, title: &str, label: &str) -> &'a str {
    field(inspection, title, label).unwrap_or_else(|| panic!("no {title} {label}"))
}

/// Every label the 9.0.5 panel drew (`inspector/run_task_sections.rs` at 92cdefb):
/// GOAL, STATUS and RESULT, in their order. A literal copy, never derived.
fn old_labels() -> [&'static str; 25] {
    [
        // GOAL
        "brief",
        "owns",
        "done when",
        // STATUS
        "result",
        "stage",
        "worker",
        "now",
        "check",
        "review",
        "stages",
        "route",
        "stage no.",
        "origin",
        "tier",
        "deps",
        "budget",
        "tries",
        "messages",
        "notes",
        "history",
        // RESULT
        "summary",
        "verdict",
        "diff",
        "merged",
        // RESULT's placeholder, `nothing yet`.
        "",
    ]
}

/// Decision 15's renames, and where the rest went: `verdict` is the review row's
/// verdict, `done when` the `accept` row, and RESULT's `nothing yet` the pipeline,
/// whose `◌` marks say what it said.
fn renamed(label: &'static str) -> &'static str {
    match label {
        "stage" => "phase",
        "stage no." => "stage",
        "stages" => "pipeline",
        "done when" => "accept",
        "verdict" => "review",
        "" => "pipeline",
        other => other,
    }
}

/// Every field the old panel could show, at once: `t2` merged with its summary, a
/// live activity, owns, messages, notes, an engine origin and a tier.
fn everything() -> App {
    let (mut snapshot, windows) = in_review_snapshot();
    let task = t2(&mut snapshot.runs[0]);
    task.state = TaskState::Merged;
    task.merge_commit = Some("0123456789".into());
    task.activity = Some("Edit src/lib.rs".into());
    task.owns = vec!["crates/daemon/src/hooks.rs".into()];
    task.message_count = 1;
    task.last_message_kind = Some(proto::MessageKind::Info);
    task.last_message_line = Some("use the new hook".into());
    task.task_notes = vec![proto::TaskNoteInfo {
        task_id: "t6".into(),
        kind: proto::TaskNoteKind::Risk,
        text: "watch the pairing".into(),
        at: GEMINI_NOW - 60,
    }];
    task.origin = TaskOrigin::Bisect;
    task.tier = Some(proto::TierInfo {
        tier: 1,
        affected: "3 modules".into(),
        steps: 3,
        cached: 0,
        ok: true,
        secs: 40,
        flaky: Vec::new(),
    });
    let mut app = app_of((snapshot, windows));
    land_detail(&mut app, Some("Mapped all nine hook events."));
    app
}

#[test]
fn sections_are_outcome_evidence_intent_detail() {
    let inspection = inspect_node(&in_review(), &t2_key());
    let sections = &inspection.sections;
    assert_eq!(
        titles(sections),
        ["OUTCOME", "EVIDENCE", "INTENT", "DETAIL"]
    );
    assert_eq!(labels(sections, "OUTCOME"), ["pipeline", "check", "accept"]);
    assert_eq!(labels(sections, "EVIDENCE"), ["diff", "review"]);
    assert_eq!(labels(sections, "INTENT"), ["brief"]);
    assert_eq!(
        labels(sections, "DETAIL"),
        [
            "phase", "worker", "deps", "budget", "tries", "stage", "route", "history"
        ]
    );
    // Nothing the 9.0.5 panel listed is missing (M9.0.5 decision 22's rule).
    let inspection = inspect_node(&everything(), &t2_key());
    let shown: Vec<&str> = inspection
        .sections
        .iter()
        .flat_map(|s| &s.fields)
        .map(|f| f.label)
        .collect();
    for old in old_labels() {
        assert!(
            shown.contains(&renamed(old)),
            "{old:?} (now {:?}) is not shown: {shown:?}",
            renamed(old)
        );
    }
    assert_eq!(
        labels(&inspection.sections, "OUTCOME"),
        ["pipeline", "check", "accept", "result"]
    );
    assert_eq!(
        labels(&inspection.sections, "EVIDENCE"),
        ["diff", "review", "summary", "merged"]
    );
    assert_eq!(labels(&inspection.sections, "INTENT"), ["brief", "owns"]);
    assert_eq!(
        labels(&inspection.sections, "DETAIL"),
        [
            "phase", "worker", "now", "deps", "budget", "tries", "stage", "origin", "tier",
            "route", "messages", "notes", "history"
        ]
    );
    assert_eq!(
        value(&inspection, "OUTCOME", "result"),
        "Mapped all nine hook events."
    );
    // The old title's size and test mode moved to the footer.
    assert!(
        inspection
            .footer
            .as_deref()
            .unwrap_or("")
            .ends_with("M · tdd")
    );
}

/// A blocked task's block text, every line of it, in OUTCOME.
#[test]
fn a_blocked_task_says_why_in_outcome() {
    let (mut snapshot, windows) = in_review_snapshot();
    let task = t2(&mut snapshot.runs[0]);
    task.state = TaskState::Blocked;
    task.block = Some(proto::BlockInfo::new(
        proto::BlockReason::Question,
        "Which event pairs?\nStart or Stop",
    ));
    let inspection = inspect_node(&app_of((snapshot, windows)), &t2_key());
    assert_eq!(
        value(&inspection, "OUTCOME", "blocked"),
        "question: Which event pairs?\nStart or Stop"
    );
    assert_eq!(value(&inspection, "DETAIL", "phase"), "blocked: question");
    assert_eq!(inspection.right.as_deref(), Some("blocked: question"));
}

#[test]
fn the_footer_names_stage_route_size_and_mode() {
    let inspection = inspect_node(&in_review(), &t2_key());
    assert_eq!(
        inspection.footer.as_deref(),
        Some("stage 1 of 2 · cx gpt-6-sol · M · tdd")
    );
    assert_eq!(inspection.right.as_deref(), Some("in review · r2"));
    // A single-stage run, a Claude route with no model, a small untested task.
    let (mut snapshot, windows) = gemini_fixture();
    let t8 = snapshot.runs[0]
        .tasks
        .iter_mut()
        .find(|t| t.id == "t8")
        .expect("t8");
    t8.test_mode = TestMode::None;
    let key = NodeKey::Task {
        run: "r1".into(),
        id: "t8".into(),
    };
    let inspection = inspect_node(&app_of((snapshot, windows)), &key);
    assert_eq!(inspection.footer.as_deref(), Some("cl default · S · none"));
    assert_eq!(inspection.right.as_deref(), Some("merged"));
}

#[test]
fn names_follow_decision_15() {
    let inspection = inspect_node(&in_review(), &t2_key());
    let labels: Vec<&str> = inspection
        .sections
        .iter()
        .flat_map(|s| &s.fields)
        .map(|f| f.label)
        .chain(inspection.fields.iter().map(|f| f.label))
        .collect();
    for name in ["phase", "stage", "pipeline"] {
        assert!(labels.contains(&name), "{name}: {labels:?}");
    }
    for old in ["stage no.", "stages", "done when", "verdict"] {
        assert!(!labels.contains(&old), "{old}: {labels:?}");
    }
    assert_eq!(value(&inspection, "DETAIL", "stage"), "1 of 2");
    assert_eq!(value(&inspection, "DETAIL", "phase"), "in review");
    assert_eq!(
        value(&inspection, "DETAIL", "deps"),
        "after t0 ✓, t6 ✓ · unblocks t3, t7 · on critical path"
    );
}

/// Ruling D-2: a merged or reported task's outcome is in OUTCOME, so it shows without
/// scrolling; the sections keep decision 12's order. (Moved from
/// `run_task_sections_tests.rs`, at its bound, with its 9.0.7 expectations.)
#[test]
fn a_finished_task_leads_with_its_result() {
    let finished = |state| {
        let (mut snapshot, windows) = gemini_fixture();
        let task = t2(&mut snapshot.runs[0]);
        task.state = state;
        task.merge_commit = Some("0123456789".into());
        app_of((snapshot, windows))
    };
    for state in [TaskState::Merged, TaskState::Reported] {
        let mut app = finished(state);
        land_detail(&mut app, Some("\n  Mapped the hooks.  \nAdded a test."));
        let inspection = inspect_node(&app, &t2_key());
        assert_eq!(
            titles(&inspection.sections),
            ["OUTCOME", "EVIDENCE", "INTENT", "DETAIL"]
        );
        let last = inspection.sections[0].fields.last().unwrap();
        assert_eq!(
            (last.label, last.value.as_str()),
            ("result", "Mapped the hooks."),
            "{state:?}"
        );
        // Before the detail lands: the verdict's review row stands in.
        let app = finished(state);
        assert_eq!(
            value(&inspect_node(&app, &t2_key()), "OUTCOME", "result"),
            "r1 ✗ changes · 1 critical, 2 minor: status.rs:118 \"SubagentStop not paired\""
        );
    }
    // A task still in review has no result line.
    let app = app_of(gemini_fixture());
    assert!(field(&inspect_node(&app, &t2_key()), "OUTCOME", "result").is_none());
}

/// Fix round 1 (m3): every M8c flat field but the three OUTCOME and EVIDENCE draw is a
/// DETAIL row with the same value, so a field missing from `DETAIL_ROWS` fails here.
#[test]
fn every_flat_field_is_a_detail_row() {
    let inspection = inspect_node(&everything(), &t2_key());
    let detail = &inspection.sections[3];
    assert_eq!(detail.title, "DETAIL");
    for flat in &inspection.fields {
        if matches!(flat.label, "pipeline" | "diff" | "review") {
            continue;
        }
        let row = detail.fields.iter().find(|f| f.label == flat.label);
        assert_eq!(
            row.map(|f| f.value.as_str()),
            Some(flat.value.as_str()),
            "{}",
            flat.label
        );
    }
}
