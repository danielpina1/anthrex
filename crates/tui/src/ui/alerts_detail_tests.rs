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

fn text_of(lines: &[ratatui::text::Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

fn app_with_orchestrator(configure: impl FnOnce(&mut proto::RunInfo)) -> App {
    use crate::tree::alert_fixtures::{at, with_orch};
    let mut run = with_orch(at("r", proto::RunState::Running, 1), 11);
    configure(&mut run);
    app_of((
        crate::tree::run_fixtures::snapshot(10_000, vec![run]),
        vec![],
    ))
}

fn detail_text(app: &App) -> String {
    let all = crate::app::alerts(app);
    assert_eq!(all.len(), 1, "{all:?}");
    text_of(&super::detail_lines(app, &all[0], 80)).join("\n")
}

/// Milestone 9.9.8: an ask's detail reads `asks you` as its phase and offers `1-9`; a
/// stuck orchestrator's reads `stuck` and offers no answer keys.
#[test]
fn an_ask_and_a_stuck_alert_name_their_phase_and_an_ask_offers_the_answer_keys() {
    let ask = app_with_orchestrator(|run| {
        run.orchestrator.as_mut().unwrap().ask = Some(proto::AskInfo {
            id: 1,
            question: "tabs or spaces?".into(),
            options: vec!["tabs".into()],
            context: String::new(),
            asked_at: 9_970,
        });
    });
    let text = detail_text(&ask);
    assert!(text.contains("asks you"), "{text}");
    assert!(text.contains("1-9") && text.contains("answer"), "{text}");

    let stuck = app_with_orchestrator(|run| {
        run.orchestrator.as_mut().unwrap().stuck =
            Some(proto::OrchestratorStuck::Dead { since: None });
    });
    let text = detail_text(&stuck);
    assert!(text.contains("stuck"), "{text}");
    assert!(!text.contains("1-9"), "{text}");
}

/// Milestone 9.9.8 review minor 8: the ask drawn through the Alerts view, its text and
/// its options hostile, puts nothing hostile on any row.
#[test]
fn a_hostile_ask_drawn_in_the_alerts_view_has_no_hostile_character() {
    use crate::safe_text::tests::{first_hostile, hostile_text};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let bad = hostile_text();
    let mut app = app_with_orchestrator(|run| {
        run.orchestrator.as_mut().unwrap().ask = Some(proto::AskInfo {
            id: 1,
            question: format!("q{bad}"),
            options: vec![format!("o{bad}")],
            context: format!("c{bad}"),
            asked_at: 9_970,
        });
    });
    for key in [
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    ] {
        let _ = app.on_key(key);
    }
    assert!(app.alerts_focus.is_some());
    for (w, h) in [(80, 24), (120, 40)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| {
                let _ = crate::ui::draw(f, &app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut drawn = String::new();
        for y in 0..h {
            let row: String = (0..w).map(|x| buffer[(x, y)].symbol()).collect();
            assert_eq!(first_hostile(&row), None, "{w}x{h} row {y}: {row:?}");
            drawn.push_str(&row);
        }
        assert!(drawn.contains("orchestrator asks"), "{w}x{h} drew the ask");
    }
}
