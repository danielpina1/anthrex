//! Milestone 9.6 task 17: the gate screen's `g` view, its Alerts row and status bar
//! texts, its purity, and hostile documents at every size.

use super::*;
use crate::app::App;
use crate::app::doc_gate::DocLoad;
use crate::app::screens::Screen;
use crate::settings::UiSettings;
use crate::tree::run_fixtures::gate_fixture;
use crate::ui::audit;
use proto::{DaemonMsg, DocGateKind, DocKind, RunReply, RunRequest};

/// Task 9's carry (m6): `g` shows the drafts as their files hold them, asked of the
/// daemon one `ShowDoc` each; side by side when wide, stacked below 100 columns; an
/// unreadable draft shows the daemon's text as `(no draft: …)`.
#[test]
fn g_shows_the_drafts_side_by_side_when_wide() {
    let mut app = opened(DocGateKind::Brainstorm, 120, 40);
    let effects = app.on_key(key('g'));
    let asks = sent(&effects);
    let asked: Vec<(DocKind, Option<u32>)> = asks
        .iter()
        .map(|(_, r)| match r {
            RunRequest::ShowDoc { kind, version, .. } => (*kind, *version),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        asked,
        [
            (DocKind::BrainstormDraft, Some(1)),
            (DocKind::BrainstormDraft, Some(2))
        ]
    );
    reply(
        &mut app,
        RunReply::Doc {
            doc: Box::new(view(
                DocKind::BrainstormDraft,
                1,
                "## Understanding\nClaude's take.",
            )),
            request_id: Some(asks[0].0),
        },
    );
    reply(
        &mut app,
        RunReply::Refused {
            request: proto::run_wire::request::SHOW_DOC.into(),
            message:
                "document brainstorm draft v2 does not match what was stored; it was not shown"
                    .into(),
            request_id: Some(asks[1].0),
        },
    );
    let rows = rows(&app, 120, 40);
    assert!(rows[1].starts_with("│claude · draft v1"), "{rows:#?}");
    assert!(rows[1].contains(" │ codex · draft v2"), "{rows:#?}");
    assert!(rows[2].starts_with("│## Understanding"), "{rows:#?}");
    assert!(
        rows[2].contains(" │ (no draft: document brainstorm draft v2 does not"),
        "{rows:#?}"
    );

    let mut narrow = opened(DocGateKind::Brainstorm, 90, 30);
    let asks = sent(&narrow.on_key(key('g')));
    for (id, _) in &asks {
        reply(
            &mut narrow,
            RunReply::Doc {
                doc: Box::new(view(DocKind::BrainstormDraft, 1, "A draft.")),
                request_id: Some(*id),
            },
        );
    }
    let rows = audit::rows(&audit::draw(&narrow, 90, 30));
    let body: Vec<&str> = rows[1..6]
        .iter()
        .map(|r| r.trim_end_matches('│').trim_end())
        .collect();
    assert_eq!(
        body,
        [
            "│claude · draft v1",
            "│A draft.",
            "│",
            "│codex · draft v2",
            "│A draft."
        ]
    );
}

#[test]
fn the_alert_row_and_status_bar_text_are_exact() {
    let app = app_with(design_run(DocGateKind::Spec), 120, 40);
    let alerts = crate::app::alerts(&app);
    assert_eq!(alerts[0].text, "spec v2 ready for review · run 3f9a");
    let mut round = design_run(DocGateKind::Brainstorm);
    round.round = 2;
    let app = app_with(round, 120, 40);
    assert_eq!(
        crate::app::alerts(&app)[0].text,
        "round 2 brainstorm v1 ready for review · run 3f9a"
    );
    let rows = rows(&app, 120, 40);
    assert!(rows[39].contains("⏸ brainstorm v1"), "{:?}", rows[39]);
    // While the orchestrator revises: no alert, no mark.
    let mut revising = design_run(DocGateKind::Spec);
    revising.doc_gate.as_mut().unwrap().revising = Some("split R1".into());
    let app = app_with(revising, 120, 40);
    assert!(
        crate::app::alerts(&app)
            .iter()
            .all(|a| !a.text.contains("ready for review"))
    );
    assert!(!rows_of(&app).join("\n").contains("⏸"));
    // A run without the design flow keeps 9.5's gate alert.
    let (snap, windows) = gate_fixture();
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    assert_eq!(
        crate::app::alerts(&app)[0].text,
        "plan awaits approval · 2 tasks"
    );
    // ASCII: the mark's twin.
    let mut app = app_with(design_run(DocGateKind::Spec), 120, 40);
    app.settings.badges.ascii = true;
    assert!(
        rows_of(&app)[39].contains("|| spec v2"),
        "{:?}",
        rows_of(&app)[39]
    );
}

fn rows_of(app: &App) -> Vec<String> {
    rows(app, 120, 40)
}

/// AGENTS.md hard rule 5: the screen's state and its renderers do no I/O, and render
/// from `&App`.
#[test]
fn rendering_takes_app_by_reference_and_does_no_io() {
    let sources = [
        ("app/doc_gate.rs", include_str!("../app/doc_gate.rs")),
        (
            "app/doc_gate_replies.rs",
            include_str!("../app/doc_gate_replies.rs"),
        ),
        (
            "app/doc_gate_note.rs",
            include_str!("../app/doc_gate_note.rs"),
        ),
        ("doc_note.rs", include_str!("../doc_note.rs")),
        (
            "app/doc_gate_info.rs",
            include_str!("../app/doc_gate_info.rs"),
        ),
        ("ui/doc_gate.rs", include_str!("doc_gate.rs")),
        ("ui/doc_gate_panes.rs", include_str!("doc_gate_panes.rs")),
        ("ui/doc_text.rs", include_str!("doc_text.rs")),
        ("ui/doc_note.rs", include_str!("doc_note.rs")),
    ];
    let banned = [
        "std::fs",
        "std::io",
        "std::net",
        "std::process",
        "std::thread",
        "std::env",
        "tokio",
        "SystemTime",
        "Instant::now",
        "File::",
        "Command::",
    ];
    for (name, text) in sources {
        for word in banned {
            assert!(!text.contains(word), "{name} names {word}");
        }
    }
    let ui = include_str!("doc_gate.rs");
    assert!(
        ui.contains("pub fn render(frame: &mut Frame, app: &App, s: &DocGateScreen, area: Rect)")
    );
    assert!(
        !ui.contains("&mut App"),
        "the renderer never takes the app mutably"
    );
}

/// Review focus 4: a document of control characters, escape sequences, a 5,000-column
/// line and 10,000 lines draws inside its area at every size, with no hostile
/// character on screen.
#[test]
fn a_hostile_document_renders_inside_its_area() {
    let bad = crate::safe_text::tests::hostile_text();
    let long = "x".repeat(5_000);
    let mut text =
        format!("# Title {bad}\n\x1b[2J\x1b]0;owned\x07R1 {bad}\n{long}\n```\n{bad}\n```\n");
    for i in 0..10_000 {
        text.push_str(&format!("line {i}\n"));
    }
    for (w, h) in [(120, 40), (80, 24), (100, 30), (20, 5), (5, 40), (1, 1)] {
        let mut app = opened(DocGateKind::Spec, w, h);
        assert!(matches!(screen(&app).doc, DocLoad::Ready(_)), "{w}x{h}");
        if let Some(Screen::DocGate(s)) = &mut app.screen {
            s.doc = DocLoad::Ready(Box::new(view(DocKind::Spec, 2, &text)));
            s.message = Some((
                crate::app::doc_gate::Tone::Refused,
                format!("refused {bad} {long}"),
            ));
            // Scrolled to the end, where the line cap's mark is.
            s.scroll = usize::MAX;
        }
        let buffer = audit::draw(&app, w, h);
        let all = audit::rows(&buffer).join("\n");
        for row in audit::rows(&buffer) {
            assert_eq!(
                crate::safe_text::tests::first_hostile(&row),
                None,
                "{w}x{h}"
            );
        }
        assert!(!all.contains('\x1b'), "{w}x{h}");
        for row in audit::rows(&buffer) {
            assert!(
                unicode_width::UnicodeWidthStr::width(row.as_str()) <= usize::from(w),
                "{w}x{h}: {row:?}"
            );
        }
        if w >= 80 {
            // The frame is whole: its right border on every interior row.
            for row in &audit::rows(&buffer)[1..usize::from(h) - 2] {
                assert!(row.ends_with('│'), "{w}x{h}: {row:?}");
            }
            assert!(all.contains("[cut: "), "{w}x{h}: the line cap is marked");
        }
    }
}

/// At 80x24 nothing overlaps: the header, the document, the panel row and the message
/// line each keep their own rows, and the frame is whole.
#[test]
fn at_80x24_nothing_overlaps() {
    let mut app = opened(DocGateKind::Spec, 80, 24);
    if let Some(Screen::DocGate(s)) = &mut app.screen {
        s.message = Some((
            crate::app::doc_gate::Tone::Refused,
            "run 3f9a is not waiting at the spec gate".into(),
        ));
    }
    let rows = rows(&app, 80, 24);
    assert_eq!(rows.len(), 24);
    assert!(
        rows[0].starts_with("┌ Spec · run 3f9a · v2 of 2 · "),
        "{rows:#?}"
    );
    assert!(rows[0].ends_with('┐'), "{rows:#?}");
    assert_eq!(
        rows[21],
        format!(
            "│run 3f9a is not waiting at the spec gate{}│",
            " ".repeat(38)
        )
    );
    assert_eq!(
        rows[20],
        format!("│2 changes · 1 disputed findings (f){}│", " ".repeat(43))
    );
    assert!(
        rows[22].starts_with('└') && rows[22].ends_with('┘'),
        "{rows:#?}"
    );
    assert!(rows[1].starts_with("│# Password reset"), "{rows:#?}");
    for row in &rows[1..22] {
        assert!(row.starts_with('│') && row.ends_with('│'), "{row:?}");
    }
}
