//! M8c.7: the run projection and the helpers every run-inspection test shares
//! (decisions 29–31a, Interfaces "Inspector contents, exact"). Each test compares the
//! exact `(label, value)` list, the name and the right-hand text.

use crate::app::App;
use crate::inspector::{FieldLayout, Inspection, inspect, local_hhmm};
use crate::settings::UiSettings;
use crate::tree::run_fixtures::{GEMINI_DAY, GEMINI_NOW, RUN_ID, gate_fixture, gemini_fixture};
use crate::tree::{NodeKey, RunFilter, run_rows};
use proto::{
    BaseMovedInfo, BlockInfo, BlockReason, DaemonMsg, PlanEditInfo, RunPath, RunReply, RunState,
    RunUsage, RunsSnapshot, Scale, TaskKind, TaskState, TokenUsage, TriageInfo, WindowInfo,
};

pub(super) fn app_of((snapshot, windows): (RunsSnapshot, Vec<WindowInfo>)) -> App {
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot)));
    app
}

/// Inspects the run-view row `key` names in the snapshot's first run.
pub(super) fn inspect_node(app: &App, key: &NodeKey) -> Inspection {
    let rows = run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    let row = rows
        .iter()
        .find(|row| &row.key == key)
        .unwrap_or_else(|| panic!("no run-view row {key:?}"));
    inspect(row, app)
}

pub(super) fn pairs(inspection: &Inspection) -> Vec<(&str, &str)> {
    inspection
        .fields
        .iter()
        .map(|field| (field.label, field.value.as_str()))
        .collect()
}

pub(super) fn value<'a>(inspection: &'a Inspection, label: &str) -> Option<&'a str> {
    inspection
        .fields
        .iter()
        .find(|field| field.label == label)
        .map(|field| field.value.as_str())
}

fn run_key(id: &str) -> NodeKey {
    NodeKey::Run(id.into())
}

fn gemini_with(change: impl FnOnce(&mut proto::RunInfo)) -> App {
    let (mut snapshot, windows) = gemini_fixture();
    change(&mut snapshot.runs[0]);
    app_of((snapshot, windows))
}

fn gate_with(change: impl FnOnce(&mut proto::RunInfo)) -> App {
    let (mut snapshot, windows) = gate_fixture();
    change(&mut snapshot.runs[0]);
    app_of((snapshot, windows))
}

const GATE_PROGRESS: &str = "░░░░░░░░░░░░░░░░░░  0/2 merged · 2 waiting";
const GATE_AGENTS: &str = "workers 0/3 · readers 0/3 · claude ok";
const NO_SPEND: &str = "tokens 0 · tool calls 0";

#[test]
fn run_fields_match_the_mockup() {
    let app = app_of(gemini_fixture());
    let inspection = inspect_node(&app, &run_key("r1"));
    assert_eq!(inspection.glyph.content, "◉");
    assert_eq!(inspection.name, "r1  Add Gemini runtime");
    assert_eq!(inspection.right.as_deref(), Some("running · 1h12m"));
    assert_eq!(inspection.layout, FieldLayout::Rows);
    assert_eq!(
        pairs(&inspection),
        [
            (
                "progress",
                "██████████░░░░░░░░  5/9 merged · 2 working · 1 review · 1 blocked"
            ),
            (
                "path",
                "critical path t0 → t6 → t2 → t3 · 2 tasks left · 1.2× the bound"
            ),
            (
                "agents",
                "workers 3/3 · readers 1/3 · claude ok · codex rate-limited 4m"
            ),
            (
                "spend",
                "tokens 1.8M (cache 71%) · tool calls 612 · est. left ~40m"
            ),
            (
                "gate",
                "plan approved 11:02 · 2 plan edits since · last: split t2"
            ),
            (
                "attention",
                "t5 blocked: question — \"Gemini has no subagent-stop event\""
            ),
        ]
    );
}

#[test]
fn run_fields_at_the_gate() {
    let app = app_of(gate_fixture());
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(inspection.name, "add-reset-3f9a  Add password reset");
    assert_eq!(
        inspection.right.as_deref(),
        Some("awaiting approval · 2h46m")
    );
    assert_eq!(
        pairs(&inspection),
        [
            ("progress", GATE_PROGRESS),
            ("agents", GATE_AGENTS),
            ("spend", NO_SPEND),
            (
                "gate",
                "awaiting approval · a approve · x reject · e edit · d remove"
            ),
        ]
    );
}

#[test]
fn run_fields_when_halted() {
    let reason = "refs/heads/anthrex/add-reset-3f9a/integration moved from 1a2b3c4 to 5d6e7f8";
    let app = gate_with(|run| {
        run.state = RunState::Halted;
        run.approved_by = Some("user".into());
        run.halted_reason = Some(reason.into());
    });
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(inspection.right.as_deref(), Some("halted · 2h46m"));
    let halted = format!("halted: {reason}");
    assert_eq!(
        pairs(&inspection),
        [
            ("progress", GATE_PROGRESS),
            ("agents", GATE_AGENTS),
            ("spend", NO_SPEND),
            ("gate", "plan approved"),
            ("attention", halted.as_str()),
        ]
    );
}

#[test]
fn run_fields_when_the_base_advanced() {
    let line = "base main moved from 1a2b3c4 to 5d6e7f8 (2 new commits); accept will list them";
    let app = gate_with(|run| {
        run.state = RunState::Running;
        run.approved_by = Some("user".into());
        run.base_moved = Some(BaseMovedInfo {
            from: "1a2b3c4d5e".into(),
            to: "5d6e7f8a9b".into(),
            commits: vec![],
            total: 2,
        });
        run.attention = vec![line.into()];
    });
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(inspection.right.as_deref(), Some("running · 2h46m"));
    assert_eq!(
        pairs(&inspection),
        [
            ("progress", GATE_PROGRESS),
            ("agents", GATE_AGENTS),
            ("spend", NO_SPEND),
            ("gate", "plan approved"),
            ("attention", line),
        ]
    );
}

#[test]
fn run_fields_with_one_edit_and_yes() {
    let mut app = gate_with(|run| {
        run.state = RunState::Running;
        run.approved_by = Some("--yes".into());
        run.approved_at = Some(GEMINI_DAY + 7 * 3600 + 15 * 60);
        run.plan_edits = vec![PlanEditInfo {
            at: GEMINI_DAY + 8 * 3600,
            text: "cancel t3".into(),
        }];
        run.plan_edits_since_approval = 1;
    });
    app.utc_offset_secs = 7200;
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(
        value(&inspection, "gate"),
        Some("plan approved by --yes 09:15 · 1 plan edit since · last: cancel t3")
    );
}

#[test]
fn run_spend_without_cache_or_estimate() {
    let app = app_of(gate_fixture());
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(value(&inspection, "spend"), Some(NO_SPEND));

    // With no `usage`, the rounds' own usage is summed, cache share included.
    let app = gate_with(|run| {
        let mut round = crate::tree::run_fixtures::worker(1, None, proto::Runtime::Claude, 1);
        round.usage = TokenUsage {
            input: 1500,
            output: 0,
            cache_read: 500,
            cache_write: 0,
        };
        run.tasks[0].rounds = vec![round];
        run.tasks[0].spent_total.tool_calls = 7;
    });
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(
        value(&inspection, "spend"),
        Some("tokens 1k (cache 25%) · tool calls 7")
    );
}

#[test]
fn run_fields_on_the_fast_path() {
    let app = gate_with(|run| {
        run.state = RunState::Running;
        run.path = Some(RunPath::Fast);
        run.approved_by = Some("fast path".into());
        run.approved_at = Some(GEMINI_DAY);
        run.triage = Some(TriageInfo {
            kinds: vec![TaskKind::Code],
            scale: Scale::Single,
            path: RunPath::Fast,
            reason: "one file".into(),
            source: proto::DeciderSource::Decider,
            fallback_reason: None,
            at: 0,
        });
    });
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(
        value(&inspection, "gate"),
        Some("fast path · no plan gate · triage: code/single (decider)")
    );
    // Without triage, only the path.
    let app = gate_with(|run| {
        run.state = RunState::Running;
        run.path = Some(RunPath::Fast);
        run.approved_by = Some("fast path".into());
    });
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(value(&inspection, "gate"), Some("fast path · no plan gate"));
}

#[test]
fn attention_counts_each_thing_once() {
    let app = gemini_with(|run| {
        run.tasks[6].state = TaskState::Blocked;
        run.tasks[6].block = Some(BlockInfo {
            reason: BlockReason::Human,
            text: "needs a key".into(),
        });
        run.attention = vec![
            "t5 blocked (question): Gemini has no subagent-stop event".into(),
            "t6 blocked (human): needs a key".into(),
            "base main moved from 1a2b3c4 to 5d6e7f8 (2 new commits); accept will list them".into(),
        ];
    });
    let inspection = inspect_node(&app, &run_key("r1"));
    assert_eq!(
        value(&inspection, "attention"),
        Some("t5 blocked: question — \"Gemini has no subagent-stop event\" · +2 more")
    );
}

#[test]
fn attention_without_blocked_tasks_shows_the_first_other_line_and_counts_the_rest() {
    let app = gate_with(|run| {
        run.state = RunState::Running;
        run.attention = vec!["final check failed on the run head".into(), "second".into()];
    });
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(
        value(&inspection, "attention"),
        Some("final check failed on the run head · +1 more")
    );
    // Halted counts every blocked task and every other line.
    let app = gemini_with(|run| {
        run.state = RunState::Halted;
        run.halted_reason = Some("integration moved".into());
    });
    let inspection = inspect_node(&app, &run_key("r1"));
    assert_eq!(
        value(&inspection, "attention"),
        Some("halted: integration moved · +1 more")
    );
}

#[test]
fn a_promotion_line_carries_the_local_request_time() {
    let mut app = gate_with(|run| {
        run.state = RunState::Running;
        run.promote_requested_at = Some(GEMINI_DAY + 11 * 3600 + 40 * 60);
        run.attention = vec![
            "promotion requested; it takes effect when the orchestrator exists (milestone 9)"
                .into(),
        ];
    });
    app.utc_offset_secs = -3600;
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(
        value(&inspection, "attention"),
        Some(
            "promotion requested at 10:40; it takes effect when the orchestrator exists \
             (milestone 9)"
        )
    );
}

#[test]
fn a_paused_run_says_where_from() {
    let app = gemini_with(|run| {
        run.state = RunState::Paused;
        run.paused_from = Some(RunState::Running);
    });
    let inspection = inspect_node(&app, &run_key("r1"));
    assert_eq!(
        inspection.right.as_deref(),
        Some("paused (from running) · 1h12m")
    );
}

#[test]
fn local_hhmm_cases() {
    assert_eq!(local_hhmm(39_720, 0), "11:02");
    assert_eq!(local_hhmm(39_720, 3600), "12:02");
    assert_eq!(local_hhmm(39_720, -43_200), "23:02");
    assert_eq!(local_hhmm(39_720, 50_400), "01:02");
    // A negative offset that crosses midnight backwards, and hostile extremes.
    assert_eq!(local_hhmm(GEMINI_DAY + 600, -3600), "23:10");
    assert_eq!(local_hhmm(0, -1), "23:59");
    assert_eq!(local_hhmm(u64::MAX, 0), "07:00");
    assert_eq!(
        local_hhmm(u64::MAX, i64::MIN),
        local_hhmm(u64::MAX, i64::MIN)
    );
}

#[test]
fn run_with_no_tasks_says_so() {
    let app = gate_with(|run| run.tasks.clear());
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(value(&inspection, "progress"), Some("no tasks yet"));
    assert_eq!(
        value(&inspection, "agents"),
        Some("workers 0/3 · readers 0/3")
    );
}

#[test]
fn cancelled_tasks_are_out_of_the_total_and_appended() {
    let app = gate_with(|run| run.tasks[1].state = TaskState::Cancelled);
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(
        value(&inspection, "progress"),
        Some("░░░░░░░░░░░░░░░░░░  0/1 merged · 1 waiting · 1 cancelled")
    );
}

/// Spec §16.4 bounds the critical path over non-cancelled tasks: a cancelled task on it
/// is not left, like a merged one.
#[test]
fn a_cancelled_task_on_the_critical_path_is_not_left() {
    let app = gemini_with(|run| {
        let t3 = run.tasks.iter_mut().find(|t| t.id == "t3").expect("t3");
        t3.state = TaskState::Cancelled;
    });
    let inspection = inspect_node(&app, &run_key("r1"));
    assert_eq!(
        value(&inspection, "path"),
        Some("critical path t0 → t6 → t2 → t3 · 1 task left · 1.2× the bound")
    );
}

#[test]
fn every_progress_category_in_order() {
    let states = [
        TaskState::Merged,
        TaskState::Preparing,
        TaskState::Working,
        TaskState::Proof,
        TaskState::Check,
        TaskState::Review,
        TaskState::MergeQueue,
        TaskState::Blocked,
        TaskState::Pending,
        TaskState::Queued,
    ];
    let app = gate_with(|run| {
        run.state = RunState::Running;
        run.tasks = states
            .iter()
            .enumerate()
            .map(|(index, state)| {
                crate::tree::run_fixtures::task(&format!("t{index}"), "x", proto::Size::S, *state)
            })
            .collect();
    });
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(
        value(&inspection, "progress"),
        Some(
            "██░░░░░░░░░░░░░░░░  1/10 merged · 2 working · 2 checking · 1 review · 1 merging \
             · 1 blocked · 2 waiting"
        )
    );
}

#[test]
fn hostile_run_values_never_panic_and_stay_one_line() {
    let app = gemini_with(|run| {
        let huge = TokenUsage {
            input: u64::MAX,
            output: u64::MAX,
            cache_read: u64::MAX,
            cache_write: u64::MAX,
        };
        run.usage = Some(RunUsage {
            total: huge,
            ..RunUsage::default()
        });
        run.estimate_left_secs = Some(u64::MAX);
        run.bound_ratio_permille = Some(u32::MAX);
        run.approved_at = Some(GEMINI_NOW + 86_400 * 400);
        run.created_at = u64::MAX;
        run.tasks[5].block = Some(BlockInfo {
            reason: BlockReason::Question,
            text: format!("line one\nline\ttwo\u{1b}[31m{}", "x".repeat(10_000)),
        });
        for task in &mut run.tasks {
            task.spent_total.tool_calls = u32::MAX;
        }
    });
    let inspection = inspect_node(&app, &run_key("r1"));
    assert_eq!(
        value(&inspection, "spend"),
        Some(
            "tokens 18446744073709.5M (cache 33%) · tool calls 38654705655 · est. left \
             ~5124095576030431h00m"
        )
    );
    assert_eq!(inspection.right.as_deref(), Some("running · 0s"));
    // An approval stamp 400 days ahead of the daemon's `now` is formatted, not aged.
    assert_eq!(
        value(&inspection, "gate"),
        Some("plan approved 12:40 · 2 plan edits since · last: split t2")
    );
    for field in &inspection.fields {
        assert!(
            !field.value.chars().any(char::is_control),
            "{}: {:?}",
            field.label,
            field.value
        );
        assert!(field.value.chars().count() <= 400, "{}", field.label);
    }
}

#[test]
fn a_run_past_the_gate_that_nobody_approved_says_so() {
    let app = gate_with(|run| run.state = RunState::Paused);
    let inspection = inspect_node(&app, &run_key(RUN_ID));
    assert_eq!(value(&inspection, "gate"), Some("plan not approved"));
    assert_eq!(inspection.right.as_deref(), Some("paused · 2h46m"));
}
