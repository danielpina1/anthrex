//! Milestone 9.6 task 17: the document gate screen, drawn (`TestBackend`). The views
//! (`d`, `f`, `g`), the texts, purity and hostile documents are in
//! `doc_gate_tests_views.rs`; the shared fixtures in `doc_gate_fixture.rs`.

#[path = "doc_gate_fixture.rs"]
mod fixture;
pub(crate) use fixture::*;

#[path = "doc_gate_tests_views.rs"]
mod views;

use crate::app::doc_gate::{DocLoad, DocPane};
use crate::theme::{Role, role};
use crate::ui::audit;
use crossterm::event::KeyCode;
use proto::{DocGateKind, DocKind, DocView, RunReply, RunRequest};

#[test]
fn the_gate_screen_at_120x40_has_the_document_and_the_review_panel() {
    let app = opened(DocGateKind::Spec, 120, 40);
    assert!(matches!(screen(&app).doc, DocLoad::Ready(_)));
    let rows = rows(&app, 120, 40);
    assert!(
        rows[0]
            .starts_with("┌ Spec · run 3f9a · v2 of 2 · revised after your note: \"split R1\" ─"),
        "{rows:#?}"
    );
    // The document on the left, a rule, the Review panel on the right (44 columns).
    let at = |row: &str, text: &str| row.find(text);
    assert_eq!(
        rows[1],
        format!(
            "│# Password reset{}│ Review{}│",
            " ".repeat(62),
            " ".repeat(32)
        )
    );
    let joined = rows.join("\n");
    for text in [
        "│R1 A user asks for a reset by mail.",
        "│R2 The link expires after an hour.",
        "│cargo test reset",
        "│ changes",
        "│   + R2",
        "│   ~ Testing: 2 lines",
        "│ disputed findings",
        "│   F2 minor at R2",
        "│     Say which hour.",
        "│     answer: kept: the hour is in R2",
    ] {
        assert!(joined.contains(text), "{text:?} in\n{joined}");
    }
    let panel = rows.iter().find(|r| r.contains("│ changes")).unwrap();
    assert_eq!(at(panel, "│ changes"), at(&rows[1], "│ Review"));
    // The status bar: the badge, the gate's mark and the keys.
    let bar = &rows[39];
    assert!(bar.starts_with(" REVIEW  ⏸ spec v2  a approve"), "{bar:?}");
    assert!(bar.contains("esc close"), "{bar:?}");
}

#[test]
fn below_100_columns_the_panel_is_one_line() {
    let app = opened(DocGateKind::Spec, 99, 30);
    let rows = rows(&app, 99, 30);
    assert!(!rows.join("\n").contains("Review"), "{rows:#?}");
    assert_eq!(
        rows[27],
        "│2 changes · 1 disputed findings (f)".to_owned() + &" ".repeat(62) + "│"
    );
    assert!(rows[1].starts_with("│# Password reset"), "{rows:#?}");
    // The document takes the width: no rule.
    assert_eq!(rows[2].matches('│').count(), 2, "{rows:#?}");
}

#[test]
fn d_shows_the_coloured_diff() {
    let mut app = opened(DocGateKind::Spec, 120, 40);
    let effects = app.on_key(key('d'));
    let [
        (
            id,
            RunRequest::ShowDoc {
                diff: true,
                version: Some(2),
                kind: DocKind::Spec,
                ..
            },
        ),
    ] = &sent(&effects)[..]
    else {
        panic!("one ShowDoc with its diff: {effects:?}");
    };
    let diff = "@@ -4,1 +4,2 @@\n R1 A user asks for a reset by mail.\n-R2 old\n+R2 The link expires after an hour.";
    let doc = DocView {
        diff: Some(diff.into()),
        ..view(DocKind::Spec, 2, SPEC)
    };
    reply(
        &mut app,
        RunReply::Doc {
            doc: Box::new(doc),
            request_id: Some(*id),
        },
    );
    assert_eq!(screen(&app).pane, DocPane::Diff);
    let buffer = audit::draw(&app, 120, 40);
    let rows = audit::rows(&buffer);
    assert_eq!(
        &rows[1..6],
        [
            "│diff against v1",
            "│@@ -4,1 +4,2 @@",
            "│ R1 A user asks for a reset by mail.",
            "│-R2 old",
            "│+R2 The link expires after an hour.",
        ]
        .map(|r| format!("{r}{}│", " ".repeat(119 - r.chars().count())))
    );
    let p = app.palette();
    let fg = |x: u16, y: u16| buffer[(x, y)].fg;
    assert_eq!(fg(1, 4), role(Role::Failed, p).fg.unwrap());
    assert_eq!(fg(1, 5), role(Role::Done, p).fg.unwrap());
    assert_eq!(fg(1, 2), role(Role::Muted, p).fg.unwrap());
    // `d` again, or Esc, is the document.
    app.on_key(code(KeyCode::Esc));
    assert_eq!(screen(&app).pane, DocPane::Document);
    assert!(app.screen.is_some(), "Esc in a view goes back, not out");
}

#[test]
fn f_lists_findings_with_severities() {
    let mut app = opened(DocGateKind::Spec, 120, 40);
    assert!(
        sent(&app.on_key(key('f'))).is_empty(),
        "the findings came with v2"
    );
    let buffer = audit::draw(&app, 120, 40);
    let rows = audit::rows(&buffer);
    let body: Vec<String> = rows[1..10]
        .iter()
        .map(|r| r.trim_end_matches('│').trim_end().to_owned())
        .collect();
    assert_eq!(
        body,
        [
            "│findings of v2",
            "│F1 blocking at R2",
            "│  R1 has no check.",
            "│  answer: fixed",
            "│",
            "│F2 minor at R2",
            "│  Say which hour.",
            "│  answer: kept: the hour is in R2",
            "│",
        ]
    );
    let p = app.palette();
    assert_eq!(
        buffer[(4, 2)].fg,
        role(Role::Failed, p).fg.unwrap(),
        "blocking"
    );
    assert_eq!(buffer[(4, 6)].fg, role(Role::Muted, p).fg.unwrap(), "minor");
}
