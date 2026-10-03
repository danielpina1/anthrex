//! Milestone 9.3 task 10b: the run inspector's rounds, a round separator's inspection,
//! and DETAIL's `round` (decision 32, KG §6), each for a run of several rounds only.

use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::tree::NodeKey;
use crate::tree::stage_fixtures::{two_round_fixture, two_stage_fixture};

const MUL: &str = "add-mul-0723";

fn rows_of(inspection: &crate::inspector::Inspection) -> Vec<(&str, &str)> {
    pairs(inspection)
        .into_iter()
        .skip_while(|(label, _)| *label != "rounds")
        .collect()
}

#[test]
fn the_inspector_lists_the_rounds() {
    let app = app_of(two_round_fixture());
    let run = inspect_node(&app, &NodeKey::Run(MUL.into()));
    assert_eq!(
        rows_of(&run),
        [
            ("rounds", "round 1 · user · completed · mul is in a"),
            ("", "round 2 · user · running · -"),
        ]
    );
    // A one-round run lists none.
    let app = app_of(two_stage_fixture());
    let run = inspect_node(&app, &NodeKey::Run(MUL.into()));
    assert!(rows_of(&run).is_empty(), "{:?}", pairs(&run));

    // The separator's own inspection.
    let app = app_of(two_round_fixture());
    let round = inspect_node(
        &app,
        &NodeKey::Round {
            run: MUL.into(),
            n: 1,
        },
    );
    assert_eq!(round.glyph.content, "✓");
    assert_eq!(round.name, "round 1");
    assert_eq!(round.right.as_deref(), Some("user · completed"));
    assert_eq!(
        pairs(&round),
        [("request", "Add mul()"), ("summary", "mul is in a")]
    );
}

#[test]
fn detail_shows_the_round_for_a_multi_round_run() {
    let task = |id: &str| NodeKey::Task {
        run: MUL.into(),
        id: id.into(),
    };
    let detail = |app: &crate::app::App, id: &str| {
        let sections = inspect_node(app, &task(id)).sections;
        let detail = (sections.into_iter())
            .find(|section| section.title == "DETAIL")
            .expect("a DETAIL section");
        detail
            .fields
            .into_iter()
            .map(|field| (field.label, field.value))
            .collect::<Vec<_>>()
    };
    let app = app_of(two_round_fixture());
    let rows = detail(&app, "t2");
    let at = |label: &str| rows.iter().position(|(l, _)| *l == label);
    assert_eq!(rows[at("round").expect("a round row")].1, "2");
    assert_eq!(
        at("round"),
        at("stage").map(|stage| stage + 1),
        "after stage"
    );
    assert_eq!(value(&inspect_node(&app, &task("t1")), "round"), Some("1"));
    // A one-round run: no `round` row.
    let app = app_of(two_stage_fixture());
    assert!(
        detail(&app, "t2")
            .iter()
            .all(|(label, _)| *label != "round")
    );
    assert_eq!(value(&inspect_node(&app, &task("t2")), "round"), None);
}
