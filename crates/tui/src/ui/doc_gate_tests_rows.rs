//! Task M9.6.17, fix round 1: the revising row (review I-1, ruling T17-4), hostile text
//! in every pane and in the Review panel (m3), and the status bar's gate mark (ruling
//! T17-3).

use super::*;
use crate::app::doc_gate::{DocLoad, DocPane};
use crate::app::screens::Screen;
use crate::theme::{Role, role};
use crate::ui::audit;
use proto::{
    ApproachTag, DocGateKind, DocKind, DocSeverity, ReportSummary, RevisingCause, RunReply,
};

/// The spec gate at v2, the orchestrator revising it on `note` for `cause`.
fn revising(note: &str, cause: RevisingCause, w: u16, h: u16) -> crate::app::App {
    let mut run = design_run(DocGateKind::Spec);
    let gate = run.doc_gate.as_mut().unwrap();
    gate.revising = Some(note.into());
    gate.revising_cause = cause;
    opened_with(run, w, h)
}

/// `text` in the frame's interior row, `w` columns wide.
fn row(text: &str, w: u16) -> String {
    let pad = usize::from(w) - 2 - text.chars().count();
    format!("│{text}{}│", " ".repeat(pad))
}

/// Review I-1: while the orchestrator revises, the first row is exactly
/// `revising v<n+1>… (your note: "<note head 60>")`, in the `Working` role (final fix
/// wave FW-47: the run waits on the orchestrator, not on you), at
/// 120×40 and 80×24, and the document starts on the next row. A note longer than 60
/// characters shows its first 60; a read-back's note is anthrex's, not the user's.
#[test]
fn the_revising_row_is_exact() {
    for (w, h) in [(120, 40), (80, 24)] {
        let app = revising("split R1", RevisingCause::Changes, w, h);
        let buffer = audit::draw(&app, w, h);
        let rows = audit::rows(&buffer);
        assert_eq!(
            rows[1],
            row("revising v3… (your note: \"split R1\")", w),
            "{w}x{h}"
        );
        assert!(
            rows[2].starts_with("│# Password reset"),
            "{w}x{h}: {rows:#?}"
        );
        let p = app.palette();
        assert_eq!(buffer[(1, 1)].fg, role(Role::Working, p).fg.unwrap());
    }
    let long = format!("{}{}", "a".repeat(60), "b".repeat(10));
    let drawn = rows(&revising(&long, RevisingCause::Changes, 120, 40), 120, 40);
    let head = "a".repeat(60);
    assert_eq!(
        drawn[1],
        row(&format!("revising v3… (your note: \"{head}\")"), 120)
    );
    let drawn = rows(&revising("split R1", RevisingCause::Back, 120, 40), 120, 40);
    assert_eq!(drawn[1], row("revising v3… (your note: \"split R1\")", 120));
    let note = "anthrex could not read back the stored spec v2";
    let drawn = rows(&revising(note, RevisingCause::ReadBack, 120, 40), 120, 40);
    assert_eq!(drawn[1], row(&format!("revising v3… ({note})"), 120));
    // ASCII: the ellipsis's twin.
    let mut app = revising("split R1", RevisingCause::Changes, 120, 40);
    app.settings.badges.ascii = true;
    let drawn = rows(&app, 120, 40);
    assert!(
        drawn[1].starts_with("|revising v3... (your note: \"split R1\")"),
        "{:?}",
        drawn[1]
    );
}

/// Final fix wave FW-47 (WB-D-I1, milestone 9.0.7 decisions 3 and 4): a design run
/// whose gate the orchestrator is revising waits on the orchestrator, not on you. No
/// cell is drawn in `Attention`, at any gate kind: not the sidebar's run row or its
/// project's roll-up, not the run view's root node, not the gate screen.
#[test]
fn a_revising_design_run_has_no_attention_cell() {
    for kind in [
        DocGateKind::Brainstorm,
        DocGateKind::Spec,
        DocGateKind::Plan,
    ] {
        let mut run = design_run(kind);
        run.doc_gate.as_mut().unwrap().revising = Some("split R1".into());
        let attention = |app: &crate::app::App, what: &str| {
            let buffer = audit::draw(app, 120, 40);
            let fg = role(Role::Attention, app.palette()).fg.unwrap();
            let area = buffer.area;
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    let cell = &buffer[(x, y)];
                    assert!(
                        cell.fg != fg || cell.symbol().trim().is_empty(),
                        "{kind:?} {what}: {:?} at ({x},{y}) is Attention\n{:#?}",
                        cell.symbol(),
                        audit::rows(&buffer)
                    );
                }
            }
        };
        let mut app = app_with(run.clone(), 120, 40);
        attention(&app, "sidebar");
        app.open_run_view(crate::tree::run_fixtures::RUN_ID.into());
        attention(&app, "run view");
        // The plan gate's screen is 9.5's plan review (`open_doc_gate`'s plan arm).
        let app = match kind {
            DocGateKind::Plan => {
                let mut app = app_with(run, 120, 40);
                let _ = app.open_doc_gate(crate::tree::run_fixtures::RUN_ID);
                app
            }
            _ => opened_with(run, 120, 40),
        };
        attention(&app, "gate screen");
    }
}

/// Every row of `app` drawn `w`×`h` is free of hostile characters and fits the width.
fn assert_contained(app: &crate::app::App, w: u16, h: u16, what: &str) {
    let buffer = audit::draw(app, w, h);
    for row in audit::rows(&buffer) {
        assert_eq!(
            crate::safe_text::tests::first_hostile(&row),
            None,
            "{what} {w}x{h}: {row:?}"
        );
        assert!(!row.contains('\x1b'), "{what} {w}x{h}");
        assert!(
            unicode_width::UnicodeWidthStr::width(row.as_str()) <= usize::from(w),
            "{what} {w}x{h}: {row:?}"
        );
    }
}

/// Review m3: hostile text in the diff, the findings, the drafts and the Review panel
/// (the gate's change summary, a disputed finding, the report's approaches, the
/// not-reviewed reason) draws inside its area at every size, with no hostile character
/// on screen.
#[test]
fn hostile_text_in_every_pane_stays_inside_it() {
    let bad = crate::safe_text::tests::hostile_text();
    let long = "y".repeat(3_000);
    let evil = format!("\x1b[2J\x1b]0;owned\x07{bad} {long}");
    let mut run = design_run(DocGateKind::Brainstorm);
    let gate = run.doc_gate.as_mut().unwrap();
    gate.changes_summary = vec![evil.clone()];
    gate.disputed = vec![finding(&evil, DocSeverity::Blocking, &evil)];
    gate.disputed[0].place = evil.clone();
    gate.not_reviewed = Some(evil.clone());
    gate.report = Some(ReportSummary {
        agree: 1,
        disagree: 0,
        approaches: vec![ApproachTag {
            name: evil.clone(),
            tag: evil.clone(),
        }],
    });
    for (w, h) in [(120, 40), (80, 24), (100, 30), (20, 5), (1, 1)] {
        let mut app = opened_with(run.clone(), w, h);
        assert_contained(&app, w, h, "panel");
        // `d`: the diff.
        let asks = sent(&app.on_key(key('d')));
        reply(
            &mut app,
            RunReply::Doc {
                doc: Box::new(proto::DocView {
                    diff: Some(format!("@@ {evil}\n-{evil}\n+{evil}")),
                    ..view(DocKind::Brainstorm, 1, "x")
                }),
                request_id: Some(asks[0].0),
            },
        );
        assert_eq!(screen(&app).pane, DocPane::Diff);
        assert_contained(&app, w, h, "diff");
        // `f`: the findings, with a hostile answer.
        if let Some(Screen::DocGate(s)) = &mut app.screen {
            s.doc = DocLoad::Ready(Box::new(proto::DocView {
                findings: vec![(
                    finding(&evil, DocSeverity::Minor, &evil),
                    Some(evil.clone()),
                )],
                ..view(DocKind::Brainstorm, 1, "x")
            }));
        }
        app.on_key(key('f'));
        assert_eq!(screen(&app).pane, DocPane::Findings);
        assert_contained(&app, w, h, "findings");
        // `g`: the drafts, one hostile, one refused in hostile words.
        let asks = sent(&app.on_key(key('g')));
        assert_eq!(asks.len(), 2);
        reply(
            &mut app,
            RunReply::Doc {
                doc: Box::new(view(DocKind::BrainstormDraft, 1, &evil)),
                request_id: Some(asks[0].0),
            },
        );
        reply(
            &mut app,
            RunReply::Refused {
                request: proto::run_wire::request::SHOW_DOC.into(),
                message: evil.clone(),
                request_id: Some(asks[1].0),
            },
        );
        assert_eq!(screen(&app).pane, DocPane::Drafts);
        assert_contained(&app, w, h, "drafts");
    }
}

/// Ruling T17-3: the status bar's `⏸ <kind> v<n>` is drawn in the `Paused` role (9.0.7
/// gives `Attention` to `⚑` alone), and under a dialog the bar is 9.0.7's exact
/// ` DIALOG  esc back`, with no mark.
#[test]
fn the_gate_mark_is_paused_and_absent_under_a_dialog() {
    let app = app_with(design_run(DocGateKind::Spec), 120, 40);
    let buffer = audit::draw(&app, 120, 40);
    let bar = &audit::rows(&buffer)[39];
    let at = bar.find("⏸ spec v2").expect("the mark");
    let x = bar[..at].chars().count() as u16;
    let p = app.palette();
    assert_eq!(buffer[(x, 39)].fg, role(Role::Paused, p).fg.unwrap());
    assert_eq!(buffer[(x + 2, 39)].fg, role(Role::Paused, p).fg.unwrap());

    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('c'));
    assert!(app.modal.is_some(), "the note editor");
    let drawn = rows(&app, 120, 40);
    assert_eq!(drawn[39].trim_end(), " DIALOG  esc back");
}
