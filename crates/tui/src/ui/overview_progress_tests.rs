//! Milestone 9.19 task 7 (decisions 5 and 6): one progress count on every surface, and
//! a design run's title never reads `0/0`.

use crate::app::App;
use crate::inspector::inspect;
use crate::settings::UiSettings;
use crate::tree::run_fixtures::{PROJECT, RUN_ID, run, snapshot, task};
use crate::tree::{NodeKey, RunFilter, run_rows};
use proto::{
    DaemonMsg, DesignMode, DocGateInfo, DocGateKind, RevisingCause, RunInfo, RunReply, RunState,
    Size, TaskState,
};

fn app_with(run: RunInfo) -> App {
    let mut app = App::new(Vec::new(), "/tmp".into(), UiSettings::default());
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot(600, vec![run]))));
    app
}

/// A merged task, a cancelled one, a replaced one (cancelled, replaced by the merged
/// task) and a reported one: one task can still merge, and it has.
fn mixed_run() -> RunInfo {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut replaced = task("t3", "old", Size::S, TaskState::Cancelled);
    replaced.replaced_by = vec!["t1".into()];
    info.tasks = vec![
        task("t1", "new", Size::S, TaskState::Merged),
        task("t2", "dropped", Size::S, TaskState::Cancelled),
        replaced,
        task("t4", "research", Size::S, TaskState::Reported),
    ];
    info
}

fn inspected_progress(app: &App) -> String {
    let rows = run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    let row = rows
        .iter()
        .find(|row| row.key == NodeKey::Run(RUN_ID.into()))
        .expect("the run row");
    let inspection = inspect(row, app);
    let field = inspection.fields.iter().find(|f| f.label == "progress");
    field.expect("a progress field").value.clone()
}

#[test]
fn progress_agrees_on_every_surface() {
    let app = app_with(mixed_run());
    let (_, right) = super::run_title(&app, RUN_ID, 120);
    assert_eq!(right.as_deref(), Some("1/1 merged · 10m"));
    let inspected = inspected_progress(&app);
    assert!(inspected.contains("  1/1 merged"), "{inspected}");
    let run = &app.runs.runs[0];
    // Decision 5's narrow form, `+` in ASCII (ruling R15).
    assert_eq!(
        crate::graph::run_text::run_text_in(run, false, false),
        "run 3f9a  1/1 ✓"
    );
    assert_eq!(
        crate::graph::run_text::run_text_in(run, true, true),
        "orchestrator  1/1 +"
    );
}

fn gate(kind: DocGateKind, version: u32) -> DocGateInfo {
    DocGateInfo {
        kind,
        version,
        revising: None,
        disputed: Vec::new(),
        not_reviewed: None,
        changes_summary: Vec::new(),
        same_runtime: false,
        report: None,
        revising_cause: RevisingCause::Changes,
    }
}

#[test]
fn a_designing_run_never_reads_zero_of_zero() {
    let designing = |state: RunState, doc_gate: Option<DocGateInfo>| {
        let mut info = run(RUN_ID, PROJECT, state);
        info.design = DesignMode::Full;
        info.doc_gate = doc_gate;
        app_with(info)
    };
    for (app, want) in [
        (
            designing(RunState::Brainstorming, None),
            "brainstorming · 10m",
        ),
        (designing(RunState::Specifying, None), "specifying · 10m"),
        (
            designing(RunState::AwaitingApproval, Some(gate(DocGateKind::Spec, 2))),
            "⏸ spec v2 · 10m",
        ),
    ] {
        let (_, right) = super::run_title(&app, RUN_ID, 120);
        assert_eq!(right.as_deref(), Some(want));
    }
}
