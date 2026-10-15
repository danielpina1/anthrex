//! Decision 47's search. Pure; no clock.

use super::*;
use crate::lessons_graph::fixture::{self, APPROVED, BILLING, KNOWN, PROPOSED, TRIAL, WEB};

/// The ids `query` matches, in the graph's order.
fn hits(query: &str) -> Vec<String> {
    let graph = fixture::graph();
    let lit = matches(&graph, query);
    assert_eq!(lit.len(), graph.nodes.len());
    graph
        .nodes
        .iter()
        .zip(lit)
        .filter(|(_, m)| *m)
        .map(|(n, _)| n.id.clone())
        .collect()
}

#[test]
fn query_matches_title_body_files_and_module_names_case_insensitively() {
    // A title, in another case.
    assert_eq!(hits("INVOICE BUILDER"), [PROPOSED]);
    // A body: every lesson's body says "second line".
    assert_eq!(hits("Second Line").len(), 5);
    // Files.
    assert_eq!(hits("migrations/"), [TRIAL]);
    assert_eq!(hits("WEB/TEMPLATES"), [APPROVED]);
    // A module's name; `billing` is also in lessons' files.
    assert_eq!(hits("web"), [WEB, APPROVED]);
    let billing = hits("Billing");
    assert!(billing.contains(&BILLING.to_string()), "{billing:?}");
    assert!(billing.contains(&KNOWN.to_string()), "{billing:?}");
    assert!(hits("nothing like this").is_empty());
    // The id is not searched (only title, body, files and module names).
    assert!(hits("l-2222").is_empty());
}

#[test]
fn n_and_big_n_cycle_in_id_order() {
    let graph = fixture::graph();
    // "second line": the five lessons; id order is kf-… then l-1… to l-4….
    let lit = matches(&graph, "second line");
    let ids = [
        "kf-0badf00d",
        "l-11111111",
        "l-22222222",
        "l-33333333",
        "l-44444444",
    ];
    let mut at: Option<String> = None;
    for want in ids.iter().chain(ids.iter().take(1)) {
        at = cycle(&graph, &lit, at.as_deref(), true);
        assert_eq!(at.as_deref(), Some(*want));
    }
    // Backwards from the first wraps to the last.
    let back = cycle(&graph, &lit, Some(ids[0]), false);
    assert_eq!(back.as_deref(), Some(ids[4]));
    assert_eq!(cycle(&graph, &lit, None, false).as_deref(), Some(ids[4]));
    // From a node that is no match, the next match after it in id order.
    assert_eq!(
        cycle(&graph, &lit, Some(WEB), true).as_deref(),
        Some(ids[0]),
        "m:web sorts after every lesson: wraps"
    );
    let none = matches(&graph, "zzz");
    assert_eq!(cycle(&graph, &none, Some(TRIAL), true), None);
}

#[test]
fn esc_clears_then_leaves() {
    let mut search = Search::default();
    search.open();
    for c in "Invoice".chars() {
        assert!(search.key(KeyCode::Char(c)));
    }
    search.key(KeyCode::Backspace);
    assert_eq!(search.query, "Invoic");
    assert!(search.key(KeyCode::Enter));
    assert!(!search.typing, "Enter ends typing");
    assert!(search.active(), "and keeps the filter");
    assert!(
        !search.key(KeyCode::Char('x')),
        "keys are no longer the query's"
    );
    assert!(search.esc(), "the first Esc clears");
    assert_eq!(search, Search::default());
    assert!(
        !search.esc(),
        "the second has nothing to clear: the page leaves"
    );
    // Esc while typing clears too.
    search.open();
    search.key(KeyCode::Char('a'));
    assert!(search.key(KeyCode::Esc));
    assert_eq!(search, Search::default());
}

#[test]
fn empty_query_matches_nothing_and_dims_nothing() {
    let graph = fixture::graph();
    assert!(matches(&graph, "").iter().all(|m| !m));
    let mut search = Search::default();
    search.open();
    assert!(!search.active(), "typing an empty query is no filter");
}
