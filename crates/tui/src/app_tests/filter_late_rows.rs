//! The flaky-races fix (bug 3, smoke stage 11e): a tree filter typed before the rows it
//! matches have arrived. The filter matches nothing, so the selection is cleared; when
//! the matching run or window arrives later, the selection must be restored, or `l`,
//! `j`, `k` and Enter do nothing until the filter is edited again. The ordering is
//! built directly: the filter first, the snapshot after.

use super::runs::{app_with_runs, deliver, run_info, snapshot};
use super::*;
use crate::tree::NodeKey;

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

/// `C-b T`, then `/`, `text` and Enter, as the smoke stage sends them.
fn filter_overview(app: &mut App, text: &str) {
    prefix(app);
    assert!(tap(app, KeyCode::Char('T')).is_empty());
    assert!(app.overview, "C-b T opened the overview");
    tap(app, KeyCode::Char('/'));
    for c in text.chars() {
        tap(app, KeyCode::Char(c));
    }
    tap(app, KeyCode::Enter);
}

#[test]
fn a_run_matching_the_filter_that_arrives_later_is_reachable_with_l() {
    let mut app = app_with_runs(vec![], snapshot(1, 0, vec![]));
    filter_overview(&mut app, "c455");
    assert_eq!(app.tree.selected, None, "the filter matched nothing yet");
    deliver(&mut app, snapshot(2, 0, vec![run_info("run-c455")]));
    assert!(
        app.tree.selected.is_some(),
        "the arriving rows got a selection"
    );
    tap(&mut app, KeyCode::Char('l'));
    tap(&mut app, KeyCode::Char('l'));
    assert!(app.run_view.is_some(), "`l` `l` opened the run view");
}

#[test]
fn a_window_matching_the_filter_that_arrives_later_is_selected() {
    let mut app = app_with(vec![win(1, "other", Status::Idle)]);
    filter_overview(&mut app, "late");
    assert_eq!(app.tree.selected, None, "the filter matched nothing yet");
    let windows = vec![win(1, "other", Status::Idle), win(2, "late", Status::Idle)];
    app.on_daemon(DaemonMsg::WindowsChanged { windows });
    assert_eq!(app.tree.selected, Some(NodeKey::Window(2)));
}
