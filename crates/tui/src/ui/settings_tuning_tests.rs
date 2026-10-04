//! Milestone 9.5 decision 48 and ruling RH-5: on opening, the Settings screen asks the
//! daemon for the project's tuning with a read-only `Stats` (which writes nothing), and
//! draws, muted, `refit: <calls> calls <m>m` beside a class's budget when a refit exists
//! and the class is not configured, and `overridden by [orchestrator.routes.orchestrator]`
//! beside the orchestrator default when that list exists. Nothing is drawn with no
//! project, or before the reply.

use super::tests::{opened, sample};
use crate::app::screens::Screen;
use crate::app::settings_screen::SettingsSection;
use crate::app::{App, Effect};
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use crate::ui::audit;
use crate::ui::stats::tests::{history, tuning_report};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Budget, ClientMsg, DaemonMsg, RunReply, RunRequest, TuningReport};

const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];
const OVERRIDDEN: &str = "overridden by [orchestrator.routes.orchestrator]";

/// `C-b S`'s effects.
fn open(app: &mut App) -> Vec<Effect> {
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.on_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))
}

/// Every tagged `Stats` in `effects`, with its id.
fn stats_sent(effects: &[Effect]) -> Vec<(u64, RunRequest)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged { id, request }) => {
                matches!(request, RunRequest::Stats { .. }).then(|| (*id, request.clone()))
            }
            _ => None,
        })
        .collect()
}

/// An app with the settings cache and `default_dir` as its project, the screen closed.
fn cached(default_dir: &str) -> App {
    let mut app = opened(false, sample());
    app.default_dir = default_dir.into();
    app.screen = None;
    app
}

/// The screen opened on `report`'s reply, in `section`.
fn with_report(report: Option<TuningReport>, section: SettingsSection) -> App {
    let mut app = cached("/r/demo");
    let sent = stats_sent(&open(&mut app));
    let (id, _) = sent[0];
    if let Some(report) = report {
        app.on_daemon(DaemonMsg::Run(RunReply::Stats {
            stats: proto::HistoryStats {
                tuning: Some(Box::new(report)),
                ..history()
            },
            request_id: Some(id),
        }));
    }
    match &mut app.screen {
        Some(Screen::Settings(s)) => {
            s.section = section;
            s.selected = 0;
        }
        _ => panic!("no settings screen"),
    }
    app
}

/// The drawn row containing `text`, if any, the row above it, and whether all of
/// `text` is muted.
fn drawn(app: &App, w: u16, h: u16, text: &str) -> Option<(String, String, bool)> {
    let buffer = audit::draw(app, w, h);
    let (x, y) = *audit::find(&buffer, text).first()?;
    let muted = role(Role::Muted, app.palette()).fg;
    let all_muted = (x..x + text.chars().count() as u16)
        .map(|cx| &buffer[(cx, y)])
        .filter(|c| c.symbol() != " ")
        .all(|c| Some(c.fg) == muted);
    let rows = audit::rows(&buffer);
    let (row, above) = (usize::from(y), usize::from(y).saturating_sub(1));
    Some((rows[row].clone(), rows[above].clone(), all_muted))
}

/// S refit and not configured; M configured with a refit; hub refit.
fn refits() -> TuningReport {
    let mut r = tuning_report();
    let b = |tool_calls, minutes| Budget {
        tool_calls,
        minutes,
        tokens: None,
    };
    r.classes[0].configured = false;
    r.classes[0].refit_budget = Some(b(55, 18));
    r.classes[1].configured = true;
    r.classes[1].refit_budget = Some(b(170, 70));
    r.classes[2].refit_budget = Some(b(190, 80));
    r
}

#[test]
fn opening_settings_sends_a_read_only_stats() {
    let mut app = cached("/r/demo");
    let effects = open(&mut app);
    assert_eq!(
        effects.len(),
        1,
        "the one Stats and nothing else: {effects:?}"
    );
    let sent = stats_sent(&effects);
    assert_eq!(
        sent[0].1,
        RunRequest::Stats {
            dir: "/r/demo".into(),
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: true,
        }
    );
    // Already open: nothing more.
    assert!(open(&mut app).is_empty());
    // No project: nothing at all (the cache is read).
    let mut none = cached("");
    assert!(open(&mut none).is_empty());
    assert!(matches!(none.screen, Some(Screen::Settings(_))));
    // The stats screen still sends a plain one (decision 48).
    let mut plain = App::new(vec![], "/r/demo".into(), UiSettings::default());
    let effects = plain.open_stats("/r/demo".into());
    assert_eq!(
        stats_sent(&effects)[0].1,
        RunRequest::Stats {
            dir: "/r/demo".into(),
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: false,
        }
    );
}

#[test]
fn settings_shows_a_muted_refit_beside_an_unconfigured_class() {
    let app = with_report(Some(refits()), SettingsSection::Limits);
    for (w, h) in SIZES {
        let (row, above, muted) = drawn(&app, w, h, "refit: 55 calls 18m").expect("S's refit");
        // Under S's two budget rows, in the value column: a row has no room at 80.
        assert!(
            above.contains("s minutes"),
            "beside S's budget: {above}\n{row}"
        );
        assert_eq!(row.find("refit:"), above.find("15 "), "{above}\n{row}");
        assert!(muted, "muted: {row}");
        // M is configured: its refit is not drawn; hub has no Settings budget.
        assert_eq!(drawn(&app, w, h, "refit: 170"), None);
        assert_eq!(drawn(&app, w, h, "refit: 190"), None);
        assert_eq!(
            audit::rows(&audit::draw(&app, w, h))
                .iter()
                .filter(|r| r.contains("refit:"))
                .count(),
            1
        );
    }
    // Before the reply, and with no project, nothing.
    let waiting = with_report(None, SettingsSection::Limits);
    assert_eq!(drawn(&waiting, 120, 40, "refit:"), None);
    let mut none = cached("");
    open(&mut none);
    if let Some(Screen::Settings(s)) = &mut none.screen {
        s.section = SettingsSection::Limits;
    }
    assert_eq!(drawn(&none, 120, 40, "refit:"), None);
}

#[test]
fn settings_shows_the_orchestrator_list_override() {
    let mut report = tuning_report();
    report.orchestrator_list = Some("claude/claude-opus-5-5 high".into());
    let app = with_report(Some(report), SettingsSection::Orchestrator);
    for (w, h) in SIZES {
        let (row, above, muted) = drawn(&app, w, h, OVERRIDDEN).expect("the override note");
        // Under the default's two rows, in the value column.
        assert!(
            above.contains("model"),
            "beside the default: {above}\n{row}"
        );
        assert_eq!(row.find("overridden"), above.find("‹ "), "{above}\n{row}");
        assert!(muted, "muted: {row}");
    }
    let plain = with_report(Some(tuning_report()), SettingsSection::Orchestrator);
    assert_eq!(drawn(&plain, 120, 40, "overridden"), None);
    let waiting = with_report(None, SettingsSection::Orchestrator);
    assert_eq!(drawn(&waiting, 120, 40, "overridden"), None);
}

/// A reply to the read-only `Stats` never opens or changes the stats screen, and the
/// notes pass the render audit's ASCII check.
#[test]
fn the_notes_render_in_ascii_and_leave_the_stats_screen_alone() {
    let mut app = with_report(Some(refits()), SettingsSection::Limits);
    assert!(matches!(app.screen, Some(Screen::Settings(_))));
    app.settings.badges.ascii = true;
    app.settings.badges =
        crate::ui::badge::BadgeSet::from_config(&app.settings.badges_config, true);
    for (w, h) in SIZES {
        assert_eq!(audit::first_non_ascii(&audit::draw(&app, w, h)), None);
    }
}

/// The note is not a row the keys select: the selection still lands on its own limit.
#[test]
fn the_refit_note_leaves_the_selection_on_its_limit() {
    let mut refits_m = refits();
    refits_m.classes[1].configured = false;
    for (selected, label) in [(2, "m tool calls"), (9, "max bounces")] {
        let mut app = with_report(Some(refits_m.clone()), SettingsSection::Limits);
        if let Some(Screen::Settings(s)) = &mut app.screen {
            s.selected = selected;
        }
        // Short screens scroll the section to keep the selected limit in view.
        for h in 12..=24 {
            let rows = audit::rows(&audit::draw(&app, 80, h));
            let shown: Vec<&String> = rows.iter().filter(|r| r.contains('▌')).collect();
            assert_eq!(shown.len(), 1, "80x{h}: {rows:#?}");
            assert!(shown[0].contains(label), "80x{h}: {rows:#?}");
        }
    }
}

/// Fix round 1 (review m6): the screen's own save can make S's budget explicit
/// (decision 3), so its note goes at once and the tuning is asked again; the new
/// answer decides.
#[test]
fn a_save_asks_the_tuning_again() {
    let mut app = with_report(Some(refits()), SettingsSection::Limits);
    assert!(drawn(&app, 120, 40, "refit: 55").is_some());
    if let Some(Screen::Settings(s)) = &mut app.screen {
        s.limits[0].text = "41".into();
    }
    let effects = app.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    let put = effects
        .iter()
        .find_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged {
                id,
                request: RunRequest::Settings(proto::SettingsRequest::Put { settings }),
            }) => Some((*id, settings.clone())),
            _ => None,
        })
        .expect("w sends the Put");
    assert!(
        stats_sent(&effects).is_empty(),
        "no Stats before the save lands"
    );
    let after = app.on_daemon(DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(proto::SettingsReply::Saved {
            doc: put.1,
            origin: Default::default(),
        }),
        request_id: Some(put.0),
    }));
    let asked = stats_sent(&after);
    assert_eq!(asked.len(), 1, "{after:?}");
    assert!(matches!(
        asked[0].1,
        RunRequest::Stats {
            read_only: true,
            ..
        }
    ));
    assert_eq!(drawn(&app, 120, 40, "refit:"), None, "no stale note");
    let mut configured = refits();
    configured.classes[0].configured = true;
    app.on_daemon(DaemonMsg::Run(RunReply::Stats {
        stats: proto::HistoryStats {
            tuning: Some(Box::new(configured)),
            ..history()
        },
        request_id: Some(asked[0].0),
    }));
    assert!(matches!(app.screen, Some(Screen::Settings(_))));
    assert_eq!(
        drawn(&app, 120, 40, "refit: 55"),
        None,
        "S is configured now"
    );
}
