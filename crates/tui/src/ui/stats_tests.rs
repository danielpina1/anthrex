//! Milestone 9.0.6 task 15: the stats screen, drawn (decision 38).

use crate::app::App;
use crate::app::screens::Screen;
use crate::app::stats::{StatsScreen, StatsState};
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use proto::{FlakyProposal, HistoryStats, StatsRow};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];

/// A history with two classes (one with no merged task, so its medians are `None`),
/// two flaky proposals and one problem. The app tests share it.
pub(crate) fn history() -> HistoryStats {
    HistoryStats {
        path: "/r/demo".into(),
        task_records: 7,
        run_records: 3,
        rows: vec![
            StatsRow {
                class: "S".into(),
                tasks: 5,
                merged: 4,
                median_lines: Some(42),
                median_tool_calls: Some(18),
                median_tokens: Some(52_300),
                median_work_secs: Some(380),
                bounces: 1,
                reverted: 0,
            },
            StatsRow {
                class: "hub".into(),
                tasks: 2,
                merged: 0,
                median_lines: None,
                median_tool_calls: None,
                median_tokens: None,
                median_work_secs: None,
                bounces: 2,
                reverted: 1,
            },
        ],
        decider_calls: 6,
        decider_fallbacks: 1,
        size_checked: 4,
        size_raised: 1,
        problems: vec!["runs.jsonl line 9: missing field `at`".into()],
        flaky_proposals: vec![
            FlakyProposal {
                test: "tests::login_retries".into(),
                runs: 4,
                last_at: 100,
            },
            FlakyProposal {
                test: "api::slow_ping".into(),
                runs: 1,
                last_at: 90,
            },
        ],
        window_days: 14,
        quarantine_after: 3,
    }
}

fn app_with(ascii: bool, state: StatsState) -> App {
    let mut settings = UiSettings::default();
    settings.badges.ascii = ascii;
    let mut app = App::new(vec![], "/tmp".into(), settings);
    app.set_terminal_size(80, 24);
    app.screen = Some(Screen::Stats(Box::new(StatsScreen {
        project: "/r/demo".into(),
        state,
        scroll: 0,
    })));
    app
}

fn ready(ascii: bool) -> App {
    app_with(ascii, StatsState::Ready(Box::new(history())))
}

fn screen_mut(app: &mut App) -> &mut StatsScreen {
    match &mut app.screen {
        Some(Screen::Stats(s)) => s,
        _ => panic!("no stats screen"),
    }
}

fn draw(app: &App, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn screen_text(app: &App, w: u16, h: u16) -> String {
    let buffer = draw(app, w, h);
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
        .collect()
}

/// The screen's interior rows (inside the frame, above the status bar), trimmed.
fn rows(app: &App, w: u16, h: u16) -> Vec<String> {
    let buffer = draw(app, w, h);
    (1..h - 2)
        .map(|y| {
            (1..w - 1)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

const READY: [&str; 14] = [
    "7 task records · 3 runs",
    "",
    "class  tasks  merged  lines  calls  tokens  work  bounces  reverted",
    "S      5      4       42     18     52k     6m    1        0",
    "hub    2      0       –      –      –       –     2        1",
    "",
    "deciders 6 calls · 1 fallback · size raised 1/4",
    "",
    "flaky proposals (14 days, after 3)",
    "  tests::login_retries  4 runs",
    "  api::slow_ping        1 run",
    "",
    "problems",
    "  runs.jsonl line 9: missing field `at`",
];

/// Decision 38's rows, exactly: the table (a `None` median reads `–`), the deciders
/// line, the flaky proposals and the problems, at 80×24 and 120×40, under the title.
#[test]
fn stats_render_rows_deciders_and_flaky_proposals() {
    let app = ready(false);
    for (w, h) in SIZES {
        let all = rows(&app, w, h);
        assert_eq!(all[..READY.len()], READY, "{w}x{h}");
        assert!(all[READY.len()..].iter().all(String::is_empty), "{w}x{h}");
        let text = screen_text(&app, w, h);
        assert!(
            text.lines().next().unwrap().contains(" stats · demo "),
            "{text}"
        );
        assert_eq!(
            super::body_lines(&app, screen(&app), w).len(),
            screen(&app).line_count(),
            "the app scrolls over exactly the lines drawn"
        );
    }
}

fn screen(app: &App) -> &StatsScreen {
    match &app.screen {
        Some(Screen::Stats(s)) => s,
        _ => panic!("no stats screen"),
    }
}

/// ASCII mode: `-` for a `None` median and for `·`, and an ASCII frame.
#[test]
fn stats_render_in_ascii() {
    let app = ready(true);
    for (w, h) in SIZES {
        let all = rows(&app, w, h);
        assert_eq!(all[0], "7 task records - 3 runs", "{w}x{h}");
        assert_eq!(
            all[4], "hub    2      0       -      -      -       -     2        1",
            "{w}x{h}"
        );
        assert_eq!(
            all[6], "deciders 6 calls - 1 fallback - size raised 1/4",
            "{w}x{h}"
        );
        let text = screen_text(&app, w, h);
        assert!(text.starts_with("+ stats - demo ---"), "{text}");
        assert!(text.is_ascii(), "{text}");
    }
}

/// With no proposal the heading still shows, with `none`; with no problem there is no
/// `problems` block.
#[test]
fn stats_render_an_empty_history() {
    let mut stats = history();
    stats.rows.clear();
    stats.flaky_proposals.clear();
    stats.problems.clear();
    stats.task_records = 1;
    stats.run_records = 1;
    let app = app_with(false, StatsState::Ready(Box::new(stats)));
    let all = rows(&app, 80, 24);
    assert_eq!(
        all[..8],
        [
            "1 task record · 1 run",
            "",
            "class  tasks  merged  lines  calls  tokens  work  bounces  reverted",
            "",
            "deciders 6 calls · 1 fallback · size raised 1/4",
            "",
            "flaky proposals (14 days, after 3)",
            "  none",
        ]
    );
    assert!(all[8..].iter().all(String::is_empty), "{all:#?}");
}

/// Loading, and a refusal or an expiry in `Failed`, never a toast.
#[test]
fn stats_render_loading_and_failed() {
    for (ascii, loading) in [(false, "loading…"), (true, "loading...")] {
        let app = app_with(ascii, StatsState::Loading(4));
        assert_eq!(rows(&app, 80, 24)[0], loading);
    }
    let app = app_with(
        false,
        StatsState::Failed("no history here\nsecond line".into()),
    );
    let all = rows(&app, 80, 24);
    assert_eq!(all[..2], ["no history here", "second line"]);
    let buffer = draw(&app, 80, 24);
    assert_eq!(
        buffer[(1, 1)].style().fg,
        role(Role::Failed, app.palette()).fg
    );
}

/// Scrolled: the first line shown is `scroll`'s, cuts are marked with the kit's marks,
/// and the view never scrolls past its last line.
#[test]
fn stats_render_scrolls_with_marks() {
    // 10 rows: 9 for the body, 7 inside the frame.
    let mut app = ready(false);
    assert_eq!(
        rows(&app, 80, 10),
        [
            READY[0],
            READY[1],
            READY[2],
            READY[3],
            READY[4],
            READY[5],
            "↓ 8 more"
        ]
    );
    screen_mut(&mut app).scroll = 3;
    assert_eq!(
        rows(&app, 80, 10),
        [
            "↑ 3 more",
            READY[3],
            READY[4],
            READY[5],
            READY[6],
            READY[7],
            "↓ 6 more"
        ]
    );
    screen_mut(&mut app).scroll = 13;
    let bottom = [
        "↑ 8 more",
        READY[8],
        READY[9],
        READY[10],
        READY[11],
        READY[12],
        READY[13],
    ];
    assert_eq!(rows(&app, 80, 10), bottom);
    let mut ascii = ready(true);
    screen_mut(&mut ascii).scroll = 3;
    let all = rows(&ascii, 80, 10);
    assert_eq!((all[0].as_str(), all[6].as_str()), ("^ 3 more", "v 6 more"));
}

/// Nothing panics at any small size, in either mode, in any state.
#[test]
fn stats_render_survives_tiny_sizes() {
    for ascii in [false, true] {
        for state in [
            StatsState::Loading(1),
            StatsState::Ready(Box::new(history())),
            StatsState::Failed("x".into()),
        ] {
            let mut app = app_with(ascii, state);
            screen_mut(&mut app).scroll = 9;
            for w in 0..24 {
                for h in 0..8 {
                    draw(&app, w, h);
                }
            }
        }
    }
}

/// Decision 5: a dialog over the screen takes the one accented border.
#[test]
fn a_modal_mutes_the_screens_border() {
    let mut app = ready(false);
    let p = app.palette();
    assert_eq!(
        draw(&app, 80, 24)[(0, 0)].style().fg,
        role(Role::Accent, p).fg
    );
    app.modal = Some(crate::app::Modal::Help);
    assert_eq!(
        draw(&app, 80, 24)[(0, 0)].style().fg,
        role(Role::Muted, p).fg
    );
}

/// Every daemon-supplied string (a class, a flaky test, a problem, the project, a
/// refusal) reaches the spans sanitised.
#[test]
fn stats_screen_text_is_sanitised() {
    let hostile = crate::safe_text::tests::hostile_text();
    let mut stats = history();
    stats.rows[0].class = hostile.clone();
    stats.flaky_proposals[0].test = hostile.clone();
    stats.problems = vec![hostile.clone()];
    let mut seen = String::new();
    for state in [
        StatsState::Ready(Box::new(stats)),
        StatsState::Failed(hostile.clone()),
    ] {
        let mut app = app_with(false, state);
        screen_mut(&mut app).project = hostile.clone().into();
        for (w, h) in SIZES {
            seen.push_str(&screen_text(&app, w, h).replace('\n', " "));
            for line in super::body_lines(&app, screen(&app), w) {
                for span in line.spans {
                    seen.push_str(&span.content);
                }
            }
            seen.push_str(&super::title(screen(&app), app.palette()));
        }
    }
    assert!(seen.contains("a b"), "the text is drawn: {seen}");
    assert_eq!(crate::safe_text::tests::first_hostile(&seen), None);
}
