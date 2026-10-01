//! M9.0.7.5: the grown Alerts box (decisions 8–10), two lines an alert, rendered with
//! `TestBackend`, over `ui/alerts_fixture.rs`'s `three_runs`.

use super::fixture::{
    NOW, REPO, app_of, blocked_t2, event, named, three_runs, three_runs_snapshot,
};
use crate::app::{Alert, AlertKey, AlertWho, App, alerts};
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::settings::UiSettings;
use crate::theme::{self, Role};
use crate::tree::alert_fixtures::{at, blocked, orch_window, with_orch};
use crate::tree::orch_fixtures::hold;
use crate::tree::run_fixtures::{pty, snapshot};
use crate::ui::{Layout, SidebarSizing, alerts_box_rows, layout_for, layout_sized};
use proto::{BlockReason, HoldState, ProposalAlertInfo, RunState, Status, WindowInfo};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

fn draw_at(app: &App, w: u16, h: u16) -> (Buffer, Layout) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut layout = None;
    terminal
        .draw(|f| layout = Some(crate::ui::draw(f, app)))
        .unwrap();
    (terminal.backend().buffer().clone(), layout.unwrap())
}

fn text_in(buffer: &Buffer, rect: Rect, y: u16) -> String {
    (rect.x..rect.x + rect.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The box's interior rows, as drawn.
fn box_rows(buffer: &Buffer, layout: &Layout) -> Vec<String> {
    let inner = layout.alerts_inner;
    (inner.y..inner.y + inner.height)
        .map(|y| text_in(buffer, inner, y))
        .collect()
}

fn row(buffer: &Buffer, y: u16) -> String {
    text_in(buffer, buffer.area, y).trim_end().to_owned()
}

fn trimmed(rows: &[String]) -> Vec<&str> {
    rows.iter().map(|r| r.trim_end()).collect()
}

/// Decision 10 at a 34-column sidebar (32 interior columns), at both sizes.
#[test]
fn three_alerts_render_two_lines_each() {
    let app = three_runs();
    for (w, h) in [(80, 24), (120, 40)] {
        let (buffer, layout) = draw_at(&app, w, h);
        assert_eq!(layout.alerts_inner.height, 7, "{w}x{h}");
        assert_eq!(
            trimmed(&box_rows(&buffer, &layout)),
            vec![
                "⚑ Docs · 77aa",
                "  plan awaits approval · 2 tasks",
                "⚑ Add mul() · 0723 › t2      41s",
                "  blocked: which crate owns the",
                "  formatting helper?",
                "✓ Fix CI · 9b1e",
                "  ready to accept · 1/1 merged",
            ],
            "{w}x{h}"
        );
    }
}

#[test]
fn the_box_title_and_hint() {
    let app = three_runs();
    let (buffer, layout) = draw_at(&app, 120, 40);
    let top = row(&buffer, layout.alerts.y);
    let bottom = text_in(
        &buffer,
        layout.alerts,
        layout.alerts.y + layout.alerts.height - 1,
    );
    assert!(top.contains(" ⚑ Alerts 3 "), "{top}");
    // One `─` before the corner, as §6.1's mockup draws it.
    assert!(bottom.ends_with(" C-b a open ─╯"), "{bottom}");
    let p = app.palette();
    // The title in `Attention`; the key in the accent, the word muted.
    let flag = top.find('⚑').unwrap();
    let x = top[..flag].chars().count() as u16;
    assert_eq!(
        buffer[(x, layout.alerts.y)].fg,
        theme::fg(Role::Attention),
        "{top}"
    );
    let y = layout.alerts.y + layout.alerts.height - 1;
    let key = bottom[..bottom.find("C-b a").unwrap()].chars().count() as u16;
    let word = bottom[..bottom.find("open").unwrap()].chars().count() as u16;
    assert_eq!(Some(buffer[(key, y)].fg), theme::role(Role::Accent, p).fg);
    assert_eq!(Some(buffer[(word, y)].fg), theme::role(Role::Muted, p).fg);
    // The box is never accented (decision 10): its corner is muted.
    assert_eq!(
        Some(buffer[(layout.alerts.x, layout.alerts.y)].fg),
        theme::role(Role::Muted, p).fg
    );
}

/// Decision 9's formula: the interior is 1 with no alert, else
/// `max(3, min(C, H/2 − 2, H − R − 5))`, saturating, at most `H − 2`.
#[test]
fn box_rows_follow_decision_9() {
    #[rustfmt::skip]
    let table: [(u16, usize, usize, u16); 24] = [
        // H, R, C, interior
        (23, 4, 0, 1), (23, 20, 0, 1), (23, 40, 0, 1),
        (23, 4, 2, 3), (23, 20, 2, 3), (23, 40, 2, 3),
        (23, 4, 7, 7), (23, 20, 7, 3), (23, 40, 7, 3),
        (23, 4, 30, 9), (23, 20, 30, 3), (23, 40, 30, 3),
        (39, 4, 0, 1), (39, 20, 0, 1), (39, 40, 0, 1),
        (39, 4, 2, 3), (39, 20, 2, 3), (39, 40, 2, 3),
        (39, 4, 7, 7), (39, 20, 7, 7), (39, 40, 7, 3),
        (39, 4, 30, 17), (39, 20, 30, 14), (39, 40, 30, 3),
    ];
    for (h, tree_rows, alert_lines, want) in table {
        let sizing = SidebarSizing {
            tree_rows,
            alert_lines,
        };
        assert_eq!(
            alerts_box_rows(h, sizing),
            want,
            "H={h} R={tree_rows} C={alert_lines}"
        );
        // The layout draws exactly that box, with the agents block above it.
        let l = layout_sized(Rect::new(0, 0, 120, h + 1), 34, sizing);
        assert_eq!(
            l.alerts_inner.height, want,
            "H={h} R={tree_rows} C={alert_lines}"
        );
        assert_eq!(l.sidebar.height + l.alerts.height, h);
    }
    // Never more than the column less its borders, and no panic on a tiny column.
    for h in 0..6 {
        let sizing = SidebarSizing {
            tree_rows: 0,
            alert_lines: 9,
        };
        assert!(alerts_box_rows(h, sizing) <= h.saturating_sub(2), "{h}");
    }
}

#[test]
fn agents_get_the_rest() {
    let app = three_runs();
    for (w, h) in [(80, 24), (120, 40)] {
        let (_, layout) = draw_at(&app, w, h);
        let column = h - 1;
        assert_eq!(layout.sidebar.y + layout.sidebar.height, layout.alerts.y);
        assert_eq!(layout.sidebar.height + layout.alerts.height, column);
        assert_eq!(
            layout.sidebar_list.height,
            column - layout.alerts.height - 3,
            "{w}x{h}"
        );
    }
}

/// Review focus 3: the mouse reads the geometry the renderer drew with. The tree
/// overflows, so the list's last row is the row the box's height decides.
#[test]
fn a_click_hits_the_row_the_grown_box_left() {
    let shells: Vec<WindowInfo> = (1..=40)
        .map(|id| pty(id, &format!("shell-{id}"), REPO, Status::Idle))
        .collect();
    let mut app = app_of(shells, three_runs_snapshot());
    let area = Rect::new(0, 0, 120, 40);
    // As `lib.rs` draws a frame: the viewports from `layout_for`, then the draw.
    let next = layout_for(&app, area);
    app.set_tree_viewports(next.sidebar_list.height, next.main_inner.height);
    let (buffer, layout) = draw_at(&app, 120, 40);
    assert_eq!(layout, next);
    let list = layout.sidebar_list;
    assert!(
        app.rows().len() > usize::from(list.height),
        "the tree overflows"
    );
    // Decision 9: an overflowing tree leaves the box its floor, three rows; the list
    // is the column less the box and three rows.
    assert_eq!(layout.alerts_inner.height, 3);
    assert_eq!(list.height, 39 - 5 - 3);
    // The list's last row, just above the footer.
    let y = list.y + list.height - 1;
    assert_eq!(y + 1, layout.sidebar_footer.y);
    // A stale box (two lines for each of three alerts, or six) would put that row
    // outside the list.
    for n in [3, 6] {
        let stale = crate::ui::layout(area, 34, n).sidebar_list;
        assert!(!stale.contains((list.x + 4, y).into()), "{n}: {stale:?}");
    }
    let drawn = text_in(&buffer, list, y);
    let id: u32 = drawn
        .split("shell-")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("a shell is drawn on the last row: {drawn:?}"));
    assert_ne!(app.focused, Some(id));
    let _ = app.on_click(list.x + 4, y, &layout);
    assert_eq!(app.focused, Some(id), "{drawn:?}");
    // A click on the box's first row reaches no window.
    let before = (app.focused, app.tree.selected.clone(), app.tree.sidebar.top);
    assert!(
        app.on_click(list.x + 4, layout.alerts_inner.y, &layout)
            .is_empty()
    );
    let after = (app.focused, app.tree.selected.clone(), app.tree.sidebar.top);
    assert_eq!(before, after);
}

/// Decision 10: the `↓ <k> more` mark is the box's last row, with the rows a whole
/// alert would not fill left blank above it.
#[test]
fn the_mark_is_the_last_row() {
    let mut windows = vec![pty(1, "shell", REPO, Status::Idle)];
    windows.extend((2..=10).map(|id| pty(id, &format!("s{id}"), REPO, Status::Idle)));
    let app = app_of(windows, three_runs_snapshot());
    assert_eq!(app.rows().len(), 14);
    // 23 − 14 − 5 = 4 rows: the gate's two, a blank, the mark.
    let (buffer, layout) = draw_at(&app, 80, 24);
    assert_eq!(
        trimmed(&box_rows(&buffer, &layout)),
        [
            "⚑ Docs · 77aa",
            "  plan awaits approval · 2 tasks",
            "",
            "↓ 2 more"
        ]
    );
}

/// The glyph cell of each alert's first line, by priority.
#[test]
fn p2_and_p3_are_attention_without_bold_p1_bold_p4_done() {
    let (snap, windows) = crate::tree::alert_fixtures::every_source();
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    let all = alerts(&app);
    for priority in 1..=4 {
        let alert = all.iter().find(|a| a.priority == priority).unwrap();
        let lines = super::alert_lines(&app, alert, 32);
        let glyph = &lines[0].spans[0];
        let who = &lines[0].spans[1];
        let (role, bold) = match priority {
            1 => (Role::Attention, true),
            2 | 3 => (Role::Attention, false),
            _ => (Role::Done, false),
        };
        let mark = if priority == 4 { "✓ " } else { "⚑ " };
        assert_eq!(glyph.content, mark, "P{priority}");
        for span in [glyph, who] {
            assert_eq!(span.style.fg, Some(theme::fg(role)), "P{priority} {span:?}");
            assert_eq!(
                span.style.add_modifier.contains(Modifier::BOLD),
                bold,
                "P{priority} {span:?}"
            );
        }
        // The text in the default colour.
        let text = lines[1].spans.last().unwrap();
        assert_eq!(text.style, ratatui::style::Style::default(), "P{priority}");
    }
}

/// Twelve alerts in a 23-row column: whole alerts only, the last row `↓ <k> more`.
#[test]
fn overflow_shows_down_more() {
    let runs = (0..12)
        .map(|n| at(&format!("halt-{n:02}"), RunState::Halted, n))
        .collect();
    let app = app_of(vec![], runs);
    let (buffer, layout) = draw_at(&app, 80, 24);
    let rows = box_rows(&buffer, &layout);
    let rows = trimmed(&rows);
    let (last, shown) = rows.split_last().unwrap();
    // Each shown alert is whole: its who line, then its text.
    assert_eq!(shown.len() % 2, 0, "{rows:#?}");
    for pair in shown.chunks(2) {
        assert!(pair[0].starts_with("⚑ "), "{rows:#?}");
        assert_eq!(pair[1], "  run halted", "{rows:#?}");
    }
    assert!(!shown.is_empty(), "{rows:#?}");
    assert_eq!(
        *last,
        format!("↓ {} more", 12 - shown.len() / 2),
        "{rows:#?}"
    );
    let y = layout.alerts_inner.y + layout.alerts_inner.height - 1;
    assert_eq!(
        Some(buffer[(layout.alerts_inner.x, y)].fg),
        theme::role(Role::Muted, app.palette()).fg
    );
}

/// The first line of the box's alert for `key`.
fn first_line_of(app: &App, key: &AlertKey) -> String {
    let alert = alerts(app).into_iter().find(|a| a.key == *key).unwrap();
    let line = &super::alert_lines(app, &alert, 32)[0];
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Review focus 4: no age the snapshot does not prove.
#[test]
fn alerts_without_a_known_start_show_no_age() {
    let mut runs = three_runs_snapshot();
    // A halted run, and a hold awaiting approval on the running one.
    runs.push(named("halt-5e5e", "Halt", RunState::Halted, 4));
    runs[1].holds = vec![hold("h1", HoldState::Awaiting, &["t1"])];
    let mut app = app_of(vec![], runs);
    for (key, ends) in [
        (AlertKey::Gate("docs-77aa".into()), "Docs · 77aa"),
        (
            AlertKey::Hold {
                run: "add-mul-0723".into(),
                hold: "h1".into(),
            },
            "Add mul() · 0723",
        ),
        (AlertKey::Halted("halt-5e5e".into()), "Halt · 5e5e"),
        (AlertKey::Accept("fix-ci-9b1e".into()), "Fix CI · 9b1e"),
    ] {
        let line = first_line_of(&app, &key);
        assert!(line.trim_end().ends_with(ends), "{key:?}: {line:?}");
        let alert = alerts(&app).into_iter().find(|a| a.key == key).unwrap();
        assert_eq!(alert.age, None, "{key:?}");
    }
    // A blocked task whose history has no `blocked (` entry has no age either.
    let mut runs = three_runs_snapshot();
    runs[1].tasks[1].history = vec![event(NOW - 41, "worker round 2 started")];
    app = app_of(vec![], runs);
    let key = AlertKey::Blocked {
        run: "add-mul-0723".into(),
        task: "t2".into(),
    };
    let line = first_line_of(&app, &key);
    assert!(
        line.trim_end().ends_with("Add mul() · 0723 › t2"),
        "{line:?}"
    );
}

#[test]
fn ages_read_now_seconds_minutes_hours() {
    let key = AlertKey::Blocked {
        run: "add-mul-0723".into(),
        task: "t2".into(),
    };
    for (ago, age) in [
        (0, "now"),
        (5, "now"),
        (9, "now"),
        (10, "10s"),
        (41, "41s"),
        (150, "2m"),
        (7300, "2h"),
    ] {
        let mut runs = three_runs_snapshot();
        runs[1].tasks[1] = blocked_t2(ago);
        let app = app_of(vec![], runs);
        let line = first_line_of(&app, &key);
        assert!(line.ends_with(&format!(" {age}")), "{ago}: {line:?}");
        assert_eq!(unicode_width::UnicodeWidthStr::width(line.as_str()), 32);
    }
    // An orchestrator's alert is aged by its window's time in its status; a
    // proposal's by its `updated_at`.
    let mut window = orch_window(11, "orch-0b0b", Status::Attention, true);
    window.since_secs = 150;
    let orch = with_orch(named("orch-0b0b", "Orch", RunState::Running, 1), 11);
    let mut app = app_of(vec![window], vec![orch]);
    let line = first_line_of(&app, &AlertKey::Orchestrator("orch-0b0b".into()));
    assert!(line.ends_with(" 2m"), "{line:?}");
    let mut snap = snapshot(NOW, vec![]);
    snap.proposals = vec![ProposalAlertInfo {
        project: "/r/shop".into(),
        updated_at: NOW - 7300,
    }];
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    let line = first_line_of(&app, &AlertKey::Proposal("/r/shop".into()));
    assert!(line.starts_with("✓ shop "), "{line:?}");
    assert!(line.ends_with(" 2h"), "{line:?}");
}

#[test]
fn the_who_is_cut_first() {
    let mut runs = three_runs_snapshot();
    runs[1].goal = "x".repeat(60);
    let app = app_of(vec![], runs);
    let alert = alerts(&app).into_iter().find(|a| a.task.is_some()).unwrap();
    for width in [20, 24, 32] {
        let line: String = super::alert_lines(&app, &alert, width)[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(line.as_str()),
            usize::from(width),
            "{line:?}"
        );
        assert!(line.starts_with("⚑ x"), "{line:?}");
        assert!(line.contains("…"), "{line:?}");
        assert!(line.contains(" › t2 ") && line.ends_with("41s"), "{line:?}");
    }
    // In ASCII the same, with ASCII marks.
    let mut app = app;
    app.settings.badges.ascii = true;
    let line: String = super::alert_lines(&app, &alert, 24)[0]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(line.is_ascii(), "{line:?}");
    assert!(line.starts_with("! x") && line.contains("..."), "{line:?}");
    assert!(line.contains(" > t2 ") && line.ends_with("41s"), "{line:?}");
}

/// Review focus 5: a hostile goal, block text and task id never reach the screen.
#[test]
fn alerts_box_text_is_sanitised() {
    let bad = hostile_text();
    let mut runs = three_runs_snapshot();
    runs[1].goal = format!("Add{bad}");
    let mut t = blocked(
        &format!("t{bad}"),
        BlockReason::Question,
        &format!("q{bad}\n{bad}"),
    );
    t.history = vec![event(NOW - 41, &format!("blocked (question): {bad}"))];
    runs[1].tasks = vec![t];
    runs[0].goal = bad.clone();
    let app = app_of(vec![], runs);
    for (w, h) in [(80, 24), (120, 40)] {
        let (buffer, layout) = draw_at(&app, w, h);
        let area = layout.alerts;
        for y in area.y..area.y + area.height {
            let text = text_in(&buffer, area, y);
            assert_eq!(first_hostile(&text), None, "{w}x{h}: {text:?}");
        }
        assert!(
            box_rows(&buffer, &layout)
                .iter()
                .any(|r| r.contains(" › t")),
            "the blocked alert is drawn"
        );
    }
    for alert in alerts(&app) {
        for width in [4, 24, 32, 400] {
            for line in super::alert_lines(&app, &alert, width) {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(first_hostile(&text), None, "{text:?}");
            }
        }
    }
}

/// A raw `Alert` (not from `app::alerts`) is sanitised by the box itself.
#[test]
fn a_raw_alert_is_sanitised_when_drawn() {
    let bad = hostile_text();
    let app = three_runs();
    let alert = Alert {
        priority: 3,
        key: AlertKey::Halted("r".into()),
        who: AlertWho::Run {
            goal: format!("g{bad}"),
            id: format!("r{bad}"),
        },
        task: Some(format!("t{bad}")),
        text: format!("x{bad}"),
        detail: bad.clone(),
        age: Some(5),
    };
    let project = Alert {
        who: AlertWho::Project(format!("p{bad}")),
        ..alert.clone()
    };
    for alert in [alert, project] {
        for width in [0, 4, 20, 400] {
            for line in super::alert_lines(&app, &alert, width) {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(first_hostile(&text), None, "{width}: {text:?}");
                assert!(unicode_width::UnicodeWidthStr::width(text.as_str()) <= usize::from(width));
            }
        }
    }
}

#[test]
fn empty_box_says_no_alerts() {
    let app = app_of(vec![pty(1, "shell", REPO, Status::Idle)], vec![]);
    for (w, h) in [(80, 24), (120, 40)] {
        let (buffer, layout) = draw_at(&app, w, h);
        assert_eq!(layout.alerts_inner.height, 1);
        assert_eq!(trimmed(&box_rows(&buffer, &layout)), ["no alerts"]);
        let top = row(&buffer, layout.alerts.y);
        assert!(top.starts_with("╭ Alerts ─"), "{top}");
        assert!(!top.contains('⚑'), "{top}");
        let x = layout.alerts.x + 2;
        assert_eq!(buffer[(x, layout.alerts.y)].symbol(), "A");
        assert_eq!(
            Some(buffer[(x, layout.alerts.y)].fg),
            theme::role(Role::Muted, app.palette()).fg
        );
        let bottom = row(&buffer, layout.alerts.y + layout.alerts.height - 1);
        assert!(!bottom.contains("open"), "{bottom}");
        assert!(bottom.starts_with("╰────"), "{bottom}");
    }
}

#[test]
fn three_alerts_in_ascii() {
    let mut app = three_runs();
    app.settings.badges.ascii = true;
    let (buffer, layout) = draw_at(&app, 80, 24);
    assert_eq!(
        trimmed(&box_rows(&buffer, &layout)),
        vec![
            "! Docs - 77aa",
            "  plan awaits approval - 2 tasks",
            "! Add mul() - 0723 > t2      41s",
            "  blocked: which crate owns the",
            "  formatting helper?",
            "+ Fix CI - 9b1e",
            "  ready to accept - 1/1 merged",
        ]
    );
    let top = row(&buffer, layout.alerts.y);
    assert!(top.contains(" ! Alerts 3 "), "{top}");
}

#[test]
fn a_long_text_wraps_to_two_lines_cut_with_a_mark() {
    let mut runs = three_runs_snapshot();
    runs[1].tasks[1].block.as_mut().unwrap().text =
        "one two three four five six seven eight nine ten eleven twelve thirteen".into();
    let app = app_of(vec![], runs);
    let alert = alerts(&app).into_iter().find(|a| a.task.is_some()).unwrap();
    let lines: Vec<String> = super::alert_lines(&app, &alert, 32)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    assert_eq!(lines.len(), 3, "{lines:#?}");
    assert_eq!(lines[1], "  blocked: one two three four");
    assert!(lines[2].starts_with("  five six seven"), "{lines:#?}");
    assert!(lines[2].ends_with('…'), "{lines:#?}");
    assert!(unicode_width::UnicodeWidthStr::width(lines[2].as_str()) <= 32);
}

/// A word wider than the text column is broken, never re-joined with a space.
#[test]
fn a_long_word_is_broken_without_a_space() {
    let word = "w".repeat(40);
    let mut runs = three_runs_snapshot();
    runs[1].tasks[1].block.as_mut().unwrap().text = format!("{word} {word}");
    let app = app_of(vec![], runs);
    let alert = alerts(&app).into_iter().find(|a| a.task.is_some()).unwrap();
    let lines: Vec<String> = super::alert_lines(&app, &alert, 32)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    assert_eq!(
        lines[1..],
        ["  blocked:".to_owned(), format!("  {}…", "w".repeat(29))],
        "{lines:#?}"
    );
}

#[test]
fn no_panic_at_tiny_sizes() {
    for sidebar in [0, 1, 2, 3, 24, 34] {
        for (w, h) in [(1, 1), (2, 2), (20, 5), (40, 3), (12, 4), (5, 40), (80, 24)] {
            let mut app = three_runs();
            app.sidebar_width = sidebar;
            draw_at(&app, w, h);
        }
    }
}
