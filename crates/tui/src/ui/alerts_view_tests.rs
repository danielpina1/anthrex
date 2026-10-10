//! M9.0.7.6: the Alerts view in the main pane (decision 11), rendered with
//! `TestBackend` over `ui/alerts_fixture.rs`'s `three_runs`, the view open on `t2`'s
//! blocked alert.

use super::super::alerts::fixture::{NOW, app_of, named, three_runs, three_runs_snapshot};
use crate::app::App;
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::theme::{self, Role};
use crate::tree::alert_fixtures::{orch_window, with_orch};
use crate::tree::orch_fixtures::hold;
use crate::tree::run_fixtures::{pty, snapshot, task};
use crate::ui::{Layout, audit};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{HoldState, ProposalAlertInfo, RunState, Size, Status, TaskState};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

pub(super) fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let _ = app.on_key(KeyEvent::new(code, mods));
}

pub(super) fn chord(app: &mut App, c: char) {
    press(app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    press(app, KeyCode::Char(c), KeyModifiers::NONE);
}

/// `C-b a`, then `j`: the view on `t2`'s blocked alert (the second, after the gate).
fn view_on_blocked() -> App {
    let mut app = three_runs();
    chord(&mut app, 'a');
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert!(app.alerts_focus.is_some());
    app
}

pub(super) fn draw_at(app: &App, w: u16, h: u16) -> (Buffer, Layout) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut layout = None;
    terminal
        .draw(|f| layout = Some(crate::ui::draw(f, app)))
        .unwrap();
    (terminal.backend().buffer().clone(), layout.unwrap())
}

/// The main pane's interior rows, trailing spaces trimmed.
pub(super) fn rows_of(app: &App, w: u16, h: u16) -> Vec<String> {
    let (buffer, layout) = draw_at(app, w, h);
    let inner = layout.main_inner;
    (inner.y..inner.y + inner.height)
        .map(|y| {
            let row: String = (inner.x..inner.x + inner.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.trim_end().to_owned()
        })
        .collect()
}

/// The columns right of the `│` rule, trimmed.
fn right_pane(rows: &[String]) -> Vec<&str> {
    rows.iter()
        .filter_map(|row| row.split_once('│').map(|(_, right)| right.trim()))
        .collect()
}

/// The columns left of the `│` rule.
fn left_pane(rows: &[String]) -> Vec<&str> {
    rows.iter()
        .filter_map(|row| row.split_once('│').map(|(left, _)| left.trim_end()))
        .collect()
}

fn bar(buffer: &Buffer) -> String {
    let y = buffer.area.height - 1;
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn the_view_shows_the_whole_alert_at_120x40() {
    let app = view_on_blocked();
    let rows = rows_of(&app, 120, 40); // the main pane's interior, trimmed
    let detail: Vec<&str> = right_pane(&rows); // the columns right of the `│` rule
    assert_eq!(detail[0], "t2  report_product in c");
    for want in [
        "phase   blocked · question",
        "asked   which crate owns the formatting helper?",
        "run     Add mul() · 0723",
        "worker  cx gpt-6-sol · 12 calls · 4m",
        "age     41s",
    ] {
        assert!(detail.contains(&want), "{want:?} not in {detail:#?}");
    }
    assert!(
        detail
            .iter()
            .any(|l| l.starts_with("actions  answer · message")),
        "{detail:#?}"
    );
    // Fix round 1 ruling: the row wraps under its label, so every entry shows.
    assert!(
        detail.contains(&"cancel task · open conversation"),
        "{detail:#?}"
    );
    assert!(
        detail
            .iter()
            .any(|l| l.contains("⏎ answer") && l.contains("o open task")),
        "{detail:#?}"
    );
    // The rows in order, then a blank, the actions and the hints; the title and the
    // position in the top border.
    assert_eq!(
        &detail[..10],
        [
            "t2  report_product in c",
            "phase   blocked · question",
            "asked   which crate owns the formatting helper?",
            "run     Add mul() · 0723",
            "worker  cx gpt-6-sol · 12 calls · 4m",
            "age     41s",
            "",
            "actions  answer · message · retry · override ·",
            "cancel task · open conversation",
            "⏎ answer  . all actions  m message  o open task",
        ]
    );
    let (buffer, layout) = draw_at(&app, 120, 40);
    let top: String = (layout.main.x..layout.main.right())
        .map(|x| buffer[(x, layout.main.y)].symbol())
        .collect();
    assert!(top.starts_with("╭ ⚑ Alerts ─"), "{top:?}");
    assert!(top.ends_with("─ 2/3 ╮"), "{top:?}");
    // The list is 33 columns, `clamp(84 × 2/5, 26, 44)`; the header, bold, starts two
    // columns right of the rule.
    let (x, y) = (layout.main_inner.x + 33, layout.main_inner.y);
    assert_eq!(buffer[(x, y)].symbol(), "│");
    assert_eq!(buffer[(x + 2, y)].symbol(), "t");
    assert!(buffer[(x + 2, y)].modifier.contains(Modifier::BOLD));
}

/// Decision 11: at an interior under 60 columns the list sits on top, a `─` rule below
/// it, the detail under that.
#[test]
fn the_view_stacks_below_60_columns() {
    let app = view_on_blocked();
    let (buffer, layout) = draw_at(&app, 80, 24);
    assert_eq!(layout.main_inner.width, 44);
    let rows = rows_of(&app, 80, 24);
    let selected = format!("▌P3 ⚑ Add mul() · 0723 › t2{}41s", " ".repeat(13));
    let rule = "─".repeat(44);
    assert_eq!(
        rows[..17],
        [
            " P2 ⚑ Docs · 77aa",
            selected.as_str(),
            " P4 ✓ Fix CI · 9b1e",
            rule.as_str(),
            " t2  report_product in c",
            " phase   blocked · question",
            " asked   which crate owns the formatting",
            "         helper?",
            " run     Add mul() · 0723",
            " worker  cx gpt-6-sol · 12 calls · 4m",
            " age     41s",
            "",
            " actions  answer · message · retry ·",
            "          override · cancel task · open",
            "          conversation",
            " ⏎ answer  . all actions  m message",
            "",
        ]
    );
    let status = bar(&buffer);
    assert!(status.starts_with(" ALERTS "), "{status:?}");
    assert!(status.ends_with("esc back"), "{status:?}");
}

#[test]
fn list_rows_mark_priority_and_selection() {
    let app = view_on_blocked();
    let rows = rows_of(&app, 120, 40);
    let list = left_pane(&rows);
    assert_eq!(
        list[..4],
        [
            " P2 ⚑ Docs · 77aa",
            "▌P3 ⚑ Add mul() · 0723 › t2  41s",
            " P4 ✓ Fix CI · 9b1e",
            "",
        ]
    );
    // Moved from task 5's box test: the selected (second) row is reversed across the
    // list, the `▌` bar in the accent left of it; the first row is not reversed.
    let (buffer, layout) = draw_at(&app, 120, 40);
    let inner = layout.main_inner;
    let reversed = |x: u16, y: u16| buffer[(x, y)].modifier.contains(Modifier::REVERSED);
    let accent = theme::role(Role::Accent, app.palette()).fg;
    assert_eq!(Some(buffer[(inner.x, inner.y + 1)].fg), accent);
    assert!(!reversed(inner.x, inner.y + 1), "the bar is not reversed");
    for x in inner.x + 1..inner.x + 33 {
        assert!(reversed(x, inner.y + 1), "selected cell ({x})");
        assert!(!reversed(x, inner.y), "first row cell ({x})");
    }
    // The priority's style: P2 and P3 Attention without bold, P4 Done.
    let attention = theme::role(Role::Attention, app.palette()).fg;
    let done = theme::role(Role::Done, app.palette()).fg;
    assert_eq!(Some(buffer[(inner.x + 4, inner.y)].fg), attention);
    assert_eq!(Some(buffer[(inner.x + 4, inner.y + 2)].fg), done);
    // In ASCII: the twins.
    let mut app = view_on_blocked();
    app.settings.badges.ascii = true;
    let rows = rows_of(&app, 120, 40);
    let first = format!("{:<33}| t2  report_product in c", " P2 ! Docs - 77aa");
    assert_eq!(rows[0], first);
    assert!(
        rows[1].starts_with(">P3 ! Add mul() - 0723 > t2  41s |"),
        "{:?}",
        rows[1]
    );
    let (buffer, _) = draw_at(&app, 120, 40);
    assert_eq!(audit::first_non_ascii(&buffer), None);
    let (buffer, _) = draw_at(&app, 80, 24);
    assert_eq!(audit::first_non_ascii(&buffer), None);
}

#[test]
fn the_status_bar_reads_alerts_and_its_keys() {
    let app = view_on_blocked();
    let (buffer, _) = draw_at(&app, 120, 40);
    assert_eq!(
        bar(&buffer),
        " ALERTS  j/k move  ⏎ answer  . actions  o open  esc back"
    );
    let (buffer, _) = draw_at(&app, 40, 24);
    let status = bar(&buffer);
    assert!(status.starts_with(" ALERTS "), "{status:?}");
    assert!(status.ends_with("esc back"), "{status:?}");
    // On the gate: its preselection's label.
    let mut app = three_runs();
    chord(&mut app, 'a');
    let (buffer, _) = draw_at(&app, 120, 40);
    assert_eq!(
        bar(&buffer),
        " ALERTS  j/k move  ⏎ review plan  . actions  o open  esc back"
    );
}

/// Decision 1: the view's frame is the one accented; the Alerts box stays muted
/// (decision 26: the box is never focused).
#[test]
fn the_view_is_the_one_accented_frame() {
    let app = view_on_blocked();
    let accent = theme::role(Role::Accent, app.palette()).fg;
    let muted = theme::role(Role::Muted, app.palette()).fg;
    for (w, h) in [(80, 24), (120, 40)] {
        let (buffer, layout) = draw_at(&app, w, h);
        assert_eq!(audit::accented_frames(&buffer, app.palette()), 1, "{w}x{h}");
        assert_eq!(Some(buffer[(layout.main.x, layout.main.y)].fg), accent);
        assert_eq!(Some(buffer[(layout.alerts.x, layout.alerts.y)].fg), muted);
        let alerts = layout.alerts_inner;
        for y in alerts.y..alerts.bottom() {
            for x in alerts.x..alerts.right() {
                assert!(!buffer[(x, y)].modifier.contains(Modifier::REVERSED));
            }
        }
    }
    // Under the help the view mutes: the audit's `help over the alerts view` fixture
    // and `every_accented_box_glyph_is_one_frames` (final fix wave I2) see it.
}

/// Review focus 5: the goal, the task's title and id, the question, the worker's
/// model and the actions' labels are agent or daemon text.
#[test]
fn alerts_view_text_is_sanitised() {
    let bad = hostile_text();
    let mut runs = three_runs_snapshot();
    runs[0].goal = bad.clone();
    runs[0].base_branch = format!("main{bad}");
    runs[0].base_sha = format!("{bad}abcdef");
    runs[1].goal = format!("Add{bad}");
    let t2 = &mut runs[1].tasks[1];
    t2.title = format!("title{bad}");
    t2.block.as_mut().unwrap().text = format!("q{bad}\n{bad}\nmore{bad}");
    t2.rounds[0].route.model = format!("m{bad}");
    for action in &mut t2.actions {
        action.label = format!("{}{bad}", action.label);
    }
    t2.actions[2].refused_why = Some(bad.clone());
    let mut halted = crate::tree::alert_fixtures::at("halt-1", RunState::Halted, 9);
    halted.halted_reason = Some(format!("why{bad}\n{bad}"));
    runs.push(halted);
    // Fix round 1: an orchestrator alert (its model) and a hold alert (its id).
    let mut held = with_orch(named("orch-1", &bad, RunState::Running, 10), 7);
    held.orchestrator.as_mut().unwrap().route.model = format!("m{bad}");
    held.tasks = vec![task("t1", &bad, Size::S, TaskState::Pending)];
    held.holds = vec![hold(&format!("epic:{bad}"), HoldState::Awaiting, &["t1"])];
    runs.push(held);
    let windows = vec![
        pty(1, "shell", "/tmp/repo", Status::Idle),
        orch_window(7, "orch-1", Status::Attention, true),
    ];
    let mut app = app_of(windows, runs);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot({
        let mut snap = snapshot(NOW, app.runs.runs.clone());
        snap.proposals = vec![ProposalAlertInfo {
            project: format!("/tmp/p{bad}").into(),
            updated_at: NOW - 5,
        }];
        snap
    })));
    chord(&mut app, 'a');
    let n = crate::app::alerts(&app).len();
    assert_eq!(n, 7);
    for at in 0..n {
        for (w, h) in [(80, 24), (120, 40)] {
            let (buffer, layout) = draw_at(&app, w, h);
            let main = layout.main;
            for y in main.y..main.bottom() {
                let text: String = (main.x..main.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                assert_eq!(first_hostile(&text), None, "alert {at} {w}x{h}: {text:?}");
            }
            assert_eq!(first_hostile(&bar(&buffer)), None, "alert {at} {w}x{h}");
        }
        let alert = crate::app::alerts(&app)[at].clone();
        for width in [1, 10, 49, 400] {
            for line in super::detail_lines(&app, &alert, width) {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(first_hostile(&text), None, "{text:?}");
            }
        }
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    }
}

#[test]
fn no_alerts_view_says_so() {
    let mut app = app_of(vec![pty(1, "shell", "/tmp/repo", Status::Idle)], vec![]);
    chord(&mut app, 'a');
    assert!(app.alerts_focus.is_some());
    for (w, h) in [(80, 24), (120, 40)] {
        let rows = rows_of(&app, w, h);
        assert_eq!(rows[0], "no alerts", "{w}x{h}");
        assert!(rows[1..].iter().all(String::is_empty), "{rows:#?}");
        let (buffer, layout) = draw_at(&app, w, h);
        let top: String = (layout.main.x..layout.main.right())
            .map(|x| buffer[(x, layout.main.y)].symbol())
            .collect();
        assert!(top.starts_with("╭ Alerts ──"), "{top:?}");
        assert!(top.ends_with("──╮"), "{top:?}");
        assert_eq!(bar(&buffer), " ALERTS  esc back");
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    for ascii in [false, true] {
        for sidebar in [true, false] {
            for (w, h) in [(1, 1), (2, 2), (20, 5), (5, 40), (40, 3), (12, 4), (70, 6)] {
                let mut app = view_on_blocked();
                app.settings.badges.ascii = ascii;
                app.sidebar_visible = sidebar;
                draw_at(&app, w, h);
                press(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
                draw_at(&app, w, h);
            }
        }
    }
}

/// Fix round 1 (Review focus 4): a hold's `plan` counts the stages of its own tasks,
/// never the run's; the gate's counts the plan's.
#[test]
fn a_holds_plan_counts_its_own_stages() {
    let staged = |id: &str, stage: u16| {
        let mut t = task(id, "work", Size::S, TaskState::Pending);
        t.stage = stage;
        t
    };
    let mut held = named("held-5a5a", "Held", RunState::Running, 4);
    held.tasks = vec![staged("t1", 1), staged("t2", 1), staged("t3", 2)];
    held.holds = vec![hold("epic:ui", HoldState::Awaiting, &["t1", "t2"])];
    let mut gate = named("gate-6b6b", "Gate", RunState::AwaitingApproval, 5);
    gate.tasks = vec![staged("t1", 1), staged("t2", 2)];
    let mut app = app_of(
        vec![pty(1, "shell", "/tmp/repo", Status::Idle)],
        vec![held, gate],
    );
    chord(&mut app, 'a');
    let detail = |app: &App| right_pane(&rows_of(app, 120, 40)).join("\n");
    // The hold's run is the older: its alert comes first.
    let hold_rows = detail(&app);
    assert!(
        hold_rows.contains("phase  hold epic:ui awaiting approval"),
        "{hold_rows}"
    );
    assert!(hold_rows.contains("plan   2 tasks\n"), "{hold_rows}");
    assert!(!hold_rows.contains("stages"), "{hold_rows}");
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    let gate_rows = detail(&app);
    assert!(
        gate_rows.contains("plan   2 tasks · 2 stages"),
        "{gate_rows}"
    );
}

/// Decision 11: a refused entry is listed, muted; the others are not.
#[test]
fn a_refused_action_is_muted() {
    let mut runs = three_runs_snapshot();
    runs[1].tasks[1].actions[2].refused_why = Some("not now".into());
    let mut app = app_of(vec![pty(1, "shell", "/tmp/repo", Status::Idle)], runs);
    chord(&mut app, 'a');
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    let (buffer, _) = draw_at(&app, 120, 40);
    let muted = theme::role(Role::Muted, app.palette()).fg;
    let fg = |text: &str| {
        let (x, y) = audit::find(&buffer, text)[0];
        Some(buffer[(x, y)].fg)
    };
    assert_eq!(fg("retry"), muted);
    assert_ne!(fg("answer ·"), muted);
    assert_ne!(fg("cancel task"), muted);
}

/// Milestone 9.10 decisions 33-34 (review focus 4): a queued goal's text, a set-up's
/// failure reason and a project's name are hostile; neither the box, the view, the
/// bar nor any alert's detail draws one of their characters.
#[test]
fn set_up_alert_text_is_sanitised() {
    use proto::{QueuedGoalInfo, SetupState};
    let bad = hostile_text();
    let goal = |id: &str, project: &str, setup: SetupState| QueuedGoalInfo {
        id: id.into(),
        project: project.into(),
        goal: format!("g{bad}\n{bad}next"),
        queued_at: NOW - 3,
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        setup,
    };
    let failed = format!("/tmp/f{bad}");
    let reading = format!("/tmp/s{bad}");
    let review = format!("/tmp/p{bad}");
    let reason = format!("why{bad}\n{bad}\nmore{bad}");
    let mut app = app_of(vec![pty(1, "shell", "/tmp/repo", Status::Idle)], vec![]);
    let mut snap = snapshot(NOW, vec![]);
    snap.queued_goals = vec![
        goal("q-1", &failed, SetupState::Failed { reason }),
        goal(
            "q-2",
            &failed,
            SetupState::Failed {
                reason: bad.clone(),
            },
        ),
        goal("q-3", &reading, SetupState::Reading),
        goal("q-4", &review, SetupState::NeedsReview),
    ];
    snap.proposals = vec![ProposalAlertInfo {
        project: review.clone().into(),
        updated_at: NOW - 5,
    }];
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    chord(&mut app, 'a');
    let n = crate::app::alerts(&app).len();
    assert_eq!(n, 3);
    for at in 0..n {
        for (w, h) in [(80, 24), (120, 40)] {
            let (buffer, _) = draw_at(&app, w, h);
            for y in 0..h {
                let text: String = (0..w).map(|x| buffer[(x, y)].symbol()).collect();
                assert_eq!(first_hostile(&text), None, "alert {at} {w}x{h}: {text:?}");
            }
        }
        let alert = crate::app::alerts(&app)[at].clone();
        for width in [1, 10, 49, 400] {
            let lines = super::detail_lines(&app, &alert, width);
            for line in &lines {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(first_hostile(&text), None, "{text:?}");
            }
            let all: String = (lines.iter().flat_map(|l| l.spans.iter()))
                .map(|s| s.content.as_ref())
                .collect();
            if width == 400 {
                assert!(all.contains("waiting"), "alert {at}: {all:?}");
                // The failed set-up (listed first, P3) shows its whole reason.
                assert_eq!(at == 0, all.contains("more"), "alert {at}: {all:?}");
            }
        }
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    }
}
