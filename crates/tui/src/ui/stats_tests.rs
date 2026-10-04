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
        rounds: 0,
        iterated_runs: 0,
        tuning: None,
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
        request: 4,
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

const READY: [&str; 16] = [
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
    "",
    "history: /r/demo",
];

/// Decision 38's rows, exactly: the table (a `None` median reads `–`), the deciders
/// line, the flaky proposals, the problems and the history file, at 80×24 and 120×40,
/// under the title.
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
    }
}

/// The CLI's units (`daemon/src/run/stats.rs`): tokens `1.2k` under 10k, work in
/// minutes rounded to the nearest.
#[test]
fn stats_cells_use_the_clis_units() {
    let cases = [
        (999, "999"),
        (1000, "1.0k"),
        (1250, "1.2k"),
        (9999, "9.9k"),
        (10_000, "10k"),
        (2_345_678, "2.3M"),
    ];
    for (n, want) in cases {
        assert_eq!(super::tokens(n), want, "{n}");
    }
    for (secs, want) in [
        (0, "0m"),
        (29, "0m"),
        (30, "1m"),
        (380, "6m"),
        (3599, "60m"),
    ] {
        assert_eq!(super::work(secs), want, "{secs}");
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
    assert_eq!(all[8..10], ["", "history: /r/demo"]);
    assert!(all[10..].iter().all(String::is_empty), "{all:#?}");
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
/// and the view never scrolls past its last line, which is `max_scroll`, the bound the
/// screen's keys use: every scroll up to it draws a different view.
#[test]
fn stats_render_scrolls_with_marks() {
    // 10 rows: 9 for the body, 7 inside the frame.
    let body = ratatui::layout::Rect::new(0, 0, 80, 9);
    let mut app = ready(false);
    let r = |i: usize| READY[i].to_string();
    let top: Vec<String> = (0..6).map(r).chain(["↓ 10 more".into()]).collect();
    assert_eq!(rows(&app, 80, 10), top);
    screen_mut(&mut app).scroll = 3;
    let mid: Vec<String> = ["↑ 3 more".into()]
        .into_iter()
        .chain((3..8).map(r))
        .chain(["↓ 8 more".into()])
        .collect();
    assert_eq!(rows(&app, 80, 10), mid);
    let max = super::max_scroll(&app, screen(&app), body);
    assert_eq!(max, 10);
    let bottom: Vec<String> = ["↑ 10 more".into()]
        .into_iter()
        .chain((10..16).map(r))
        .collect();
    screen_mut(&mut app).scroll = max;
    assert_eq!(rows(&app, 80, 10), bottom);
    screen_mut(&mut app).scroll = max + 3;
    assert_eq!(
        rows(&app, 80, 10),
        bottom,
        "past the bound draws the bottom"
    );
    let mut views = Vec::new();
    for scroll in 0..=max {
        screen_mut(&mut app).scroll = scroll;
        views.push(rows(&app, 80, 10));
    }
    views.dedup();
    assert_eq!(
        views.len(),
        max + 1,
        "no scroll up to the bound is a dead press"
    );
    let mut ascii = ready(true);
    screen_mut(&mut ascii).scroll = 3;
    let all = rows(&ascii, 80, 10);
    assert_eq!((all[0].as_str(), all[6].as_str()), ("^ 3 more", "v 8 more"));
}

/// M9.0.7.12 fix round 1: the help's own limit (`kit::from_top_until`) leaves the stats
/// screen's end where it was. A scroll past the bound draws the last page, its last
/// line on the frame's last interior row, at 80×10 and at 80×14.
#[test]
fn the_stats_scroll_still_stops_at_its_last_page() {
    for h in [10u16, 14] {
        let body = ratatui::layout::Rect::new(0, 0, 80, h - 1);
        let mut app = ready(false);
        let max = super::max_scroll(&app, screen(&app), body);
        assert_eq!(max, READY.len() - usize::from(h - 4), "{h} rows");
        screen_mut(&mut app).scroll = max;
        let bottom = rows(&app, 80, h);
        screen_mut(&mut app).scroll = usize::MAX;
        assert_eq!(rows(&app, 80, h), bottom, "{h} rows");
        assert_eq!(bottom.last().map(String::as_str), READY.last().copied());
    }
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
    app.modal = Some(crate::app::Modal::Help(Default::default()));
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
    stats.path = hostile.clone().into();
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

fn class(
    name: &str,
    samples: u32,
    (calls, minutes): (u32, u32),
    refit: proto::RefitState,
    weight: (u64, bool),
) -> proto::ClassTuning {
    proto::ClassTuning {
        class: name.into(),
        samples,
        budget: proto::Budget {
            tool_calls: calls,
            minutes,
            tokens: None,
        },
        refit,
        configured: false,
        refit_budget: None,
        weight_secs: Some(weight.0),
        weight_derived: weight.1,
        route: "standard/medium".into(),
    }
}

/// Milestone 9.5 decision 11's report for `refit.jsonl` with S configured (ruling
/// RH-5): its `budget S: configured …` line, the derived weights' `*` note, one
/// proposal. The Settings and audit tests share it.
pub(crate) fn tuning_report() -> proto::TuningReport {
    use proto::RefitState::{Configured, NotYet};
    let mut s = class("S", 34, (40, 15), Configured, (540, false));
    s.configured = true;
    s.refit_budget = Some(proto::Budget {
        tool_calls: 55,
        minutes: 18,
        tokens: None,
    });
    proto::TuningReport {
        path: "/r/demo/tuning.toml".into(),
        min_samples: 30,
        refit_budgets: true,
        classes: vec![
            s,
            class("M", 12, (150, 60), NotYet, (1620, true)),
            class("hub", 3, (150, 60), NotYet, (1620, true)),
        ],
        proposals: vec![proto::TuningProposal {
            id: "thresholds.s".into(),
            text: "S line threshold 20 → 35 (p90 of 34 merged S tasks)".into(),
            current: "20".into(),
            proposed: "35".into(),
            change: proto::TuningChange::Threshold {
                class: "s".into(),
                lines: 35,
            },
        }],
        moved_bad_file: None,
        applied: Vec::new(),
        dismissed: Vec::new(),
        orchestrator_list: None,
        parse_error: None,
        project: None,
    }
}

/// `history()` with [`tuning_report`].
pub(crate) fn history_with_tuning() -> HistoryStats {
    HistoryStats {
        tuning: Some(Box::new(tuning_report())),
        ..history()
    }
}

/// Milestone 9.5 decision 48 and ruling RH-6: after the flaky block, the lines
/// `anthrex run stats` prints of the tuning block, its `*` note and `budget S:
/// configured …` line muted, and the hint `apply: anthrex run stats --apply <id>` in
/// place of the CLI's `apply with …` line; no new key, hint or action.
#[test]
fn the_stats_screen_shows_the_tuning_block_read_only() {
    let block = [
        "tuning: /r/demo/tuning.toml  (refit after 30 samples per class)",
        "  CLASS  SAMPLES  BUDGET          WEIGHT  REFIT",
        "  S      34       40 calls 15m    9m      configured",
        "  M      12/30    150 calls 60m   27m*    not yet",
        "  hub    3/30     150 calls 60m   27m*    not yet",
        "  * derived from another class's median",
        "  budget S: configured 40 calls 15m (refit would be 55 calls 18m)",
        "tuning proposals:",
        "  thresholds.s  S line threshold 20 → 35 (p90 of 34 merged S tasks)",
        "apply: anthrex run stats --apply <id>",
    ];
    for ascii in [false, true] {
        let mut app = app_with(ascii, StatsState::Ready(Box::new(history_with_tuning())));
        let p = app.palette();
        let lines = super::body_lines(&app, screen(&app), 120);
        let texts: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        let flaky = texts
            .iter()
            .position(|t| t == "  api::slow_ping        1 run");
        let at = flaky.expect("the flaky block") + 2;
        let want: Vec<String> = block
            .iter()
            .map(|t| {
                if ascii {
                    t.replace('→', "->")
                } else {
                    t.to_string()
                }
            })
            .collect();
        assert_eq!(texts[at - 1], "", "a blank line before the block");
        assert_eq!(texts[at..at + block.len()], want, "ascii {ascii}");
        assert_eq!(texts[at + block.len()], "", "then the problems");
        assert!(!texts.iter().any(|t| t.contains("apply with")), "{texts:?}");
        assert!(!texts.iter().any(|t| t.contains("dismiss")), "{texts:?}");
        let muted = role(Role::Muted, p);
        for (i, line) in lines[at..at + block.len()].iter().enumerate() {
            let is_muted = line.style == muted;
            assert_eq!(is_muted, i == 5 || i == 6, "{:?}", texts[at + i]);
        }
        // Read-only: the same hints, and no key acts on a proposal.
        let words: Vec<String> = super::hints().into_iter().map(|h| h.word).collect();
        assert_eq!(words, ["scroll", "page", "back"]);
        let before = app.screen.clone();
        for key in ['a', 'd', 'y', 'x'] {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(key),
                crossterm::event::KeyModifiers::NONE,
            );
            assert!(app.on_key(key).is_empty(), "{key:?} sends nothing");
        }
        assert_eq!(app.screen, before);
        if ascii {
            for (w, h) in SIZES {
                assert_eq!(crate::ui::audit::first_non_ascii(&draw(&app, w, h)), None);
            }
        }
    }
}

/// Whole-branch review D, M-3: at 80 columns a long tuning line wraps under itself, so
/// a route proposal's evidence is never cut off; a line that fits is kept as printed.
#[test]
fn a_long_tuning_line_wraps_at_80_columns() {
    let text =
        "S route standard/low → standard/medium (14 of 34 S tasks, 41%, reached rung 2 or higher)";
    let mut report = tuning_report();
    report.proposals[0].id = "route.s".into();
    report.proposals[0].text = text.into();
    let stats = HistoryStats {
        tuning: Some(Box::new(report)),
        ..history()
    };
    let app = app_with(false, StatsState::Ready(Box::new(stats)));
    let texts: Vec<String> = (super::body_lines(&app, screen(&app), 80).iter())
        .map(|l| l.to_string())
        .collect();
    let at = texts
        .iter()
        .position(|t| t == "tuning proposals:")
        .expect("the block");
    let end = at
        + texts[at..]
            .iter()
            .position(|t| t.starts_with("apply: "))
            .expect("the hint");
    let proposal = &texts[at + 1..end];
    assert!(proposal.len() >= 2, "wrapped: {proposal:?}");
    assert!(
        proposal
            .iter()
            .all(|t| unicode_width::UnicodeWidthStr::width(t.as_str()) <= 80),
        "{proposal:?}"
    );
    assert!(
        proposal[0].starts_with("  route.s ") && proposal[0].contains(" S route "),
        "{proposal:?}"
    );
    for continuation in &proposal[1..] {
        assert!(continuation.starts_with("      "), "indented: {proposal:?}");
    }
    let words: Vec<&str> = proposal.iter().flat_map(|t| t.split_whitespace()).collect();
    let want: Vec<&str> = format!("route.s {text}")
        .leak()
        .split_whitespace()
        .collect();
    assert_eq!(words, want, "nothing is cut");
    // A line that fits keeps its columns.
    assert!(texts.contains(&"  S      34       40 calls 15m    9m      configured".to_string()));
}
