//! Milestone 9.0.7 decision 36: the shared render audit. One `App` per key region of
//! decision 1 (`fixtures`), drawn whole (`draw`) and read back (`rows`, `find`), and
//! §6.9's checks over a drawn frame: `accented_frames`, `first_non_ascii`,
//! `assert_actionable`. Every screen task adds its screen's fixture here; fixture text
//! is ASCII only, so an ASCII render can be checked whole. Pure, and test-only.

use crate::app::{App, Modal};
use crate::theme::{Palette, Role, role};
use crate::tree::run_fixtures::{PROJECT, RUN_ID, gate_fixture, pty, three_task_fixture};
use crate::{settings::UiSettings, tree};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DaemonMsg, RunReply, RunsSnapshot, WindowInfo};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

/// `app` drawn whole by `ui::draw` on a `w`×`h` terminal.
pub(crate) fn draw(app: &App, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("a test terminal");
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .expect("a frame");
    terminal.backend().buffer().clone()
}

/// Every row of `buffer`, its cells' symbols joined, trailing spaces trimmed.
pub(crate) fn rows(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (area.y..area.bottom())
        .map(|y| {
            let row: String = (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.trim_end().to_owned()
        })
        .collect()
}

/// Where `text` starts on screen, `(column, row)` of its first cell, for every match,
/// top to bottom: cells matched by their symbols, so wide glyphs count as drawn.
pub(crate) fn find(buffer: &Buffer, text: &str) -> Vec<(u16, u16)> {
    let want: Vec<char> = text.chars().collect();
    let area = buffer.area;
    let mut out = Vec::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let mut cell_x = x;
            let mut at = 0;
            while at < want.len() && cell_x < area.right() {
                let symbol: Vec<char> = buffer[(cell_x, y)].symbol().chars().collect();
                if symbol.is_empty() || want[at..].iter().take(symbol.len()).ne(&symbol) {
                    break;
                }
                at += symbol.len();
                cell_x += 1;
            }
            if at == want.len() && !want.is_empty() {
                out.push((x, y));
            }
        }
    }
    out
}

const CORNERS: [&str; 3] = ["╭", "┌", "+"];
const BOTTOM_LEFTS: [&str; 3] = ["╰", "└", "+"];
const HORIZONTALS: [&str; 2] = ["─", "-"];
const VERTICALS: [&str; 2] = ["│", "|"];
/// Every glyph a border is drawn with, the junctions an edge leaves on a box included.
const BORDER_GLYPHS: &str = "╭╮╰╯┌┐└┘├┤┬┴┼─│+-|";

/// How many frames wear the accent. A frame is a top-left corner (`╭`, `┌` or `+`) in
/// the accent whose left side runs down, as verticals (`│`, `|`) in the accent, to a
/// bottom-left corner (`╰`, `└` or `+`) in the accent whose bottom edge goes on to the
/// right: the first accented border glyph after the corner (past a bottom title) is a
/// horizontal (`─`, `-`). The bottom edge is read, not the top, because a title
/// starts in the column right after the top corner and can fill the top row; a
/// modal's top row can also lie on a pane's top row. A left side that runs off the
/// buffer's last row is a frame clipped at the screen's edge and counts (ratatui itself
/// draws a clipped block's bottom border on the last row; this is a frame drawn
/// without one, or cut by hand), unless its
/// corner is a `+` right after an accented horizontal (an ASCII frame's top-right
/// corner, whose right side would read the same). Each top-left corner is one frame.
pub(crate) fn accented_frames(buffer: &Buffer, p: Palette) -> usize {
    let accent = role(Role::Accent, p).fg;
    let area = buffer.area;
    let lit = |x: u16, y: u16| Some(buffer[(x, y)].fg) == accent;
    let is = |x: u16, y: u16, set: &[&str]| lit(x, y) && set.contains(&buffer[(x, y)].symbol());
    let mut count = 0;
    for y in area.y..area.bottom().saturating_sub(1) {
        for x in area.x..area.right() {
            if !is(x, y, &CORNERS) || !is(x, y + 1, &VERTICALS) {
                continue;
            }
            let mut bottom = y + 1;
            while bottom < area.bottom() && is(x, bottom, &VERTICALS) {
                bottom += 1;
            }
            let frame = if bottom == area.bottom() {
                let top_right =
                    buffer[(x, y)].symbol() == "+" && x > area.x && is(x - 1, y, &HORIZONTALS);
                !top_right
            } else {
                // A top-left corner ends the side too: a frame stacked on this one
                // shares its bottom row.
                let ends = is(x, bottom, &BOTTOM_LEFTS) || is(x, bottom, &CORNERS);
                let border = |e: u16| {
                    let symbol = buffer[(e, bottom)].symbol();
                    lit(e, bottom) && !symbol.is_empty() && BORDER_GLYPHS.contains(symbol)
                };
                ends && (x + 1..area.right())
                    .find(|&e| border(e))
                    .is_some_and(|e| is(e, bottom, &HORIZONTALS))
            };
            if frame {
                count += 1;
            }
        }
    }
    count
}

/// The first cell, row by row, whose symbol is not ASCII: `(column, row, symbol)`.
pub(crate) fn first_non_ascii(buffer: &Buffer) -> Option<(u16, u16, String)> {
    let area = buffer.area;
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .find(|&(x, y)| !buffer[(x, y)].symbol().is_ascii())
        .map(|(x, y)| (x, y, buffer[(x, y)].symbol().to_owned()))
}

/// §5.1 principle 3: `text` is on screen, and at least one of its cells is not drawn
/// in `Muted` (anything the user would act on is never only muted). Fails otherwise.
pub(crate) fn assert_actionable(buffer: &Buffer, text: &str, p: Palette) {
    let muted = role(Role::Muted, p).fg;
    let matches = find(buffer, text);
    assert!(
        !matches.is_empty(),
        "{text:?} is not on screen:\n{}",
        rows(buffer).join("\n")
    );
    let lit = matches.iter().any(|&(x, y)| {
        let cells = text.chars().filter(|c| *c != ' ').count();
        (x..buffer.area.right())
            .map(|cx| &buffer[(cx, y)])
            .filter(|cell| cell.symbol() != " ")
            .take(cells)
            .any(|cell| Some(cell.fg) != muted)
    });
    assert!(lit, "{text:?} is drawn only in muted");
}

/// What a fixture's frame must show (§6.9): its state word, its `esc` hint where the
/// mode has one, and the keys a user acts on, none of them only in `Muted`.
pub(crate) struct Shows {
    pub state: &'static str,
    pub esc: Option<&'static str>,
    pub actionable: &'static [&'static str],
}

/// Each fixture's [`Shows`], by its name. A fixture without a row fails the audit.
pub(crate) fn shows(name: &str) -> Shows {
    let row = |state, esc, actionable| Shows {
        state,
        esc,
        actionable,
    };
    match name {
        // Milestone 9.0.7 decision 29: the pane's title, ` sh shell · /r/demo `.
        "pane" | "sidebar tree, then hidden" => row("sh shell", None, &["C-b ? help"]),
        "sidebar tree" | "sidebar tree overflowing" => row("TREE", Some("esc back"), &["j/k move"]),
        // Milestone 9.0.7 decision 31: the overview wears its own badge.
        "project overview" => row("OVERVIEW", Some("esc back"), &["j/k move"]),
        "run view at the gate" => row("RUN", Some("esc back"), &["a approve", "x reject"]),
        "run view running" | "run view on a task" => row("RUN", Some("esc back"), &["j/k move"]),
        // Milestone 9.0.7 decision 12: the task panel's state word, always shown.
        "run view on a task in review" => row("in review · r2", Some("esc back"), &["j/k move"]),
        // Decision 22: at 80x24 the two-stage run is the compact list; the task panel
        // below names `t2`'s state whole.
        "run view as a list" => row("in review · r1", Some("esc back"), &["j/k move"]),
        "conversation" => row("CHAT", None, &["C-b ? help"]),
        // Decision 11: the Alerts view on `t2`'s blocked alert. Its detail row's `o open
        // task` drops first at 80x24 (decision 34's priorities); the bar keeps `o open`.
        "alerts view" => row("phase", Some("esc back"), &["⏎ answer", "o open"]),
        // Milestone 9.0.7 decisions 9 and 10: the grown box, two lines an alert.
        "alerts box" => row("Alerts 3", None, &["C-b a open"]),
        "sidebar tree over the alerts box" => {
            row("TREE", Some("esc back"), &["j/k move", "C-b a open"])
        }
        "help over the sidebar tree" | "profile under the help" => row("keys", None, &["any key"]),
        "confirm over the overview" => row("Kill 'shell'?", Some("esc back"), &["y kill"]),
        "action menu over the run view" => row("review plan", Some("esc close"), &["j/k move"]),
        "profile with a page" => row("reject proposal", Some("esc back"), &["y reject"]),
        "settings" => row("SETTINGS", Some("esc back"), &["w save"]),
        "stats" => row("STATS", Some("esc back"), &["j/k scroll"]),
        other => panic!("the audit fixture {other:?} names nothing it shows"),
    }
}

fn app_of(windows: Vec<WindowInfo>, snapshot: RunsSnapshot) -> App {
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let _ = app.set_terminal_size(80, 24);
    let _ = app.run_subscription();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot)));
    app
}

fn gate() -> App {
    let (snap, windows) = gate_fixture();
    app_of(windows, snap)
}

/// The gate fixture's run with 45 shells: more rows than the sidebar list holds at
/// 80x24 or 120x40.
fn crowded() -> App {
    let (snap, _) = gate_fixture();
    let windows = (1..=45)
        .map(|id| pty(id, &format!("shell-{id}"), PROJECT, proto::Status::Idle))
        .collect();
    app_of(windows, snap)
}

fn running() -> App {
    let (snap, windows) = three_task_fixture();
    app_of(windows, snap)
}

fn chord(app: &mut App, c: char) {
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

fn tap(app: &mut App, c: char) {
    app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

/// `C-b T`, the run's node selected, then `l`: the run view.
fn run_view(mut app: App) -> App {
    chord(&mut app, 'T');
    let key = tree::NodeKey::Run(RUN_ID.into());
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, key);
    tap(&mut app, 'l');
    assert!(app.run_view.is_some(), "the run view opened");
    app
}

/// The run view with task `id` selected: its task panel below the canvas.
fn on_task(mut app: App, id: &str) -> App {
    let run = app.runs.runs[0].clone();
    let rows = tree::run_rows(&run, &app.windows, &app.tree, tree::RunFilter::All);
    let key = tree::NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    };
    let rows: Vec<_> = rows.into_iter().map(|row| row.key).collect();
    assert!(rows.contains(&key), "{id} is on the canvas");
    app.tree.selected = Some(key);
    app
}

/// `t1` in review round 2, after round 1 asked for changes.
fn in_review_r2(app: &mut App) {
    let t1 = &mut app.runs.runs[0].tasks[1];
    t1.state = proto::TaskState::Review;
    let route = t1.rounds[0].route.clone();
    let review = |round, verdict: Option<proto::Verdict>| proto::ReviewInfo {
        round,
        route: route.clone(),
        verdict,
        summary: "one bug".into(),
        findings: Vec::new(),
        blocking: verdict.is_some(),
    };
    t1.review_route = Some(route.clone());
    t1.reviews = vec![review(1, Some(proto::Verdict::Changes)), review(2, None)];
}

fn with(mut app: App, change: impl FnOnce(&mut App)) -> App {
    change(&mut app);
    app
}

/// One `App` per key region of decision 1 that exists, each reached by the keys a
/// user presses where it can be. Later tasks add theirs (the plan review's frame, the
/// old dialogs).
pub(crate) fn fixtures() -> Vec<(&'static str, App)> {
    use crate::app::profile_screen::ProfilePage;
    use crate::app::screens::Screen;
    use crate::app::stats::{StatsScreen, StatsState};
    let profile = || crate::ui::profile::tests::app_with_profile(|_, _| {});
    vec![
        ("pane", gate()),
        ("sidebar tree", with(gate(), |a| chord(a, 't'))),
        // Decision 27: rows cut above and below, marked in the accented border.
        (
            "sidebar tree overflowing",
            with(crowded(), |a| {
                chord(a, 't');
                a.tree.sidebar.top = 6;
            }),
        ),
        (
            "sidebar tree, then hidden",
            with(gate(), |a| {
                chord(a, 't');
                chord(a, 's');
            }),
        ),
        ("project overview", with(gate(), |a| chord(a, 'T'))),
        // Decisions 9 and 10: three alerts grow the box under the agents block.
        ("alerts box", crate::ui::alerts::fixture::three_runs()),
        (
            "sidebar tree over the alerts box",
            with(crate::ui::alerts::fixture::three_runs(), |a| chord(a, 't')),
        ),
        ("run view at the gate", run_view(gate())),
        ("run view running", run_view(running())),
        ("run view on a task", on_task(run_view(running()), "t1")),
        (
            "run view on a task in review",
            on_task(run_view(with(running(), in_review_r2)), "t1"),
        ),
        (
            "run view as a list",
            crate::ui::run_list::tests::two_stage_run_selected("t2"),
        ),
        ("conversation", with(gate(), |a| chord(a, 'm'))),
        // Decision 11: `C-b a`, then `j`: the Alerts view on the blocked alert.
        (
            "alerts view",
            with(crate::ui::alerts::fixture::three_runs(), |a| {
                chord(a, 'a');
                tap(a, 'j');
            }),
        ),
        (
            "help over the sidebar tree",
            with(gate(), |a| {
                chord(a, 't');
                chord(a, '?');
            }),
        ),
        (
            "confirm over the overview",
            with(gate(), |a| {
                chord(a, 'T');
                chord(a, 'x');
                assert!(
                    matches!(a.modal, Some(Modal::Confirm { .. })),
                    "`C-b x` asks"
                );
            }),
        ),
        (
            "action menu over the run view",
            with(run_view(gate()), |a| tap(a, '.')),
        ),
        (
            "profile with a page",
            with(profile(), |a| {
                crate::ui::profile::tests::screen_mut(a).page = Some(ProfilePage::Reject);
            }),
        ),
        ("profile under the help", with(profile(), |a| chord(a, '?'))),
        (
            "settings",
            crate::ui::settings::tests::opened(false, crate::ui::settings::tests::sample()),
        ),
        (
            "stats",
            with(gate(), |a| {
                a.screen = Some(Screen::Stats(Box::new(StatsScreen {
                    project: "/r/demo".into(),
                    state: StatsState::Ready(Box::new(crate::ui::stats::tests::history())),
                    scroll: 0,
                    request: 4,
                })));
            }),
        ),
    ]
}
