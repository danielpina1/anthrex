//! The run title change (protocol 18): a run with a title is named by it in the
//! inspector's title line, the tree and the alerts, its full goal on a row of its own;
//! a run without one is shown exactly as before.

use super::run_tests::{app_of, inspect_node, pairs};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::gemini_fixture;

fn gemini_titled(title: &str) -> crate::app::App {
    let (mut snapshot, windows) = gemini_fixture();
    snapshot.runs[0].title = title.into();
    app_of((snapshot, windows))
}

#[test]
fn a_titled_run_is_named_by_its_title_with_its_goal_on_its_own_row() {
    let app = gemini_titled("Gemini runtime support");
    let inspection = inspect_node(&app, &NodeKey::Run("r1".into()));
    assert_eq!(inspection.name, "r1  Gemini runtime support");
    let fields = pairs(&inspection);
    assert_eq!(fields[0], ("goal", "Add Gemini runtime"));
    assert_eq!(fields[1].0, "progress");
}

#[test]
fn an_untitled_run_is_named_by_its_goal_with_no_goal_row() {
    let app = gemini_titled("");
    let inspection = inspect_node(&app, &NodeKey::Run("r1".into()));
    assert_eq!(inspection.name, "r1  Add Gemini runtime");
    assert!(pairs(&inspection).iter().all(|(label, _)| *label != "goal"));
}

#[test]
fn the_tree_and_alerts_name_a_run_by_its_title() {
    let (snapshot, _) = gemini_fixture();
    let mut run = snapshot.runs[0].clone();
    assert_eq!(crate::tree::run_title(&run), "Add Gemini runtime");
    assert_eq!(crate::tree::run_label(&run), "Add Gemini runtime");
    run.title = "Gemini runtime support".into();
    assert_eq!(crate::tree::run_title(&run), "Gemini runtime support");
    assert_eq!(crate::tree::run_label(&run), "Gemini runtime support");
    // A blank goal and no title: the id, as before.
    run.title = String::new();
    run.goal = "  ".into();
    assert_eq!(crate::tree::run_title(&run), "r1");
    // A title names even a run whose goal is blank.
    run.title = "Gemini runtime support".into();
    assert_eq!(crate::tree::run_title(&run), "Gemini runtime support");
}
