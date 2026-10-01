//! M9.0.5.10: the task inspection's sections (decisions 22–24), as milestone 9.0.7
//! decision 12 orders them: OUTCOME, EVIDENCE, INTENT, DETAIL.

use super::run_task_sections as project;
use super::run_tests::{app_of, inspect_node};
use super::{Section, SectionField};
use crate::app::App;
use crate::app::task_detail::{DetailState, TaskDetailCache, detail_key};
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{GEMINI_NOW, gemini_fixture, reviewer, worker};
use proto::{
    AgentRole, BlockInfo, BlockReason, CheckInfo, HoldInfo, HoldKind, HoldState, RunInfo, RunState,
    Runtime, SummarySource, TaskDetailInfo, TaskInfo, TaskState,
};

fn t2_key() -> NodeKey {
    NodeKey::Task {
        run: "r1".into(),
        id: "t2".into(),
    }
}

fn t2(run: &mut RunInfo) -> &mut TaskInfo {
    run.tasks
        .iter_mut()
        .find(|task| task.id == "t2")
        .expect("t2")
}

/// The Gemini fixture with `change` applied to run `r1`.
fn gemini_with(change: impl FnOnce(&mut RunInfo)) -> App {
    let (mut snapshot, windows) = gemini_fixture();
    change(&mut snapshot.runs[0]);
    app_of((snapshot, windows))
}

/// `t2`'s detail, landed.
fn with_detail(app: &mut App, brief: &str, summary: Option<(&str, SummarySource)>) {
    let task = app.runs.runs[0]
        .tasks
        .iter()
        .find(|task| task.id == "t2")
        .expect("t2");
    app.task_detail = Some(TaskDetailCache {
        run_id: "r1".into(),
        task_id: "t2".into(),
        key: detail_key(task),
        state: DetailState::Ready(Box::new(TaskDetailInfo {
            run_id: "r1".into(),
            task_id: "t2".into(),
            brief: brief.into(),
            acceptance: vec!["hooks map".into(), "stop marks idle".into()],
            worker_summary: summary.map(|(text, _)| text.to_owned()),
            summary_source: summary.map(|(_, source)| source),
        })),
    });
}

fn sections(app: &App) -> Vec<Section> {
    inspect_node(app, &t2_key()).sections
}

fn field<'a>(sections: &'a [Section], title: &str, label: &str) -> Option<&'a SectionField> {
    sections
        .iter()
        .find(|section| section.title == title)
        .unwrap_or_else(|| panic!("no section {title}"))
        .fields
        .iter()
        .find(|field| field.label == label)
}

fn value<'a>(sections: &'a [Section], title: &str, label: &str) -> &'a str {
    field(sections, title, label)
        .unwrap_or_else(|| panic!("no {title} {label}"))
        .value
        .as_str()
}

/// The panel's body rows at 76 columns, as text.
fn body(sections: &[Section]) -> Vec<String> {
    crate::inspector::panel::sections::body_lines(sections, 76, crate::theme::Palette::PLAIN)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

#[test]
fn sections_are_outcome_evidence_intent_detail_in_order() {
    let sections = sections(&app_of(gemini_fixture()));
    let titles: Vec<&str> = sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["OUTCOME", "EVIDENCE", "INTENT", "DETAIL"]);
    let labels = |n: usize| -> Vec<&str> { sections[n].fields.iter().map(|f| f.label).collect() };
    assert_eq!(labels(0), ["pipeline", "check", "accept"]);
    assert_eq!(labels(1), ["diff", "review"]);
    assert_eq!(labels(2), ["brief"]);
    assert_eq!(
        labels(3),
        [
            "phase", "worker", "deps", "budget", "tries", "route", "history"
        ]
    );
}

#[test]
fn goal_says_loading_before_the_detail() {
    let mut app = app_of(gemini_fixture());
    assert_eq!(value(&sections(&app), "INTENT", "brief"), "loading…");
    app.task_detail = Some(TaskDetailCache {
        run_id: "r1".into(),
        task_id: "t2".into(),
        key: detail_key(&app.runs.runs[0].tasks[0]),
        state: DetailState::InFlight(3),
    });
    assert_eq!(value(&sections(&app), "INTENT", "brief"), "loading…");
    app.task_detail.as_mut().unwrap().state = DetailState::Failed("no such task".into());
    assert_eq!(value(&sections(&app), "INTENT", "brief"), "no such task");
    // Another task's detail is not t2's.
    with_detail(&mut app, "t2's brief", None);
    app.task_detail.as_mut().unwrap().task_id = "t3".into();
    assert_eq!(value(&sections(&app), "INTENT", "brief"), "loading…");
}

/// The rows from `title`'s on.
fn from<'a>(rows: &'a [String], title: &str) -> &'a [String] {
    let at = rows
        .iter()
        .position(|row| row == title)
        .expect("the section");
    &rows[at..]
}

/// Milestone 9.0.7 decision 12: INTENT's brief is one wrapped line until `b`.
#[test]
fn intent_shows_one_brief_line_until_expanded() {
    let mut app = gemini_with(|run| t2(run).owns = vec!["src/a.rs".into(), "src/b.rs".into()]);
    with_detail(&mut app, "one\ntwo\nthree\nfour\nfive", None);
    let rows = body(&sections(&app));
    assert_eq!(
        from(&rows, "INTENT")[..4],
        [
            "INTENT",
            "brief     one",
            "          … (b: more)",
            "owns      src/a.rs, src/b.rs",
        ]
    );
    let at = rows.iter().position(|row| row == "OUTCOME").unwrap();
    assert_eq!(
        rows[at + 3..at + 5],
        ["accept    ◌ hooks map", "          ◌ stop marks idle"]
    );
    app.brief_expanded = Some(t2_key());
    let rows = body(&sections(&app));
    assert_eq!(
        from(&rows, "INTENT")[1..6],
        [
            "brief     one",
            "          two",
            "          three",
            "          four",
            "          five"
        ]
    );
    assert!(!rows.iter().any(|row| row.contains("b: more")));
    // Exactly one line: nothing to expand.
    app.brief_expanded = None;
    with_detail(&mut app, "one", None);
    assert!(
        !body(&sections(&app))
            .iter()
            .any(|row| row.contains("b: more"))
    );
}

#[test]
fn stage_words() {
    let (snapshot, _) = gemini_fixture();
    let mut run = snapshot.runs[0].clone();
    let mut task = t2(&mut run).clone();
    let cases = [
        (TaskState::Pending, "waiting for its dependencies"),
        (TaskState::Queued, "queued for a worker"),
        (TaskState::Preparing, "preparing its worktree"),
        (TaskState::Working, "worker is working"),
        (TaskState::Proof, "running the test proof"),
        (TaskState::Check, "running the check"),
        (TaskState::Review, "in review"),
        (TaskState::MergeQueue, "waiting to merge"),
        (TaskState::Merged, "merged"),
        (TaskState::Cancelled, "cancelled"),
        (TaskState::Reported, "report delivered"),
        (TaskState::Blocked, "blocked"),
    ];
    for (state, words) in cases {
        task.state = state;
        assert_eq!(project::stage_words(&run, &task), words, "{state:?}");
    }
    task.block = Some(BlockInfo {
        reason: BlockReason::Question,
        text: "which?".into(),
    });
    assert_eq!(project::stage_words(&run, &task), "blocked: question");
    task.block_source = Some(proto::DeciderSource::Fallback);
    assert_eq!(
        project::stage_words(&run, &task),
        "blocked: question (fallback)"
    );
    task.block = Some(BlockInfo {
        reason: BlockReason::MessagePause,
        text: String::new(),
    });
    assert_eq!(project::stage_words(&run, &task), "paused by a message");
    // Held by a hold that awaits approval; not by one that was approved.
    task.state = TaskState::Queued;
    task.hold = Some("h1".into());
    let mut hold = HoldInfo {
        id: "h1".into(),
        kind: HoldKind::Promotion,
        state: HoldState::Awaiting,
        tasks: vec!["t2".into()],
        created_at: 0,
        decided_at: None,
        decided_by: None,
    };
    run.holds = vec![hold.clone()];
    assert_eq!(
        project::stage_words(&run, &task),
        "queued for a worker · held"
    );
    hold.state = HoldState::Approved;
    run.holds = vec![hold];
    assert_eq!(project::stage_words(&run, &task), "queued for a worker");
    // At the gate every task is planned, whatever its state.
    run.state = RunState::AwaitingApproval;
    assert_eq!(project::stage_words(&run, &task), "planned, not started");
}

#[test]
fn worker_line() {
    let app = app_of(gemini_fixture());
    let task = &app.runs.runs[0]
        .tasks
        .iter()
        .find(|t| t.id == "t2")
        .unwrap();
    // Live: elapsed from the snapshot clock (`now − 1560`).
    assert_eq!(
        project::worker_line(task, &app).as_deref(),
        Some("worker #1 · codex · 26m · 41 tool calls")
    );
    let mut task = (*task).clone();
    // The latest worker round wins; a model is shown after the runtime; an ended
    // round's time is frozen at its end.
    let mut second = worker(2, None, Runtime::Claude, GEMINI_NOW - 3000);
    second.round = 3;
    second.route.model = "claude-opus-5".into();
    second.ended_at = Some(GEMINI_NOW - 3000 + 125);
    second.tool_calls = 7;
    let mut older = worker(1, None, Runtime::Codex, GEMINI_NOW - 9000);
    older.tool_calls = 99;
    task.rounds = vec![older, second.clone()];
    assert_eq!(
        project::worker_line(&task, &app).as_deref(),
        Some("worker #2 r3 · claude claude-opus-5 · 2m · 7 tool calls")
    );
    // No worker round: no line.
    task.rounds = vec![reviewer(1, None, Runtime::Claude, GEMINI_NOW)];
    assert_eq!(project::worker_line(&task, &app), None);
    assert_eq!(second.role, AgentRole::Worker);
}

#[test]
fn now_line_from_activity_and_reviewer_prefix() {
    let (snapshot, _) = gemini_fixture();
    let mut task = snapshot.runs[0]
        .tasks
        .iter()
        .find(|t| t.id == "t2")
        .unwrap()
        .clone();
    assert_eq!(project::now_line(&task), None, "no activity");
    // t2's live round is its second reviewer's.
    task.activity = Some("Read crates/daemon/src/hooks.rs".into());
    assert_eq!(
        project::now_line(&task).as_deref(),
        Some("reviewer: Read crates/daemon/src/hooks.rs")
    );
    // The worker live and the reviewers done: no prefix.
    task.rounds[0].ended_at = None;
    for round in &mut task.rounds[1..] {
        round.ended_at = Some(GEMINI_NOW);
    }
    task.activity = Some("Bash cargo test\n-p x".into());
    assert_eq!(
        project::now_line(&task).as_deref(),
        Some("Bash cargo test -p x")
    );
    // Shown in DETAIL as `now`.
    let app = gemini_with(|run| t2(run).activity = Some("Edit src/lib.rs".into()));
    assert_eq!(
        value(&sections(&app), "DETAIL", "now"),
        "reviewer: Edit src/lib.rs"
    );
}

#[test]
fn check_line() {
    let (snapshot, _) = gemini_fixture();
    let mut run = snapshot.runs[0].clone();
    let mut task = t2(&mut run).clone();
    let check = |ok: bool, timed_out: bool| CheckInfo {
        at: 0,
        ok,
        code: Some(i32::from(!ok)),
        timed_out,
        secs: 3,
        summary: "\n  raw tail line\nsecond".into(),
        on_candidate: false,
        decider_summary: None,
        summary_source: None,
        tier: None,
    };
    task.last_check = None;
    assert_eq!(project::check_line(&run, &task), "not yet");
    task.last_check = Some(check(true, false));
    task.state = TaskState::Review;
    assert_eq!(project::check_line(&run, &task), "✓ passed · raw tail line");
    task.state = TaskState::MergeQueue;
    assert_eq!(
        project::check_line(&run, &task),
        "✓ passed · raw tail line → merge queue"
    );
    // Failed, then bounced back to the worker: the decider's summary wins.
    let mut failed = check(false, false);
    failed.decider_summary = Some("2 tests fail in calc\nmore".into());
    task.last_check = Some(failed);
    task.state = TaskState::Working;
    task.bounces.check = 1;
    assert_eq!(
        project::check_line(&run, &task),
        "✗ failed · 2 tests fail in calc → bounced (check 1/2)"
    );
    task.state = TaskState::Blocked;
    assert_eq!(
        project::check_line(&run, &task),
        "✗ failed · 2 tests fail in calc → blocked"
    );
    task.last_check = Some(check(false, true));
    task.state = TaskState::Check;
    assert_eq!(
        project::check_line(&run, &task),
        "✗ timed out · raw tail line"
    );
    run.unverified = true;
    assert_eq!(project::check_line(&run, &task), "–");
}

#[test]
fn evidence_shows_the_diff_review_summary_and_merge() {
    let mut app = gemini_with(|run| {
        let task = t2(run);
        task.state = TaskState::Merged;
        task.merge_commit = Some("0123456789abcdef".into());
        task.merged_without_approval = Some("review timed out".into());
    });
    with_detail(
        &mut app,
        "brief",
        Some(("Mapped the hooks.\nAdded a test.", SummarySource::TaskDone)),
    );
    let sections = sections(&app);
    let evidence: Vec<(&str, Option<&str>, &str)> = sections[1]
        .fields
        .iter()
        .map(|f| (f.label, f.note, f.value.as_str()))
        .collect();
    assert_eq!(
        evidence,
        [
            (
                "diff",
                None,
                "+212 −31 · 4 files · test `status::gemini_stop_marks_idle` red a1b2c3d ✓"
            ),
            (
                "review",
                None,
                "r1 ✗ changes · 1 critical, 2 minor: status.rs:118 \"SubagentStop not paired\"\none pairing bug"
            ),
            (
                "summary",
                Some("task_done"),
                "Mapped the hooks.\nAdded a test."
            ),
            (
                "merged",
                None,
                "0123456 · without approval: review timed out"
            ),
        ]
    );
    let rows = body(&sections);
    let at = rows
        .iter()
        .position(|r| r == "summary (task_done)")
        .unwrap();
    assert_eq!(
        rows[at + 1..at + 3],
        ["          Mapped the hooks.", "          Added a test."]
    );
    // Merged by override: nothing is ticked, though a criterion is listed.
    assert_eq!(
        value(&sections, "OUTCOME", "accept"),
        "◌ hooks map\n◌ stop marks idle"
    );
    with_detail(
        &mut app,
        "brief",
        Some(("tail", SummarySource::LastMessage)),
    );
    assert_eq!(
        field(&self::sections(&app), "EVIDENCE", "summary")
            .unwrap()
            .note,
        Some("last message")
    );
}

/// RESULT's `nothing yet` is gone: with no diff and no verdict EVIDENCE says where the
/// review stands, and the pipeline's `◌` marks say what is not reached.
#[test]
fn evidence_without_a_diff_or_verdict_reads_the_review_state() {
    let app = gemini_with(|run| {
        let task = t2(run);
        task.state = TaskState::Working;
        task.reviews.clear();
        task.diff = None;
        task.test = None;
        task.red = None;
    });
    let sections = sections(&app);
    let evidence: Vec<(&str, &str)> = sections[1]
        .fields
        .iter()
        .map(|f| (f.label, f.value.as_str()))
        .collect();
    assert_eq!(evidence, [("review", "not yet")]);
    assert_eq!(
        value(&sections, "OUTCOME", "pipeline"),
        "done › proof ✓ › check ✓ › review ◌ › merge ◌"
    );
    assert!(
        !body(&sections)
            .iter()
            .any(|row| row.contains("nothing yet"))
    );
}

/// Pinning in spirit: nothing M8c's flat list showed is dropped.
#[test]
fn every_old_field_is_still_shown() {
    let app = gemini_with(|run| {
        let task = t2(run);
        task.message_count = 2;
        task.last_message_kind = Some(proto::MessageKind::Info);
        task.last_message_line = Some("use the new hook".into());
        task.task_notes = vec![proto::TaskNoteInfo {
            task_id: "t6".into(),
            kind: proto::TaskNoteKind::Risk,
            text: "watch the SubagentStop pairing".into(),
            at: GEMINI_NOW - 60,
        }];
    });
    let flat = inspect_node(&app, &t2_key()).fields;
    let sections = sections(&app);
    for (title, label) in [
        ("OUTCOME", "pipeline"),
        ("EVIDENCE", "diff"),
        ("EVIDENCE", "review"),
        ("DETAIL", "route"),
        ("DETAIL", "deps"),
        ("DETAIL", "budget"),
        ("DETAIL", "tries"),
        ("DETAIL", "messages"),
        ("DETAIL", "notes"),
        ("DETAIL", "history"),
    ] {
        let old = flat
            .iter()
            .find(|f| f.label == label)
            .unwrap_or_else(|| panic!("{label} is in the flat list"));
        assert_eq!(value(&sections, title, label), old.value, "{label}");
    }
    for old in &flat {
        let shown = sections
            .iter()
            .flat_map(|s| &s.fields)
            .any(|f| f.label == old.label);
        assert!(shown, "{} is shown", old.label);
    }
}

#[test]
fn worker_text_is_sanitised() {
    let hostile = hostile_text();
    let mut app = gemini_with(|run| {
        let task = t2(run);
        task.activity = Some(hostile.clone());
        task.state = TaskState::Merged;
        task.merge_commit = Some(hostile.clone());
        task.merged_without_approval = Some(hostile.clone());
        task.reviews[0].summary = hostile.clone();
    });
    with_detail(
        &mut app,
        &format!("{hostile}\n{hostile}"),
        Some((&hostile, SummarySource::TaskDone)),
    );
    app.brief_expanded = Some(t2_key());
    if let Some(DetailState::Ready(detail)) = app.task_detail.as_mut().map(|c| &mut c.state) {
        detail.acceptance = vec![hostile.clone()];
    }
    let sections = sections(&app);
    for row in body(&sections) {
        assert_eq!(first_hostile(&row), None, "{row:?}");
    }
    // Each planted value reached the panel.
    for (title, label) in [
        ("INTENT", "brief"),
        ("OUTCOME", "accept"),
        ("OUTCOME", "result"),
        ("DETAIL", "now"),
        ("EVIDENCE", "summary"),
        ("EVIDENCE", "review"),
        ("EVIDENCE", "merged"),
    ] {
        assert!(field(&sections, title, label).is_some(), "{title} {label}");
    }
}

/// Review of fd9dd25: one criterion or owns entry is one row, whatever line breaks
/// it carries, so it cannot forge another criterion (`◌` since milestone 9.0.7).
#[test]
fn a_list_entry_with_a_line_break_stays_one_row() {
    let mut app = gemini_with(|run| t2(run).owns = vec!["src/a.rs\nsrc/forged.rs".into()]);
    with_detail(&mut app, "brief", None);
    if let Some(DetailState::Ready(detail)) = app.task_detail.as_mut().map(|c| &mut c.state) {
        detail.acceptance = vec!["real\n☐ FORGED".into(), "x\r☐ FORGED2".into()];
    }
    let rows = body(&sections(&app));
    let criteria: Vec<&str> = rows
        .iter()
        .filter_map(|row| {
            row.split_once('☐')
                .map(|_| row.trim_start_matches("accept").trim())
        })
        .collect();
    assert_eq!(criteria, ["◌ real ☐ FORGED", "◌ x ☐ FORGED2"], "{rows:?}");
    assert!(
        rows.contains(&"owns      src/a.rs src/forged.rs".to_owned()),
        "{rows:?}"
    );
}
