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

/// Milestone 9.9.8 review minor 8 and fix 2: the ask drawn through the Alerts view puts
/// nothing hostile on any row, whether the hostile text is in the options (which must
/// still be drawn, cleaned) or in the context (which can fill the pane, so only the
/// absence is asserted there).
#[test]
fn a_hostile_ask_drawn_in_the_alerts_view_has_no_hostile_character() {
    use crate::safe_text::tests::{first_hostile, hostile_text};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let bad = hostile_text();
    for hostile_options in [true, false] {
        let (context, option) = if hostile_options {
            ("the style guide".to_owned(), format!("o{bad}"))
        } else {
            (format!("the style{bad}guide"), "tabs".to_owned())
        };
        let mut app = app_with_orchestrator(|run| {
            run.orchestrator.as_mut().unwrap().ask = Some(proto::AskInfo {
                id: 1,
                question: format!("q{bad}"),
                options: vec![option],
                context,
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
            if hostile_options {
                assert!(
                    drawn.contains("the style guide"),
                    "{w}x{h} drew the context"
                );
                assert!(drawn.contains("1. o"), "{w}x{h} drew the option: {drawn}");
            }
        }
    }
}

/// Milestone 9.9.8 fix 2: the detail pane draws an ask's context and its numbered
/// options, wrapped, so the `1-9` keys have something to answer.
#[test]
fn an_asks_context_and_options_are_drawn_in_the_detail() {
    let app = app_with_orchestrator(|run| {
        run.orchestrator.as_mut().unwrap().ask = Some(proto::AskInfo {
            id: 1,
            question: "tabs or spaces?".into(),
            options: vec!["tabs".into(), "spaces".into()],
            context: "the style guide is silent".into(),
            asked_at: 9_970,
        });
    });
    let all = crate::app::alerts(&app);
    let rows = text_of(&super::detail_lines(&app, &all[0], 24));
    let text = rows.join("\n");
    assert!(text.contains("the style guide"), "{text}");
    assert!(rows.iter().any(|r| r.contains("1. tabs")), "{text}");
    assert!(rows.iter().any(|r| r.contains("2. spaces")), "{text}");
}

/// Milestone 9.19 decision 7, ruling R13: a delivery alert on stage 3 of a plan of
/// three reads `3 of 3`, both with the snapshot shaped as the daemon's `stage_infos`
/// builds it (one record per stage from 1 to the plan's highest, the last not created
/// yet) and with the brief's two records (review M1).
#[test]
fn the_stage_row_never_passes_its_denominator() {
    for records in [3, 2] {
        let text = stage_three_alert(records);
        assert!(text.contains("stage   3 of 3"), "{records} records: {text}");
    }
}

/// The detail of a delivery alert on stage 3 of a plan of three with `records` stage
/// records.
fn stage_three_alert(records: u16) -> String {
    use crate::tree::run_fixtures::{PROJECT, RUN_ID, run, snapshot, task};
    use proto::{DeliveryAlert, DeliveryAlertKind, DeliveryInfo, DeliveryMode, FullInfo};
    let mut info = run(RUN_ID, PROJECT, proto::RunState::Running);
    info.tasks = (1..=3u16)
        .map(|n| {
            let mut t = task(
                &format!("t{n}"),
                "a",
                proto::Size::S,
                proto::TaskState::Queued,
            );
            t.stage = n;
            t
        })
        .collect();
    info.stages = (1..=records)
        .map(|n| proto::StageInfo {
            actions: Vec::new(),
            n,
            branch: format!("anthrex/{RUN_ID}/stage-{n}"),
            head: (n < 3).then(|| "abc".to_owned()),
            tasks: 1,
            merged: 0,
            full: FullInfo::default(),
            fix_tasks: Vec::new(),
            propagate_red: None,
            pr: None,
            round: 1,
        })
        .collect();
    info.delivery = Some(DeliveryInfo {
        mode: DeliveryMode::Pr,
        remote: "origin".into(),
        repo: "o/r".into(),
        watching: true,
        delivering: true,
        poll_secs: 60,
        skipped_stages: Vec::new(),
        alerts: vec![DeliveryAlert {
            kind: DeliveryAlertKind::PrClosedUnmerged,
            stage: Some(3),
            text: "stage 3's PR closed".into(),
            user_only: true,
        }],
    });
    let app = app_of((snapshot(10_000, vec![info]), vec![]));
    detail_text(&app)
}

/// Milestone 9.19 decision 5 (review I2): the Accept detail's `merged` row is the run's
/// one progress count, a reported task in neither number.
#[test]
fn the_accept_detail_counts_as_the_run_does() {
    use crate::tree::run_fixtures::{PROJECT, RUN_ID, run, snapshot, task};
    use proto::{Size, TaskState};
    let mut info = run(RUN_ID, PROJECT, proto::RunState::Complete);
    info.tasks = vec![
        task("t1", "a", Size::S, TaskState::Merged),
        task("t2", "b", Size::S, TaskState::Reported),
        task("t3", "c", Size::S, TaskState::Cancelled),
    ];
    let app = app_of((snapshot(10_000, vec![info]), vec![]));
    let text = detail_text(&app);
    assert!(text.contains("merged  1/1"), "{text}");
}
