//! M9.0.7.2: `App::key_region` (decision 1), reached by the keys a user presses.

use super::super::runs::{app_with_runs, open_run_view};
use super::super::*;
use crate::app::region::KeyRegion;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture};

fn chord(app: &mut App, c: char) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char(c), KeyModifiers::NONE)
}

/// The gate fixture, its shell focused.
fn gate_app() -> App {
    let (snap, windows) = gate_fixture();
    app_with_runs(windows, snap)
}

/// One assertion per row of decision 1's table, in the keymap's own precedence.
#[test]
fn key_region_follows_the_keymaps_precedence() {
    // Nothing: the terminal pane has the keys.
    let mut app = gate_app();
    assert_eq!(app.key_region(), KeyRegion::Pane);

    // `C-b t`: the sidebar tree.
    chord(&mut app, 't');
    assert_eq!(app.key_region(), KeyRegion::Sidebar);

    // `C-b T`: the project overview.
    let mut app = gate_app();
    chord(&mut app, 'T');
    assert_eq!(app.key_region(), KeyRegion::Overview);

    // `C-b T`, then a run's `l`: the run view is the overview's frame too.
    let mut app = gate_app();
    open_run_view(&mut app, RUN_ID);
    assert_eq!(app.key_region(), KeyRegion::Overview);

    // `p` at the gate: the plan review, over the run view.
    press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
    assert!(app.plan_review.is_some());
    assert_eq!(app.key_region(), KeyRegion::Review);

    // `C-b m`: the conversation view.
    let mut app = gate_app();
    chord(&mut app, 'm');
    assert!(app.conversation.is_open());
    assert_eq!(app.key_region(), KeyRegion::Conversation);

    // `C-b a`: the alerts.
    let mut app = gate_app();
    chord(&mut app, 'a');
    assert!(app.alerts_focus.is_some());
    assert_eq!(app.key_region(), KeyRegion::Alerts);

    // `C-b P`: a full-body screen.
    let mut app = gate_app();
    chord(&mut app, 'P');
    assert!(app.screen.is_some());
    assert_eq!(app.key_region(), KeyRegion::Screen);

    // `C-b ?` over any of them: the dialog has the keys.
    let opens: [&[char]; 6] = [&[], &['t'], &['T'], &['m'], &['a'], &['P']];
    for keys in opens {
        let mut app = gate_app();
        for &c in keys {
            chord(&mut app, c);
        }
        chord(&mut app, '?');
        assert!(app.modal.is_some(), "help over {keys:?}");
        assert_eq!(app.key_region(), KeyRegion::Dialog, "help over {keys:?}");
    }
    // Over the plan review too.
    let mut app = gate_app();
    open_run_view(&mut app, RUN_ID);
    press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
    chord(&mut app, '?');
    assert_eq!(app.key_region(), KeyRegion::Dialog);
}
