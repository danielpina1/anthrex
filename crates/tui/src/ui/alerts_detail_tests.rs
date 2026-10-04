//! Whole-branch review D, I-1: an alert on a racing or paired task names who writes it.

use super::worker_text;
use crate::app::App;
use crate::tree::run_fixtures::{pair_writing_fixture, racers_live_fixture};
use proto::{DaemonMsg, RunReply, RunsSnapshot, WindowInfo};

fn app_of((snapshot, windows): (RunsSnapshot, Vec<WindowInfo>)) -> App {
    let mut app = App::new(
        windows,
        "/tmp".into(),
        crate::settings::UiSettings::default(),
    );
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot)));
    app
}

#[test]
fn a_racing_or_paired_task_has_a_worker_row() {
    let app = app_of(racers_live_fixture());
    let text = worker_text(&app, &app.runs.runs[0].tasks[0]).expect("a racer");
    assert!(text.contains("calls"), "{text}");
    let app = app_of(pair_writing_fixture());
    let text = worker_text(&app, &app.runs.runs[0].tasks[0]).expect("the test writer");
    assert!(text.contains("calls"), "{text}");
}
