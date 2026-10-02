//! Final fix wave (task 5's deferred minor): a frame builds the alerts once and shares
//! them between the sidebar's sizing, the box, the Alerts view and the status bar's
//! flag.

use crate::app::alerts::built;
use crate::ui::audit;

fn key(app: &mut crate::app::App, ctrl: bool, c: char) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mods = if ctrl {
        KeyModifiers::CONTROL
    } else {
        KeyModifiers::NONE
    };
    app.on_key(KeyEvent::new(KeyCode::Char(c), mods));
}

#[test]
fn a_frame_builds_the_alerts_once() {
    let boxed = super::fixture::three_runs();
    let mut hidden = super::fixture::three_runs();
    hidden.sidebar_visible = false;
    let mut view = super::fixture::three_runs();
    key(&mut view, true, 'b');
    key(&mut view, false, 'a');
    assert!(view.alerts_focus.is_some());
    view.sidebar_visible = false;
    for (name, app) in [("box", boxed), ("flag", hidden), ("view", view)] {
        for (w, h) in [(80, 24), (120, 40)] {
            let _ = built::take();
            let _ = audit::draw(&app, w, h);
            assert_eq!(built::take(), 1, "{name} at {w}x{h}");
        }
    }
}
