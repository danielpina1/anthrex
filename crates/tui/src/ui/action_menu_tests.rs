//! Milestone 9.0.6 task 10: the menu, its pages and the moved-base page, drawn.

use crate::actions_request::ActionTarget;
use crate::app::actions::{ActionStep, MovedBasePage};
use crate::app::{App, Modal};
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use crate::tree::run_fixtures::{RUN_ID, run, snapshot, task, three_task_fixture};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{
    ActionInfo, ActionKind, BaseMovedInfo, DaemonMsg, RunReply, RunState, Size, TaskState,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

fn action(kind: ActionKind, label: &str, effect: &str, refused: Option<&str>) -> ActionInfo {
    ActionInfo {
        needs: kind.needs(),
        destructive: kind.destructive(),
        effect: effect.into(),
        label: label.into(),
        refused_why: refused.map(str::to_owned),
        kind,
    }
}

const NOT_COMPLETE: &str = "run add-reset-3f9a is running; accept applies only to a complete run";

fn running_app() -> App {
    let (mut snap, windows) = three_task_fixture();
    snap.runs[0].actions = vec![
        action(
            ActionKind::Pause,
            "pause",
            "pause: no new task, gate or delivery starts; open turns finish",
            None,
        ),
        action(
            ActionKind::Cancel,
            "cancel run",
            "cancel: stop 1 worker and cancel 2 unmerged tasks; the run then completes",
            None,
        ),
        action(
            ActionKind::Accept,
            "accept",
            "accept: merge 1 task into main@",
            Some(NOT_COMPLETE),
        ),
    ];
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app
}

const DISCARD_EFFECT: &str = "discard: remove the run's worktrees and anthrex/add-mul-0723/* \
     branches (uncommitted work is kept under refs/anthrex/salvage/add-mul-0723/); main is \
     unchanged";

fn complete_app() -> App {
    let mut info = run("add-mul-0723", "/tmp/r", RunState::Complete);
    info.goal = "Add mul()".into();
    info.base_sha = "b0".repeat(20);
    info.run_branch = "anthrex/add-mul-0723/integration".into();
    info.run_head = "d1".repeat(20);
    let mut t1 = task("t1", "mul", Size::S, TaskState::Merged);
    t1.start_commit = Some("b0".repeat(20));
    info.tasks = vec![t1];
    info.actions = vec![
        action(
            ActionKind::Accept,
            "accept",
            "accept: merge 1 task into main@b0b0b0b",
            None,
        ),
        action(ActionKind::Discard, "discard", DISCARD_EFFECT, None),
    ];
    let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot(
        10_000,
        vec![info],
    ))));
    app
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn draw(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The dialog's rows (its frame included), trailing spaces trimmed: the 64 columns
/// centred in `width`.
fn dialog_rows(buffer: &Buffer, width: u16, height: u16) -> Vec<String> {
    let w = width.min(64);
    let x0 = (width - w) / 2;
    (0..height)
        .map(|y| {
            (x0..x0 + w)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .filter(|row| row.starts_with('┌') || row.starts_with('│') || row.starts_with('└'))
        .map(|row| row.trim_end().to_string())
        .collect()
}

/// The dialog's interior lines, borders and padding stripped.
fn inner(rows: &[String]) -> Vec<String> {
    rows.iter()
        .filter(|r| r.starts_with('│'))
        .map(|r| {
            r.trim_start_matches('│')
                .trim_end_matches('│')
                .strip_prefix(' ')
                .unwrap_or("")
                .trim_end()
                .to_string()
        })
        .collect()
}

fn open_menu(app: &mut App) {
    app.open_actions((RUN_ID.into(), ActionTarget::Run), None);
}

fn moved_base_app(commits: usize, total: u32, typed: &str, wrong: bool) -> App {
    let mut app = complete_app();
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Accept),
    );
    let Some(Modal::Action(flow)) = app.modal.as_mut() else {
        panic!("no menu");
    };
    flow.step = ActionStep::MovedBase(MovedBasePage {
        info: flow.items[0].clone(),
        moved: BaseMovedInfo {
            from: "b".repeat(40),
            to: "a".repeat(40),
            commits: (0..commits)
                .map(|i| format!("a{i:06} someone: fix the readme"))
                .collect(),
            total,
        },
        typed: typed.into(),
        wrong,
    });
    app
}

#[test]
fn menu_renders_entries_and_muted_refusals() {
    let mut app = running_app();
    open_menu(&mut app);
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = draw(&app, w, h);
        let rows = dialog_rows(&buffer, w, h);
        assert_eq!(
            rows[0], "┌ Add password reset · 3f9a ───────────────────────────────────┐",
            "{w}x{h}"
        );
        assert_eq!(
            inner(&rows),
            vec![
                "▌ pause",
                "  cancel run",
                "  accept  run add-reset-3f9a is running; accept applies onl…",
                "  stats",
                "",
                "⏎ choose · j/k move · esc close",
            ],
            "{w}x{h}"
        );
        // The refused entry is muted from its mark to its reason; `cancel run` is
        // destructive and drawn in `Failed`.
        let x0 = (w - 64) / 2;
        let y0 = (0..h).find(|&y| buffer[(x0, y)].symbol() == "┌").unwrap();
        let muted = role(Role::Muted, app.palette()).fg;
        let failed = role(Role::Failed, app.palette()).fg;
        assert_eq!(buffer[(x0 + 4, y0 + 3)].fg, muted.unwrap(), "{w}x{h}");
        assert_eq!(buffer[(x0 + 12, y0 + 3)].fg, muted.unwrap(), "{w}x{h}");
        assert_eq!(buffer[(x0 + 4, y0 + 2)].fg, failed.unwrap(), "{w}x{h}");
        assert_ne!(buffer[(x0 + 4, y0 + 1)].fg, muted.unwrap(), "{w}x{h}");
    }
    // The ` MENU ` badge while it is open.
    let buffer = draw(&app, 80, 24);
    let bar: String = (0..80).map(|x| buffer[(x, 23)].symbol()).collect();
    assert!(bar.starts_with(" MENU "), "{bar:?}");
}

#[test]
fn a_disconnected_menu_says_so() {
    let mut app = running_app();
    app.on_link_lost("gone");
    open_menu(&mut app);
    let rows = dialog_rows(&draw(&app, 80, 24), 80, 24);
    assert_eq!(inner(&rows), vec!["not connected", "", "esc close"]);
}

#[test]
fn an_empty_menu_says_so() {
    let (snap, windows) = crate::tree::stage_fixtures::staged_fixture();
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app.open_actions((RUN_ID.into(), ActionTarget::Stage(2)), None);
    let rows = dialog_rows(&draw(&app, 80, 24), 80, 24);
    assert_eq!(
        rows[0],
        "┌ Add password reset · 3f9a › stage 2 ─────────────────────────┐"
    );
    assert_eq!(
        inner(&rows),
        vec!["no actions", "", "⏎ choose · j/k move · esc close"]
    );
}

#[test]
fn confirm_page_renders_destructive_in_failed() {
    let mut app = complete_app();
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Discard),
    );
    press(&mut app, KeyCode::Enter);
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = draw(&app, w, h);
        let rows = dialog_rows(&buffer, w, h);
        assert_eq!(
            rows[0],
            "┌ discard ─────────────────────────────────────────────────────┐"
        );
        assert_eq!(
            inner(&rows),
            vec![
                "discard: remove the run's worktrees and",
                "anthrex/add-mul-0723/* branches (uncommitted work is kept",
                "under refs/anthrex/salvage/add-mul-0723/); main is unchanged",
                "",
                "removes  the run's remaining worktrees and its",
                "         anthrex/add-mul-0723/* branches",
                "keeps    main unchanged · uncommitted work under",
                "         refs/anthrex/salvage/add-mul-0723/",
                "",
                "y discard · esc back",
            ],
            "{w}x{h}"
        );
        let x0 = (w - 64) / 2;
        let y0 = (0..h).find(|&y| buffer[(x0, y)].symbol() == "┌").unwrap();
        let failed = role(Role::Failed, app.palette()).fg.unwrap();
        // The title, the `y` and the verb draw in `Failed`.
        assert_eq!(buffer[(x0 + 2, y0)].fg, failed);
        let hints = y0 + 10;
        assert_eq!(buffer[(x0 + 2, hints)].symbol(), "y");
        assert_eq!(buffer[(x0 + 2, hints)].fg, failed);
        assert_eq!(buffer[(x0 + 4, hints)].fg, failed);
    }

    // A refusal the snapshot brought shows in `Failed`.
    let mut app = complete_app();
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Accept),
    );
    press(&mut app, KeyCode::Enter);
    let Some(Modal::Action(flow)) = app.modal.as_mut() else {
        panic!("no menu");
    };
    let ActionStep::Confirm(page) = &mut flow.step else {
        panic!("no page");
    };
    page.info.refused_why = Some("run add-mul-0723 is accepted".into());
    let rows = inner(&dialog_rows(&draw(&app, 80, 24), 80, 24));
    assert_eq!(
        rows,
        vec![
            "accept: merge 1 task into main@b0b0b0b",
            "",
            "tasks   1/1 merged",
            "base    main@b0b0b0b",
            "merges  anthrex/add-mul-0723/integration@d1d1d1d",
            "",
            "run add-mul-0723 is accepted",
            "",
            "y accept · esc back",
        ]
    );
}

#[test]
fn moved_base_page_renders() {
    let app = moved_base_app(1, 1, "072", true);
    for (w, h) in [(80, 24), (120, 40)] {
        let rows = dialog_rows(&draw(&app, w, h), w, h);
        assert_eq!(
            rows[0],
            "┌ accept ──────────────────────────────────────────────────────┐"
        );
        assert_eq!(
            inner(&rows),
            vec![
                "main moved bbbbbbb → aaaaaaa (1 commit)",
                "  a000000 someone: fix the readme",
                "",
                "type 0723 to merge onto aaaaaaa",
                "› 072█",
                "type 0723 exactly",
                "",
                "⏎ accept · esc back",
            ],
            "{w}x{h}"
        );
    }
    // Fifty commits of a hundred: as many as fit, then the rest counted.
    let app = moved_base_app(50, 100, "", false);
    let rows = inner(&dialog_rows(&draw(&app, 80, 24), 80, 24));
    assert_eq!(rows.len(), 22, "{rows:#?}");
    assert_eq!(rows[0], "main moved bbbbbbb → aaaaaaa (100 commits)");
    assert_eq!(rows[1], "  a000000 someone: fix the readme");
    assert_eq!(rows[16], "  … and 85 more");
    assert_eq!(rows[18], "type 0723 to merge onto aaaaaaa");
    let rows = inner(&dialog_rows(&draw(&app, 120, 40), 120, 40));
    assert_eq!(rows[rows.len() - 6], "  … and 69 more");

    // ASCII: every glyph has its twin.
    let mut app = moved_base_app(1, 3, "07", false);
    app.settings.badges.ascii = true;
    let buffer = draw(&app, 80, 24);
    let text: Vec<String> = (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let text = text.join("\n");
    assert!(
        text.contains("main moved bbbbbbb -> aaaaaaa (3 commits)"),
        "{text}"
    );
    assert!(text.contains("  ... and 2 more"), "{text}");
    // Final fix wave M5: the rename box's cursor, a reversed cell in ASCII (was `_`).
    let (y, line) = text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("> 07 "))
        .unwrap_or_else(|| panic!("{text}"));
    let x = u16::try_from(line.find("> 07 ").unwrap() + 4).unwrap();
    let cursor = &buffer[(x, u16::try_from(y).unwrap())];
    assert!(cursor.modifier.contains(ratatui::style::Modifier::REVERSED));
    assert!(text.contains("enter accept - esc back"), "{text}");
    let dialog: Vec<String> = (0..24)
        .map(|y| (8..72).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .filter(|row| row.starts_with('+') || row.starts_with('|'))
        .collect();
    assert_eq!(dialog.len(), 10, "{text}");
    assert!(dialog.iter().all(|row| row.is_ascii()), "{dialog:#?}");
}

/// Decision 13 on the moved-base page: a snapshot that refuses the accept shows the
/// reason in `Failed`.
#[test]
fn moved_base_page_shows_a_new_refusal() {
    let mut app = moved_base_app(1, 1, "", false);
    let Some(Modal::Action(flow)) = app.modal.as_mut() else {
        panic!("no menu");
    };
    let ActionStep::MovedBase(page) = &mut flow.step else {
        panic!("no page");
    };
    page.info.refused_why = Some("run add-mul-0723 is accepted".into());
    let buffer = draw(&app, 80, 24);
    let rows = inner(&dialog_rows(&buffer, 80, 24));
    assert_eq!(rows[4], "› _".replace('_', "█"), "{rows:#?}");
    assert_eq!(rows[5], "run add-mul-0723 is accepted", "{rows:#?}");
    let failed = role(Role::Failed, app.palette()).fg.unwrap();
    let y = (0..24)
        .find(|&y| {
            (8..72)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("is accepted")
        })
        .unwrap();
    assert_eq!(buffer[(10, y)].fg, failed);
}

/// Preflight F23: the menu's frame is drawn in `Accent` (the menu-over-Settings case is
/// task 14's).
#[test]
fn one_accented_border_while_the_menu_is_open() {
    let mut app = running_app();
    open_menu(&mut app);
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = draw(&app, w, h);
        let accent = role(Role::Accent, app.palette()).fg.unwrap();
        let x0 = (w - 64) / 2;
        let y0 = (0..h).find(|&y| buffer[(x0, y)].symbol() == "┌").unwrap();
        let y1 = (y0..h).find(|&y| buffer[(x0, y)].symbol() == "└").unwrap();
        for x in x0..x0 + 64 {
            for y in [y0, y1] {
                let cell = &buffer[(x, y)];
                if cell.symbol() == "─" || "┌┐└┘".contains(cell.symbol()) {
                    assert_eq!(cell.fg, accent, "{w}x{h} ({x},{y})");
                }
            }
        }
        for y in y0..=y1 {
            assert_eq!(buffer[(x0, y)].fg, accent, "{w}x{h} left {y}");
            assert_eq!(buffer[(x0 + 63, y)].fg, accent, "{w}x{h} right {y}");
        }
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    let mut menu = running_app();
    open_menu(&mut menu);
    let mut page = complete_app();
    page.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Discard),
    );
    press(&mut page, KeyCode::Enter);
    let moved = moved_base_app(50, 100, "0723", true);
    for app in [&menu, &page, &moved] {
        for (w, h) in [(20, 5), (1, 1), (64, 3), (5, 40)] {
            draw(app, w, h);
        }
    }
}
