//! M8c.7: the task projection (Interfaces "Inspector contents, exact": Task). Each test compares the exact `(label, value)` list, the
//! name and the right-hand text.

use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::app::App;
use crate::inspector::FieldLayout;
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{GEMINI_NOW, gemini_fixture};
use proto::{
    BlockInfo, BlockReason, DeciderSource, Finding, RunInfo, Severity, Size, SizeCheckInfo,
    TaskEventInfo, TaskInfo, TaskState, TestMode,
};

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: "r1".into(),
        id: id.into(),
    }
}

/// The Gemini fixture with `change` applied to task `id`.
pub(super) fn with_task(id: &str, change: impl FnOnce(&mut TaskInfo)) -> App {
    with_run(|run| {
        let task = run
            .tasks
            .iter_mut()
            .find(|task| task.id == id)
            .expect("task");
        change(task);
    })
}

fn with_run(change: impl FnOnce(&mut RunInfo)) -> App {
    let (mut snapshot, windows) = gemini_fixture();
    change(&mut snapshot.runs[0]);
    app_of((snapshot, windows))
}

const T2_HISTORY: &str = "12:31 review r1 changes · 12:20 check passed · 12:02 started";

#[test]
fn task_fields_match_the_mockup() {
    let app = app_of(gemini_fixture());
    let inspection = inspect_node(&app, &task_key("t2"));
    assert_eq!(inspection.glyph.content, "◐");
    assert_eq!(inspection.name, "t2  map Gemini hook events to status");
    assert_eq!(
        inspection.right.as_deref(),
        Some("M · tdd · review round 2")
    );
    assert_eq!(inspection.layout, FieldLayout::Rows);
    assert_eq!(
        pairs(&inspection),
        [
            ("stages", "done ✓ → proof ✓ → check ✓ → review ● → merge ·"),
            (
                "route",
                "codex · standard · high effort  →  reviewer claude · frontier"
            ),
            (
                "deps",
                "waits on t0 ✓ t6 ✓ · unblocks t3, t7 · on critical path"
            ),
            (
                "budget",
                "███████░░░ 104/150 tool calls · 38/60 min · 410k tokens"
            ),
            (
                "tries",
                "review 1/2 bounces · check 0/2 · escalation step 1"
            ),
            (
                "diff",
                "4 files · +212 −31 · test `status::gemini_stop_marks_idle` red a1b2c3d ✓"
            ),
            (
                "review",
                "r1 ✗ 1 critical, 2 minor: status.rs:118 \"SubagentStop not paired\""
            ),
            ("history", T2_HISTORY),
        ]
    );
}

fn stages(app: &App, id: &str) -> String {
    value(&inspect_node(app, &task_key(id)), "stages")
        .expect("a stages row")
        .to_owned()
}

#[test]
fn task_stage_marks() {
    let check_mode = with_task("t2", |task| task.test_mode = TestMode::Check);
    assert_eq!(
        stages(&check_mode, "t2"),
        "done ✓ → proof – → check ✓ → review ● → merge ·"
    );
    let unverified = with_run(|run| run.unverified = true);
    assert_eq!(
        stages(&unverified, "t2"),
        "done ✓ → proof ✓ → check – → review ● → merge ·"
    );
    let unreviewed = with_task("t2", |task| {
        task.review_route = None;
        task.size = Size::S;
        task.state = TaskState::Check;
    });
    assert_eq!(
        stages(&unreviewed, "t2"),
        "done ✓ → proof ✓ → check ● → review – → merge ·"
    );
    let queued = with_task("t2", |task| {
        task.state = TaskState::MergeQueue;
        task.reviews[1].verdict = Some(proto::Verdict::Approve);
    });
    assert_eq!(
        stages(&queued, "t2"),
        "done ✓ → proof ✓ → check ✓ → review ✓ → merge ●"
    );
    let working = with_task("t2", |task| {
        task.state = TaskState::Working;
        task.done_signal = None;
        task.last_proof = None;
        task.last_check.as_mut().expect("check").ok = false;
    });
    assert_eq!(
        stages(&working, "t2"),
        "done ● → proof · → check ✗ → review ✗ → merge ·"
    );
    let proving = with_task("t2", |task| {
        task.state = TaskState::Proof;
        task.reviews.clear();
    });
    assert_eq!(
        stages(&proving, "t2"),
        "done ✓ → proof ● → check ✓ → review · → merge ·"
    );
    let failed_proof = with_task("t2", |task| {
        task.last_proof.as_mut().expect("proof").ok = false;
        task.state = TaskState::Merged;
    });
    assert_eq!(
        stages(&failed_proof, "t2"),
        "done ✓ → proof ✗ → check ✓ → review ✗ → merge ✓"
    );
}

#[test]
fn task_deps_include_implicit_ones_once() {
    let app = with_task("t3", |task| {
        task.implicit_deps = vec!["t2".into(), "t0".into(), "t3".into()];
    });
    let inspection = inspect_node(&app, &task_key("t3"));
    assert_eq!(
        value(&inspection, "deps"),
        Some("waits on t2 ◐ t0 ✓ · on critical path")
    );
    // Implicit dependents unblock too, once each.
    let app = with_task("t5", |task| task.implicit_deps = vec!["t2".into()]);
    let inspection = inspect_node(&app, &task_key("t2"));
    assert_eq!(
        value(&inspection, "deps"),
        Some("waits on t0 ✓ t6 ✓ · unblocks t3, t5, t7 · on critical path")
    );
    // A working dependency's glyph is a static `●`, never the spinner (controller
    // ruling), though its worker's window is `Working` and the spinner has moved on.
    let mut app = with_task("t3", |task| task.deps = vec!["t2".into()]);
    app.spinner_frame = 3;
    let t2 = app.runs.runs[0]
        .tasks
        .iter_mut()
        .find(|t| t.id == "t2")
        .expect("t2");
    t2.state = TaskState::Working;
    let inspection = inspect_node(&app, &task_key("t3"));
    assert_eq!(
        value(&inspection, "deps"),
        Some("waits on t2 ● · on critical path")
    );
    // No deps, no dependents, off the critical path: no row.
    let inspection = inspect_node(&app, &task_key("t8"));
    assert_eq!(value(&inspection, "deps"), None);
}

#[test]
fn task_fields_when_blocked() {
    let app = with_task("t5", |task| {
        task.block = Some(BlockInfo {
            reason: BlockReason::MisSized,
            text: "too big".into(),
        });
    });
    let inspection = inspect_node(&app, &task_key("t5"));
    assert_eq!(inspection.glyph.content, "⊘");
    assert_eq!(
        inspection.right.as_deref(),
        Some("S · tdd · blocked: mis-sized")
    );
    let words = [
        (BlockReason::Human, "human"),
        (BlockReason::Conflict, "conflict"),
        (BlockReason::DepCancelled, "dependency cancelled"),
        (BlockReason::Question, "question"),
        (BlockReason::Environment, "environment"),
    ];
    for (reason, word) in words {
        let app = with_task("t5", |task| {
            task.block.as_mut().expect("block").reason = reason;
        });
        let right = inspect_node(&app, &task_key("t5")).right;
        assert_eq!(right, Some(format!("S · tdd · blocked: {word}")));
    }
}

#[test]
fn task_stage_text_for_every_state() {
    let cases = [
        (TaskState::Pending, "waiting"),
        (TaskState::Queued, "queued"),
        (TaskState::Preparing, "preparing"),
        (TaskState::Working, "working"),
        (TaskState::Proof, "test proof"),
        (TaskState::Check, "check"),
        (TaskState::MergeQueue, "merge queue"),
        (TaskState::Merged, "merged"),
        (TaskState::Cancelled, "cancelled"),
    ];
    for (state, stage) in cases {
        let app = with_task("t8", |task| {
            task.state = state;
            task.test_mode = TestMode::None;
        });
        let right = inspect_node(&app, &task_key("t8")).right;
        assert_eq!(right, Some(format!("S · none · {stage}")), "{state:?}");
    }
    let gate = with_run(|run| run.state = proto::RunState::AwaitingApproval);
    let inspection = inspect_node(&gate, &task_key("t8"));
    assert_eq!(inspection.right.as_deref(), Some("S · tdd · planned"));
    assert_eq!(inspection.glyph.content, "○");
}

#[test]
fn task_without_diff_or_review_omits_them() {
    let app = app_of(gemini_fixture());
    let inspection = inspect_node(&app, &task_key("t0"));
    assert_eq!(inspection.name, "t0  t0 work");
    assert_eq!(inspection.right.as_deref(), Some("S · tdd · merged"));
    assert_eq!(
        pairs(&inspection),
        [
            ("stages", "done · → proof · → check · → review – → merge ✓"),
            ("route", "claude · standard · medium effort"),
            ("deps", "unblocks t2 · on critical path"),
            (
                "budget",
                "░░░░░░░░░░ 0/100 tool calls · 0/30 min · 0 tokens"
            ),
            ("tries", "review 0/2 bounces · check 0/2"),
        ]
    );
    // The test part alone while a task is live, with no proof mark before a proof.
    let app = with_task("t2", |task| {
        task.diff = None;
        task.last_proof = None;
    });
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "diff"),
        Some("test `status::gemini_stop_marks_idle` red a1b2c3d")
    );
}

#[test]
fn task_budget_with_tokens() {
    let app = with_task("t2", |task| task.budget.tokens = Some(3_000_000));
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "budget"),
        Some("███████░░░ 104/150 tool calls · 38/60 min · 410k/3.0M tokens")
    );
    // The minute fraction wins when it is larger; an empty budget never divides by 0.
    let app = with_task("t2", |task| task.spent_session.secs = 3300);
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "budget"),
        Some("█████████░ 104/150 tool calls · 55/60 min · 410k tokens")
    );
    let app = with_task("t2", |task| {
        task.budget.tool_calls = 0;
        task.budget.minutes = 0;
    });
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "budget"),
        Some("██████████ 104/0 tool calls · 38/0 min · 410k tokens")
    );
}

#[test]
fn task_tries_every_counter() {
    let app = with_task("t2", |task| {
        task.bounces.proof = 1;
        task.bounces.merge = 2;
        task.bounces.check = 1;
        task.stalls = 3;
        task.rung = 0;
    });
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "tries"),
        Some("review 1/2 bounces · check 1/2 · proof 1/2 · merge 2/2 · stalls 3")
    );
}

#[test]
fn task_size_raised_by_the_cross_check() {
    let size_check = |agreed, decided| SizeCheckInfo {
        engine: Size::S,
        decided,
        agreed,
        reason: "touches two crates".into(),
        source: DeciderSource::Decider,
    };
    let app = with_task("t2", |task| {
        task.size_check = Some(size_check(false, Some(Size::M)));
    });
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "tries"),
        Some("review 1/2 bounces · check 0/2 · escalation step 1 · size raised S→M")
    );
    for (agreed, decided) in [(true, Some(Size::M)), (false, None)] {
        let app = with_task("t2", |task| {
            task.size_check = Some(size_check(agreed, decided));
        });
        assert_eq!(
            value(&inspect_node(&app, &task_key("t2")), "tries"),
            Some("review 1/2 bounces · check 0/2 · escalation step 1")
        );
    }
}

#[test]
fn task_blocked_by_the_fallback() {
    let app = with_task("t5", |task| {
        task.block_source = Some(DeciderSource::Fallback)
    });
    assert_eq!(
        inspect_node(&app, &task_key("t5")).right.as_deref(),
        Some("S · tdd · blocked: question (fallback)")
    );
    let app = with_task("t5", |task| {
        task.block_source = Some(DeciderSource::Decider)
    });
    assert_eq!(
        inspect_node(&app, &task_key("t5")).right.as_deref(),
        Some("S · tdd · blocked: question")
    );
}

#[test]
fn a_review_with_no_findings_and_a_finding_without_a_file() {
    let app = with_task("t2", |task| {
        task.reviews[0].findings.clear();
        task.reviews[0].blocking = false;
    });
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "review"),
        Some("r1 ✓ no findings")
    );
    let app = with_task("t2", |task| {
        task.reviews[0].findings = vec![Finding {
            severity: Severity::Important,
            file: None,
            line: None,
            input: Some("the plan".into()),
            text: "t4 overlaps".into(),
        }];
    });
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "review"),
        Some("r1 ✗ 1 important: the plan \"t4 overlaps\"")
    );
}

#[test]
fn task_history_is_local_time() {
    let mut app = app_of(gemini_fixture());
    app.utc_offset_secs = 3600;
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "history"),
        Some("13:31 review r1 changes · 13:20 check passed · 13:02 started")
    );
}

#[test]
fn a_long_history_shows_the_newest_ten() {
    let app = with_task("t2", |task| {
        task.history = (0..500)
            .map(|index| TaskEventInfo {
                at: GEMINI_NOW - index * 60,
                text: format!("e{index}"),
            })
            .collect();
    });
    let history = value(&inspect_node(&app, &task_key("t2")), "history")
        .expect("history")
        .to_owned();
    assert_eq!(history.split(" · ").count(), 10);
    assert!(history.starts_with("12:40 e0 · 12:39 e1"), "{history}");
    assert!(history.ends_with("12:31 e9"), "{history}");
}

#[test]
fn a_minor_only_review_is_a_check_with_its_findings() {
    let app = with_task("t2", |task| {
        task.reviews[0].blocking = false;
        task.reviews[0].findings.remove(1);
    });
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "review"),
        Some("r1 ✓ 2 minor: hooks.rs:12 \"naming\"")
    );
}

/// A task's title reaches the name through `clean`: no control character, bounded.
#[test]
fn a_hostile_title_is_cleaned_in_the_name() {
    let app = with_task("t2", |task| {
        task.title = format!("map\u{1b}[2J\nhooks{}", "z".repeat(400));
    });
    let inspection = inspect_node(&app, &task_key("t2"));
    let expected = format!("map [2J hooks{}…", "z".repeat(300 - 13));
    assert_eq!(inspection.name, format!("t2  {expected}"));
}
