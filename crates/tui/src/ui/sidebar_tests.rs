//! M9.0.7.4: the agents block's overflow marks and its run rows (decisions 27, 28).

use crate::app::App;
use crate::settings::UiSettings;
use crate::theme::{self, Role};
use crate::tree::run_fixtures::{PROJECT, pty, run, snapshot, task};
use crate::ui::{self, audit, tree_view};
use crate::{tree, ui::badge::BadgeSet};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{RunState, Size, Status, TaskState};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

/// `n` idle shells `shell-1`…`shell-n` in one project, the terminal `w`×`h`.
fn shells(n: u32, w: u16, h: u16) -> App {
    let windows = (1..=n)
        .map(|id| pty(id, &format!("shell-{id}"), PROJECT, Status::Idle))
        .collect();
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let _ = app.set_terminal_size(w, h);
    app
}

fn ascii(app: &mut App) {
    app.settings.badges = BadgeSet::from_config(&app.settings.badges_config, true);
}

/// `app` at `w`×`h`, the tree's viewports set from the frame's layout first, as the
/// event loop does (`lib.rs`), so the sidebar scrolls to the focused window.
fn frame(app: &mut App, w: u16, h: u16) -> (Buffer, ui::Layout) {
    let l = ui::layout_for(app, Rect::new(0, 0, w, h));
    app.set_tree_viewports(l.sidebar_list.height, l.main_inner.height);
    (audit::draw(app, w, h), l)
}

fn text_in(buffer: &Buffer, rect: Rect, y: u16) -> String {
    (rect.x..rect.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The number in `… <glyph> <n> more …`.
fn count_after(text: &str, glyph: &str) -> usize {
    let at = text
        .find(glyph)
        .unwrap_or_else(|| panic!("no {glyph:?} in {text:?}"));
    text[at + glyph.len()..]
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no count after {glyph:?} in {text:?}"))
}

/// Decision 27: tree rows above and below the list are counted in the agents block's
/// border, muted; above + shown + below is every row.
#[test]
fn overflow_is_marked_in_the_border() {
    for ascii_mode in [false, true] {
        let mut app = shells(30, 80, 24);
        if ascii_mode {
            ascii(&mut app);
        }
        app.focus(20);
        let (buffer, l) = frame(&mut app, 80, 24);
        let (up, down, top_right, bottom_right) = if ascii_mode {
            ("^", "v", "+", "+")
        } else {
            ("↑", "↓", "╮", "╯")
        };
        let top = text_in(&buffer, l.sidebar, l.sidebar.y);
        let bottom = text_in(&buffer, l.sidebar, l.sidebar.bottom() - 1);
        let above = count_after(&top, up);
        let below = count_after(&bottom, down);
        assert!(
            top.ends_with(&format!(" {up} {above} more {top_right}")),
            "{top:?}"
        );
        assert!(
            bottom.ends_with(&format!(" {down} {below} more {bottom_right}")),
            "{bottom:?}"
        );
        let shown = (l.sidebar_list.y..l.sidebar_list.bottom())
            .filter(|&y| !text_in(&buffer, l.sidebar_list, y).trim().is_empty())
            .count();
        // Thirty shells and their project's row.
        let rows = app.rows().len();
        assert_eq!(rows, 31);
        assert!(above > 0 && below > 0, "{top:?} {bottom:?}");
        assert_eq!(above + shown + below, rows, "{top:?} {bottom:?}");
        let muted = theme::role(Role::Muted, app.palette()).fg;
        let x = l.sidebar.x + top.find(up).map(|at| top[..at].width()).unwrap() as u16;
        assert_eq!(
            Some(buffer[(x, l.sidebar.y)].fg),
            muted,
            "the mark is muted"
        );
        if ascii_mode {
            assert_eq!(audit::first_non_ascii(&buffer), None);
        }
    }
    // Nothing cut, no mark.
    let mut app = shells(3, 80, 24);
    let (buffer, l) = frame(&mut app, 80, 24);
    let top = text_in(&buffer, l.sidebar, l.sidebar.y);
    let bottom = text_in(&buffer, l.sidebar, l.sidebar.bottom() - 1);
    assert!(
        !top.contains("more") && !bottom.contains("more"),
        "{top:?} {bottom:?}"
    );
}

/// Decision 27: the spacer row is gone; the last tree row sits right on the footer.
#[test]
fn the_list_reaches_the_footer() {
    let mut app = shells(40, 120, 40);
    let (buffer, l) = frame(&mut app, 120, 40);
    assert_eq!(l.sidebar_list.bottom(), l.sidebar_footer.y);
    let last = text_in(&buffer, l.sidebar_list, l.sidebar_footer.y - 1);
    assert!(last.contains("shell-"), "{last:?}");
    let footer = text_in(&buffer, l.sidebar_footer, l.sidebar_footer.y);
    assert!(footer.contains("40 agents"), "{footer:?}");
}

/// A run `Add mul()` / `add-mul-0723` with two of its three tasks merged.
fn mul_app(goal: &str) -> App {
    let mut info = run("add-mul-0723", PROJECT, RunState::Running);
    info.goal = goal.into();
    info.tasks = vec![
        task("t1", "mul", Size::S, TaskState::Merged),
        task("t2", "tests", Size::S, TaskState::Merged),
        task("t3", "docs", Size::S, TaskState::Queued),
    ];
    let windows = vec![pty(1, "shell", PROJECT, Status::Idle)];
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let _ = app.on_run_reply(proto::RunReply::Snapshot(snapshot(10_000, vec![info])));
    app
}

fn row_line(app: &App, run_row: bool, width: u16) -> ratatui::text::Line<'static> {
    let rows = app.rows();
    let row = rows
        .iter()
        .find(|row| match row.kind {
            tree::RowKind::Run { .. } => run_row,
            tree::RowKind::Window { .. } => !run_row,
            _ => false,
        })
        .expect("the row");
    tree_view::narrow_line(app, row, width, 1, false)
}

fn line_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Decision 28: the run's one name, `kit::run_name_in`, and `<m>/<n> ✓` with the
/// `✓` in `Done` and the count muted.
#[test]
fn run_rows_name_the_run_once() {
    let app = mul_app("Add mul()");
    let p = app.palette();
    let line = row_line(&app, true, 34);
    let text = line_text(&line);
    assert!(text.contains("Add mul() · 0723"), "{text:?}");
    assert_eq!(text.matches("Add mul()").count(), 1, "{text:?}");
    assert!(text.ends_with("2/3 ✓"), "{text:?}");
    assert_eq!(text.width(), 34, "{text:?}");
    let tick = line
        .spans
        .iter()
        .find(|s| s.content == "✓")
        .expect("a tick span");
    assert_eq!(tick.style, theme::role(Role::Done, p));
    let count = line
        .spans
        .iter()
        .find(|s| s.content.contains("2/3"))
        .unwrap();
    assert_eq!(count.style, theme::role(Role::Muted, p));
    // A narrow sidebar cuts the goal, never the short id.
    let text = line_text(&row_line(&app, true, 24));
    assert!(text.contains("· 0723"), "{text:?}");
    assert!(text.contains('…'), "{text:?}");
    // A blank goal falls back to the run id, as `tree::run_title` does.
    let app = mul_app("  ");
    let text = line_text(&row_line(&app, true, 40));
    assert!(text.contains("add-mul-0723 · 0723"), "{text:?}");
    // ASCII.
    let mut app = mul_app("Add mul()");
    ascii(&mut app);
    let text = line_text(&row_line(&app, true, 34));
    assert!(text.contains("Add mul() - 0723"), "{text:?}");
    assert!(text.ends_with("2/3 +"), "{text:?}");
    assert!(text.is_ascii(), "{text:?}");
}

/// The display column each prefix span starts at.
fn columns(line: &ratatui::text::Line<'_>, spans: usize) -> Vec<usize> {
    let mut at = 0;
    line.spans
        .iter()
        .take(spans)
        .map(|span| {
            let start = at;
            at += span.content.width();
            start
        })
        .collect()
}

/// Decision 28 (pinning): a run row's glyph and position sit in a window row's columns.
#[test]
fn run_rows_align_with_window_rows() {
    let (snap, windows) = tree::run_fixtures::three_task_fixture();
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let _ = app.on_run_reply(proto::RunReply::Snapshot(snap));
    for width in [24, 34, 60] {
        let run_line = row_line(&app, true, width);
        let window_line = row_line(&app, false, width);
        // guides, focus mark, fold mark, glyph, position.
        assert_eq!(
            columns(&run_line, 5),
            columns(&window_line, 5),
            "at {width}: {:?} / {:?}",
            line_text(&run_line),
            line_text(&window_line)
        );
        assert_eq!(
            run_line.spans[4].content.width(),
            window_line.spans[4].content.width()
        );
    }
}

/// The audit's fixtures carry this task's texts, in colour and in ASCII: the pane
/// title, the run row's name and progress, and the overflow marks.
#[test]
fn the_audit_fixtures_carry_the_new_texts() {
    let cases: [(&str, &[&str], &[&str]); 3] = [
        ("pane", &["sh shell · /r/demo"], &["sh shell - /r/demo"]),
        ("sidebar tree", &["· 3f9a", "0/2 ✓"], &["- 3f9a", "0/2 +"]),
        (
            "sidebar tree overflowing",
            &["↑ 6 more ╮", " more ╯"],
            &["^ 6 more +", "v "],
        ),
    ];
    for (name, unicode, plain) in cases {
        let (_, mut app) = audit::fixtures()
            .into_iter()
            .find(|(n, _)| *n == name)
            .expect("the fixture");
        for (w, h) in [(80, 24), (120, 40)] {
            let shown = audit::rows(&audit::draw(&app, w, h)).join("\n");
            for text in unicode {
                assert!(shown.contains(text), "{name} at {w}x{h}: {text:?}\n{shown}");
            }
        }
        ascii(&mut app);
        for (w, h) in [(80, 24), (120, 40)] {
            let shown = audit::rows(&audit::draw(&app, w, h)).join("\n");
            for text in plain {
                assert!(shown.contains(text), "{name} at {w}x{h}: {text:?}\n{shown}");
            }
        }
    }
}

/// Review fix round 1 (principle 6): at the narrow sidebar widths the top mark is kept
/// whole; ` · tree` goes first, then `agents` is cut with `…`, never mid-word unmarked.
#[test]
fn the_mark_never_cuts_the_title_mid_word() {
    for ascii_mode in [false, true] {
        let (up, corner, ellipsis, tree) = if ascii_mode {
            ("^", "+", ".", "agents - tree")
        } else {
            ("↑", "╮", "…", "agents · tree")
        };
        for width in [24, 26] {
            for above in [5, 50, 100, 1000] {
                let mut app = shells(above + 20, 80, 24);
                if ascii_mode {
                    ascii(&mut app);
                }
                app.sidebar_width = width;
                app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
                app.on_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
                assert!(app.tree_input.is_some());
                app.tree.sidebar.top = above as usize;
                let buffer = audit::draw(&app, 80, 24);
                let top: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
                let mark = format!(" {up} {above} more {corner}");
                assert!(top.ends_with(&mark), "{width} {above}: {top:?}");
                let title = top[..top.len() - mark.len()]
                    .chars()
                    .skip(1)
                    .collect::<String>();
                let title = title.trim_matches([' ', '─', '-']);
                let whole = title == tree || title == "agents";
                let cut = title
                    .strip_suffix(ellipsis)
                    .is_some_and(|head| "agents".starts_with(head.trim_end_matches('.')));
                assert!(whole || cut, "{width} {above}: {top:?}");
            }
        }
    }
}

/// Final fix wave M3: one selection idiom. The tree's selected row is reversed and the
/// `▌` bar (`>` in ASCII) in the accent stands in the column left of it, the agents
/// block's left border, as a graph box's (decision 20) and the compact list's.
#[test]
fn the_selected_tree_row_wears_the_selection_bar() {
    for ascii_mode in [false, true] {
        let mut app = shells(3, 80, 24);
        if ascii_mode {
            ascii(&mut app);
        }
        app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        app.on_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        let (buffer, l) = frame(&mut app, 80, 24);
        let list = l.sidebar_list;
        let reversed = |y: u16| {
            buffer[(list.x + 2, y)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED)
        };
        let y = (list.y..list.bottom())
            .find(|&y| reversed(y))
            .expect("a selected row");
        let bar = if ascii_mode { ">" } else { "▌" };
        assert_eq!(buffer[(l.sidebar.x, y)].symbol(), bar, "ascii {ascii_mode}");
        let accent = theme::role(Role::Accent, app.palette()).fg;
        assert_eq!(Some(buffer[(l.sidebar.x, y)].fg), accent);
        let side = if ascii_mode { "|" } else { "│" };
        for other in (list.y..list.bottom()).filter(|&o| o != y) {
            assert_eq!(buffer[(l.sidebar.x, other)].symbol(), side, "row {other}");
        }
        // Outside tree mode nothing is selected, and the border is whole.
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let (buffer, _) = frame(&mut app, 80, 24);
        assert_eq!(buffer[(l.sidebar.x, y)].symbol(), side);
    }
}

/// Task 4's deferred minor: beside a six-digit overflow mark at the least sidebar width
/// the title keeps the mark whole and cuts `agents` with `…` (`...` in ASCII).
#[test]
fn the_title_cuts_agents_beside_a_six_digit_mark() {
    let mark = UnicodeWidthStr::width("↑ 123456 more");
    let interior = ui::MIN_SIDEBAR_WIDTH - 2;
    assert_eq!(super::title_beside(true, mark, interior, false), "agen…");
    assert_eq!(super::title_beside(false, mark, interior, true), "ag...");
    // One column wider still shows `agents` whole, ` · tree` gone first.
    assert_eq!(
        super::title_beside(true, mark, interior + 1, false),
        "agents"
    );
}
