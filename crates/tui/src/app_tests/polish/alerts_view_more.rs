//! The final fix wave's Alerts view reducer tests: M2 (Enter on an empty view) and
//! task 6's deferred minor (the screens over the view give it back; `C-b <`, `>` and
//! `r` work under it).

use super::super::*;
use crate::app::AlertKey;
use crate::app::region::KeyRegion;
use crate::ui::alerts::fixture::three_runs;

fn chord(app: &mut App, c: char) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char(c), KeyModifiers::NONE)
}

fn selected(app: &App) -> Option<AlertKey> {
    app.alerts_focus.as_ref().and_then(|f| f.selected.clone())
}

/// M2 (decision 11): with no alerts the view shows `no alerts` and takes Esc only;
/// Enter leaves it open.
#[test]
fn enter_on_an_empty_view_does_nothing() {
    let mut app = App::new(
        vec![crate::tree::run_fixtures::pty(
            1,
            "shell",
            "/tmp",
            proto::Status::Idle,
        )],
        "/tmp".into(),
        Default::default(),
    );
    assert!(crate::app::alerts(&app).is_empty());
    assert!(chord(&mut app, 'a').is_empty());
    assert_eq!(app.key_region(), KeyRegion::Alerts);
    assert_eq!(selected(&app), None);
    for code in [KeyCode::Enter, KeyCode::Char('o'), KeyCode::Char('.')] {
        assert!(press(&mut app, code, KeyModifiers::NONE).is_empty());
        assert_eq!(app.key_region(), KeyRegion::Alerts, "{code:?}");
    }
    assert!(press(&mut app, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert_eq!(app.alerts_focus, None);
}

/// Task 6's deferred minor: `C-b P` and `C-b S` open their screens over the view, and
/// Esc gives the view back with its selection.
#[test]
fn a_screen_over_the_view_gives_it_back() {
    for screen in ['P', 'S'] {
        let mut app = three_runs();
        let _ = chord(&mut app, 'a');
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        let before = app.alerts_focus.clone();
        assert!(selected(&app).is_some());
        let _ = chord(&mut app, screen);
        assert_eq!(app.key_region(), KeyRegion::Screen, "C-b {screen}");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(app.screen, None, "C-b {screen}");
        assert_eq!(app.key_region(), KeyRegion::Alerts, "C-b {screen}");
        assert_eq!(app.alerts_focus, before, "C-b {screen}");
    }
}

/// Task 6's deferred minor: the sidebar's width and the reconnect keep working under
/// the view, which stays open.
#[test]
fn width_and_reconnect_work_under_the_view() {
    let mut app = three_runs();
    let _ = chord(&mut app, 'a');
    let width = app.sidebar_width;
    let _ = chord(&mut app, '>');
    assert!(app.sidebar_width > width, "C-b > widens");
    let _ = chord(&mut app, '<');
    assert_eq!(app.sidebar_width, width, "C-b < narrows");
    app.link = crate::app::Link::Lost {
        reason: "gone".into(),
    };
    assert_eq!(chord(&mut app, 'r'), vec![Effect::Reconnect]);
    assert_eq!(app.key_region(), KeyRegion::Alerts);
}
