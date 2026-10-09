//! Milestone 9.0.6 task 14: the Settings screen, drawn (decision 36).

use crate::app::App;
use crate::app::screens::Screen;
use crate::app::settings_screen::{SaveOutcome, SettingsPage, SettingsScreen, SettingsSection};
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::settings::key;
use proto::{BudgetLimit, DaemonMsg, Origin, RunReply, SettingsDoc, SettingsLimits, SettingsReply};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use std::collections::BTreeMap;

const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];

pub(crate) fn sample() -> SettingsDoc {
    let budget = |tool_calls, minutes| BudgetLimit {
        tool_calls,
        minutes,
    };
    SettingsDoc {
        limits: SettingsLimits {
            budget_s: budget(40, 15),
            budget_m: budget(80, 30),
            budget_l: budget(5, 60),
            stall_after_secs: 600,
            max_writers: 2,
            max_readers: 4,
            max_bounces: 3,
        },
        design_default: None,
        roles: Default::default(),
    }
}

/// `models` and `max_bounces` from the defaults.
fn origin() -> BTreeMap<String, Origin> {
    let defaults = [key::ROLES, key::MAX_BOUNCES];
    proto::SETTINGS_KEYS
        .iter()
        .map(|k| {
            let o = if defaults.contains(k) {
                Origin::Default
            } else {
                Origin::File
            };
            (k.to_string(), o)
        })
        .collect()
}

/// The sample, open on the Settings screen.
pub(crate) fn opened(ascii: bool, doc: SettingsDoc) -> App {
    let mut settings = UiSettings::default();
    settings.badges.ascii = ascii;
    let mut app = App::new(vec![], "/tmp".into(), settings);
    app.set_terminal_size(80, 24);
    let id = match app.settings_fetch() {
        crate::app::Effect::Send(proto::ClientMsg::RunTagged { id, .. }) => id,
        other => panic!("{other:?}"),
    };
    app.on_daemon(DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(SettingsReply::Current {
            doc,
            origin: origin(),
            path: "/cfg/config.toml".into(),
        }),
        request_id: Some(id),
    }));
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.on_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT));
    assert!(matches!(app.screen, Some(Screen::Settings(_))));
    app
}

pub(crate) fn screen_mut(app: &mut App) -> &mut SettingsScreen {
    match &mut app.screen {
        Some(Screen::Settings(s)) => s,
        _ => panic!("no settings screen"),
    }
}

pub(crate) fn draw(app: &App, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

/// What a frame shows, one line per row.
pub(crate) fn screen_text(app: &App, w: u16, h: u16) -> String {
    let buffer = draw(app, w, h);
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
        .collect()
}

/// The screen's interior rows (inside the frame), trailing spaces trimmed.
pub(crate) fn rows(app: &App, w: u16, h: u16) -> Vec<String> {
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

fn section(app: &mut App, s: SettingsSection) {
    screen_mut(app).section = s;
    screen_mut(app).selected = 0;
}

/// Rows `1..` of the interior, cut to the section's lines (the footer is checked apart).
pub(crate) fn head(app: &App, w: u16, h: u16, n: usize) -> Vec<String> {
    rows(app, w, h).into_iter().take(n).collect()
}

/// The interior's last `n` rows.
fn tail(app: &App, w: u16, h: u16, n: usize) -> Vec<String> {
    let all = rows(app, w, h);
    all[all.len() - n..].to_vec()
}

/// Milestone 9.8: the screen opens on `models`, drawn whole as the spec's table
/// (`ui/models_table_tests.rs` checks its rows), and the status bar keeps the
/// section's hints.
#[test]
fn models_section_renders_at_80x24_and_120x40() {
    let mut app = opened(false, sample());
    app.catalogs = crate::app::model_picker::tests::fixture_catalogs();
    for (w, h) in SIZES {
        let text = screen_text(&app, w, h);
        assert!(
            text.starts_with("┌ settings · /cfg/config.toml ─"),
            "{text}"
        );
        let all = rows(&app, w, h);
        assert!(
            all[0].starts_with(" models · scope ‹ everywhere › / this repo (tmp)"),
            "{w}x{h}: {all:?}"
        );
        assert!(all[0].ends_with("tab: limits"), "{w}x{h}");
        assert!(
            all[3].starts_with("▸orchestrator          Claude · claude-opus-5-5 high     —"),
            "{w}x{h}: {all:?}"
        );
        // Only what blocks `w` shows here (M9.8.12 removed the roster's warnings).
        assert!(
            all[13..all.len() - 1].iter().all(|r| r.is_empty()),
            "{w}x{h}: {all:?}"
        );
        assert_eq!(
            all.last().unwrap(),
            " ⏎ choose model   e effort   f if-it-struggles   x reset   w save   esc back"
        );
        let bar = text.lines().last().unwrap().trim_end().to_string();
        assert!(bar.contains("⏎ choose model"), "{bar}");
        assert!(bar.contains("esc back"), "{bar}");
    }
}

#[test]
fn limits_section_renders_at_80x24_and_120x40() {
    let mut app = opened(false, sample());
    section(&mut app, SettingsSection::Limits);
    for (w, h) in SIZES {
        assert_eq!(
            head(&app, w, h, 13),
            vec![
                "models  limits",
                "",
                "▌ s tool calls      40            at least 1   hard stop at 60 calls",
                "  s minutes         15            at least 1   hard stop at 22m30s",
                "  m tool calls      80            at least 1   hard stop at 120 calls",
                "  m minutes         30            at least 1   hard stop at 45m",
                "  l tool calls      5             at least 1   hard stop at 8 calls",
                "  l minutes         60            at least 1   hard stop at 90m",
                "  stall after secs  600           5 to 7200",
                "  max writers       2             1 to 8",
                "  max readers       4             1 to 8",
                "  max bounces       3  (default)  1 to 5",
                "",
            ],
            "{w}x{h}"
        );
        // M9.8.12: the roster's strength warnings went with the roster.
        assert!(
            !screen_text(&app, w, h).contains("cross-runtime reviewer"),
            "{w}x{h}"
        );
    }
    // Out of range: the value in `Failed`, config's own message above the hints.
    let at = screen_mut(&mut app)
        .limits
        .iter()
        .position(|f| f.key == key::MAX_WRITERS)
        .unwrap();
    screen_mut(&mut app).limits[at].text = "9".into();
    let failed = role(Role::Failed, app.palette()).fg;
    let buffer = draw(&app, 80, 24);
    assert_eq!(buffer[(21, 1 + 9)].symbol(), "9");
    assert_eq!(buffer[(21, 1 + 9)].fg, failed.unwrap());
    let text = screen_text(&app, 80, 24);
    assert!(
        text.contains("✗ orchestrator.max_writers: must be between 1 and 8"),
        "{text}"
    );
    assert!(
        text.contains("  max writers       9             1 to 8"),
        "{text}"
    );
}

#[test]
fn ascii_mode_draws_only_ascii() {
    let mut app = opened(true, sample());
    app.catalogs = crate::app::model_picker::tests::fixture_catalogs();
    let mut seen = String::new();
    for s in SettingsSection::ALL {
        section(&mut app, s);
        for (w, h) in SIZES {
            let text = screen_text(&app, w, h);
            assert!(text.is_ascii(), "{s:?} {w}x{h}\n{text}");
            seen.push_str(&text);
        }
    }
    for want in [
        "+ settings - /cfg/config.toml -",
        " models - scope < everywhere > / this repo (tmp)",
        ">orchestrator          Claude - claude-opus-5-5 high     -",
        " implementer - small   Claude - Sonnet         low      -",
        " helpers >             Claude - claude-haiku-4-5 -        -",
        " enter choose model   e effort",
        "enter choose model",
    ] {
        assert!(seen.contains(want), "{want}\n{seen}");
    }
    screen_mut(&mut app).page = None;
    screen_mut(&mut app).outcome = Some(SaveOutcome::Saved);
    section(&mut app, SettingsSection::Models);
    let text = screen_text(&app, 120, 40);
    assert!(
        text.contains("saved - new runs use these models - runs in progress keep theirs"),
        "{text}"
    );
    // The picker in ASCII.
    let catalogs = app.catalogs.clone();
    screen_mut(&mut app).models.open_picker(&catalogs, false);
    let text = screen_text(&app, 120, 40);
    assert!(text.is_ascii(), "{text}");
    for want in [
        "+ choose model - orchestrator -",
        "> * claude-opus-5-5  as Opus (1M context)                  low ... max",
        "    custom...   type any model name",
        " j/k move  enter select  r refresh  esc cancel",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    screen_mut(&mut app).section = SettingsSection::Limits;
    let text = screen_text(&app, 120, 40);
    assert!(
        text.contains("saved - new runs use these settings - runs in progress keep theirs"),
        "{text}"
    );
}

#[test]
fn the_pages_render_as_kit_dialogs() {
    let mut app = opened(false, sample());
    screen_mut(&mut app).page = Some(SettingsPage::Discard);
    let failed = role(Role::Failed, app.palette()).fg.unwrap();
    for (w, h) in SIZES {
        let text = screen_text(&app, w, h);
        assert!(text.contains("│ discard unsaved settings? y"), "{text}");
        assert!(text.contains("│ y discard · esc back"), "{text}");
        let buffer = draw(&app, w, h);
        let (x, y) = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .find(|&(x, y)| {
                buffer[(x, y)].symbol() == "d"
                    && (x..x + 15)
                        .map(|x| buffer[(x.min(w - 1), y)].symbol())
                        .collect::<String>()
                        == "discard changes"
            })
            .unwrap();
        assert_eq!(buffer[(x, y)].fg, failed, "the destructive title");
        // The action word is in `Failed` too (decision 5).
        let hint_y = (0..h)
            .find(|&y| {
                (0..w)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .contains("y discard · esc back")
            })
            .unwrap();
        let x = (0..w)
            .find(|&x| buffer[(x, hint_y)].symbol() == "y")
            .unwrap();
        for dx in [0, 2, 8] {
            assert_eq!(buffer[(x + dx, hint_y)].fg, failed, "{w}x{h} +{dx}");
        }
    }
}

#[test]
fn problems_warnings_and_outcomes_render() {
    let mut app = opened(false, sample());
    // Milestone 9.8 (M9.8.12): a row the daemon's own rules refuse blocks `w`, and the
    // roster's strength warnings are gone from every section with the roster.
    let mut bad = sample();
    bad.roles.rows.insert(
        proto::models::Role::Planner,
        proto::models::RoleChoice {
            model: proto::models::ModelRef::parse("codex:default").unwrap(),
            effort: Some("Not An Effort".into()),
            fallback: None,
        },
    );
    screen_mut(&mut app).load(&bad, &origin());
    let text = screen_text(&app, 80, 24);
    assert!(
        text.contains("✗ models.planner.effort: \"Not An Effort\" is not an effort name"),
        "{text}"
    );
    for s in SettingsSection::ALL {
        section(&mut app, s);
        let text = screen_text(&app, 120, 40);
        assert!(!text.contains("cross-runtime reviewer"), "{s:?}\n{text}");
    }
    section(&mut app, SettingsSection::Models);
    let s = screen_mut(&mut app);
    s.load(&sample(), &origin());
    s.outcome = Some(SaveOutcome::Refused(vec!["one".into(), "two".into()]));
    // On `models`, the outcome sits above the keys.
    let t = tail(&app, 120, 40, 4);
    assert_eq!(t[..3], ["not saved", "✗ one", "✗ two"]);
    screen_mut(&mut app).outcome = Some(SaveOutcome::Saved);
    let t = tail(&app, 120, 40, 2);
    assert_eq!(
        t[0],
        "saved · new runs use these models · runs in progress keep theirs"
    );
    let done = role(Role::Done, app.palette()).fg.unwrap();
    assert_eq!(draw(&app, 120, 40)[(1, 36)].fg, done);
    section(&mut app, SettingsSection::Limits);
    let t = tail(&app, 120, 40, 1);
    assert_eq!(
        t,
        vec!["saved · new runs use these settings · runs in progress keep theirs"]
    );
    assert_eq!(draw(&app, 120, 40)[(1, 37)].fg, done);
}

/// Decision 5 and preflight F23: one accented border, the region with the keys: the
/// screen's frame alone, else the dialog's (the discard question, or the action menu over
/// the screen) and none of the screen's.
#[test]
fn one_accented_border() {
    use crate::actions_request::ActionTarget;
    let (snap, windows) = crate::tree::run_fixtures::three_task_fixture();
    let run_id = snap.runs[0].run_id.clone();
    let mut states = Vec::new();
    for state in [0, 2, 3] {
        let mut app = opened(false, sample());
        app.windows = windows.clone();
        app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap.clone())));
        match state {
            2 => screen_mut(&mut app).page = Some(SettingsPage::Discard),
            3 => {
                app.open_actions((run_id.clone(), ActionTarget::Run), None);
                assert!(matches!(app.modal, Some(crate::app::Modal::Action(_))));
            }
            _ => {}
        }
        states.push(app);
    }
    for (i, app) in states.iter().enumerate() {
        for (w, h) in SIZES {
            let buffer = draw(app, w, h);
            let accent = role(Role::Accent, app.palette()).fg.unwrap();
            let cells: Vec<(u16, u16)> = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let c = &buffer[(x, y)];
                    c.fg == accent && "─│┌┐└┘".contains(c.symbol()) && !c.symbol().is_empty()
                })
                .collect();
            let x0 = cells.iter().map(|c| c.0).min().unwrap();
            let x1 = cells.iter().map(|c| c.0).max().unwrap();
            let y0 = cells.iter().map(|c| c.1).min().unwrap();
            let y1 = cells.iter().map(|c| c.1).max().unwrap();
            for &(x, y) in &cells {
                assert!(
                    x == x0 || x == x1 || y == y0 || y == y1,
                    "state {i} {w}x{h}: ({x},{y}) is inside the box"
                );
            }
            // A whole box: its corners and both sides are accented (the top edge
            // carries the title).
            for (x, y, corner) in [(x0, y0, "┌"), (x1, y0, "┐"), (x0, y1, "└"), (x1, y1, "┘")]
            {
                assert_eq!(buffer[(x, y)].symbol(), corner, "state {i} {w}x{h}");
            }
            for y in y0..=y1 {
                assert_eq!(buffer[(x0, y)].fg, accent, "state {i} {w}x{h} left {y}");
                assert_eq!(buffer[(x1, y)].fg, accent, "state {i} {w}x{h} right {y}");
            }
            if i == 0 {
                assert_eq!((x0, y0, x1, y1), (0, 0, w - 1, h - 2), "the screen's frame");
            } else {
                assert!(x1 - x0 < 64, "state {i} {w}x{h}: only the dialog's frame");
            }
        }
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    let mut app = opened(false, sample());
    let mut loading = opened(false, sample());
    *screen_mut(&mut loading) = SettingsScreen::loading();
    for s in SettingsSection::ALL {
        section(&mut app, s);
        for (page, picker) in [
            (None, false),
            (None, true),
            (Some(SettingsPage::Discard), false),
        ] {
            screen_mut(&mut app).page = page;
            screen_mut(&mut app).models.picker = None;
            if picker {
                let catalogs = app.catalogs.clone();
                screen_mut(&mut app).models.open_picker(&catalogs, false);
            }
            for (w, h) in [(20, 5), (1, 1), (64, 3), (5, 40), (80, 4)] {
                draw(&app, w, h);
                draw(&loading, w, h);
            }
        }
    }
    assert!(screen_text(&loading, 80, 24).contains("loading…"));
}

#[test]
fn settings_screen_text_is_sanitised() {
    let hostile = crate::safe_text::tests::hostile_text();
    let doc = sample();
    let mut app = opened(false, doc);
    // Milestone 9.8: a catalog's label, description and efforts, in the table and the
    // picker.
    let mut catalogs = crate::app::model_picker::tests::fixture_catalogs();
    for m in &mut catalogs.list[0].models {
        m.label = hostile.clone();
        m.description = hostile.clone();
        m.efforts.push(hostile.clone());
    }
    catalogs.list[0].cli_version = hostile.clone();
    app.catalogs = catalogs;
    screen_mut(&mut app).path = hostile.clone();
    screen_mut(&mut app).outcome = Some(SaveOutcome::Refused(vec![hostile.clone()]));
    let mut seen = String::new();
    for s in SettingsSection::ALL {
        section(&mut app, s);
        for (w, h) in SIZES {
            seen.push_str(&screen_text(&app, w, h).replace('\n', " "));
            let Some(Screen::Settings(screen)) = &app.screen else {
                unreachable!()
            };
            for line in super::body_lines(&app, screen, w) {
                for span in line.spans {
                    seen.push_str(&span.content);
                }
            }
        }
    }
    for line in super::page_lines(&app, &SettingsPage::Discard, 60) {
        for span in line.spans {
            seen.push_str(&span.content);
        }
    }
    section(&mut app, SettingsSection::Models);
    let catalogs = app.catalogs.clone();
    screen_mut(&mut app).models.open_picker(&catalogs, false);
    for (w, h) in SIZES {
        seen.push_str(&screen_text(&app, w, h).replace('\n', " "));
    }
    assert_eq!(crate::safe_text::tests::first_hostile(&seen), None);
}
