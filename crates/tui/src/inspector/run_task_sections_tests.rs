//! M9.0.5.10: the task inspection as GOAL, STATUS and RESULT (decisions 22–24).

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
fn sections_are_goal_status_result_in_order() {
    let sections = sections(&app_of(gemini_fixture()));
    let titles: Vec<&str> = sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["GOAL", "STATUS", "RESULT"]);
    let labels = |n: usize| -> Vec<&str> { sections[n].fields.iter().map(|f| f.label).collect() };
    assert_eq!(labels(0), ["brief"]);
    assert_eq!(
        labels(1),
        [
            "stage", "worker", "check", "review", "stages", "route", "deps", "budget", "tries",
            "history"
        ]
    );
    assert_eq!(labels(2), ["verdict", "diff"]);
}

#[test]
fn goal_says_loading_before_the_detail() {
    let mut app = app_of(gemini_fixture());
    assert_eq!(value(&sections(&app), "GOAL", "brief"), "loading…");
    app.task_detail = Some(TaskDetailCache {
        run_id: "r1".into(),
        task_id: "t2".into(),
        key: detail_key(&app.runs.runs[0].tasks[0]),
        state: DetailState::InFlight(3),
    });
    assert_eq!(value(&sections(&app), "GOAL", "brief"), "loading…");
    app.task_detail.as_mut().unwrap().state = DetailState::Failed("no such task".into());
    assert_eq!(value(&sections(&app), "GOAL", "brief"), "no such task");
    // Another task's detail is not t2's.
    with_detail(&mut app, "t2's brief", None);
    app.task_detail.as_mut().unwrap().task_id = "t3".into();
    assert_eq!(value(&sections(&app), "GOAL", "brief"), "loading…");
}

#[test]
fn goal_shows_three_brief_lines_until_expanded() {
    let mut app = gemini_with(|run| t2(run).owns = vec!["src/a.rs".into(), "src/b.rs".into()]);
    with_detail(&mut app, "one\ntwo\nthree\nfour\nfive", None);
    let rows = body(&sections(&app));
    assert_eq!(
        rows[..8],
        [
            "GOAL",
            "brief     one",
            "          two",
            "          three",
            "          … (b: more)",
            "owns      src/a.rs, src/b.rs",
            "done when ☐ hooks map",
            "          ☐ stop marks idle",
        ]
    );
    app.brief_expanded = Some(t2_key());
    let rows = body(&sections(&app));
    assert_eq!(rows[4..6], ["          four", "          five"]);
    assert!(!rows.iter().any(|row| row.contains("b: more")));
    // Exactly three lines: nothing to expand.
    app.brief_expanded = None;
    with_detail(&mut app, "one\ntwo\nthree", None);
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
    // Shown in STATUS as `now`.
    let app = gemini_with(|run| t2(run).activity = Some("Edit src/lib.rs".into()));
    assert_eq!(
        value(&sections(&app), "STATUS", "now"),
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
fn result_shows_the_summary_verdict_diff_and_merge() {
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
    let result: Vec<(&str, Option<&str>, &str)> = sections[2]
        .fields
        .iter()
        .map(|f| (f.label, f.note, f.value.as_str()))
        .collect();
    assert_eq!(
        result,
        [
            (
                "summary",
                Some("task_done"),
                "Mapped the hooks.\nAdded a test."
            ),
            ("verdict", None, "changes · one pairing bug"),
            (
                "diff",
                None,
                "4 files · +212 −31 · test `status::gemini_stop_marks_idle` red a1b2c3d ✓"
            ),
            (
                "merged",
                None,
                "0123456 · without approval: review timed out"
            ),
        ]
    );
    let rows = body(&sections);
    let at = rows.iter().position(|r| r == "RESULT").unwrap();
    assert_eq!(
        rows[at + 1..at + 4],
        [
            "summary (task_done)",
            "          Mapped the hooks.",
            "          Added a test.",
        ]
    );
    with_detail(
        &mut app,
        "brief",
        Some(("tail", SummarySource::LastMessage)),
    );
    assert_eq!(
        field(&self::sections(&app), "RESULT", "summary")
            .unwrap()
            .note,
        Some("last message")
    );
}

#[test]
fn result_reads_nothing_yet() {
    let app = gemini_with(|run| {
        let task = t2(run);
        task.reviews.clear();
        task.diff = None;
        task.test = None;
        task.red = None;
    });
    let sections = sections(&app);
    let result: Vec<(&str, &str)> = sections[2]
        .fields
        .iter()
        .map(|f| (f.label, f.value.as_str()))
        .collect();
    assert_eq!(result, [("", "nothing yet")]);
    let rows = body(&sections);
    assert_eq!(rows[rows.len() - 2..], ["RESULT", "nothing yet"]);
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
    for label in [
        "stages", "route", "deps", "budget", "tries", "messages", "notes", "history",
    ] {
        let old = flat
            .iter()
            .find(|f| f.label == label)
            .unwrap_or_else(|| panic!("{label} is in the flat list"));
        assert_eq!(value(&sections, "STATUS", label), old.value, "{label}");
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
        ("GOAL", "brief"),
        ("GOAL", "done when"),
        ("STATUS", "now"),
        ("RESULT", "summary"),
        ("RESULT", "verdict"),
        ("RESULT", "merged"),
    ] {
        assert!(field(&sections, title, label).is_some(), "{title} {label}");
    }
}

/// Review of fd9dd25: one criterion or owns entry is one row, whatever line breaks
/// it carries, so it cannot forge another `☐` criterion.
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
                .map(|_| row.trim_start_matches("done when").trim())
        })
        .collect();
    assert_eq!(criteria, ["☐ real ☐ FORGED", "☐ x ☐ FORGED2"], "{rows:?}");
    assert!(
        rows.contains(&"owns      src/a.rs src/forged.rs".to_owned()),
        "{rows:?}"
    );
}

/// Ruling D-2: a merged or reported task's outcome heads STATUS, so it shows without
/// scrolling; the sections keep decision 22's order.
#[test]
fn a_finished_task_leads_status_with_its_result() {
    for state in [TaskState::Merged, TaskState::Reported] {
        let mut app = gemini_with(|run| {
            let task = t2(run);
            task.state = state;
            task.merge_commit = Some("0123456789".into());
        });
        with_detail(
            &mut app,
            "brief",
            Some((
                "\n  Mapped the hooks.  \nAdded a test.",
                SummarySource::TaskDone,
            )),
        );
        let sections = sections(&app);
        let titles: Vec<&str> = sections.iter().map(|s| s.title).collect();
        assert_eq!(titles, ["GOAL", "STATUS", "RESULT"]);
        let first = &sections[1].fields[0];
        assert_eq!(
            (first.label, first.value.as_str()),
            ("result", "Mapped the hooks."),
            "{state:?}"
        );
        // Before the detail lands: RESULT's first line stands in.
        let mut app = gemini_with(|run| {
            let task = t2(run);
            task.state = state;
            task.merge_commit = Some("0123456789".into());
        });
        app.task_detail = None;
        assert_eq!(
            value(&self::sections(&app), "STATUS", "result"),
            "changes · one pairing bug"
        );
    }
    // A task still in review has no result line.
    let app = app_of(gemini_fixture());
    assert!(field(&sections(&app), "STATUS", "result").is_none());
}
