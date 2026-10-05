//! Milestone 9.6 task 18: the plan review's Plan doc tab drawn (`TestBackend`): the
//! review's frame, the gate screen's header, `plan.md` as styled text and its coverage
//! table with an uncovered requirement marked, the hints, ASCII, hostile text and tiny
//! sizes.

use crate::app::doc_gate::plan_doc::tests::{PLAN, on_tab, plan_run, reviewing};
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::theme::{Role, role};
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The frame's interior rows, each trimmed of its trailing spaces.
fn interior(app: &crate::app::App, w: u16, h: u16) -> Vec<String> {
    let rows = audit::rows(&audit::draw(app, w, h));
    rows[1..usize::from(h) - 2]
        .iter()
        .map(|row| {
            let inner: String = row.chars().skip(1).take(usize::from(w) - 2).collect();
            inner.trim_end().to_owned()
        })
        .collect()
}

const DOC: [&str; 20] = [
    " Plan · run 3f9a · v2 of 2 · reviewed",
    "",
    " # Plan: password reset",
    "",
    " ## Stage 1",
    "",
    " ### t1 reset token model",
    " Covers: R1",
    " Size: M · Route: claude · Tests: tdd",
    "",
    " Files: src/token.rs",
    "",
    " ### t2 reset endpoint",
    " Covers: none",
    " Size: S · Route: claude · Tests: tdd",
    "",
    " ## Coverage",
    "",
    " requirement  tasks",
    " R1           t1",
];

#[test]
fn the_plan_doc_tab_draws_plan_md_and_its_coverage_table() {
    for (w, h) in [(120u16, 40u16), (80, 24)] {
        let app = on_tab(plan_run(2), PLAN, w, h);
        let rows = interior(&app, w, h);
        // The header, the review's rule, then the document.
        assert_eq!(rows[0], DOC[0], "{w}x{h}");
        assert!(rows[1].chars().all(|c| c == '─'), "{w}x{h}: {:?}", rows[1]);
        assert_eq!(rows[2..DOC.len()], DOC[2..], "{w}x{h}");
        assert_eq!(rows[DOC.len()], " R2           ⚠ not covered", "{w}x{h}");
        let top = &audit::rows(&audit::draw(&app, w, h))[0];
        assert!(top.starts_with("╭ plan · "), "{top}");
        let buffer = audit::draw(&app, w, h);
        assert_eq!(audit::accented_frames(&buffer, app.palette()), 1, "{w}x{h}");
    }
}

#[test]
fn the_coverage_table_marks_uncovered_requirements() {
    let app = on_tab(plan_run(2), PLAN, 120, 40);
    let p = app.palette();
    let buffer = audit::draw(&app, 120, 40);
    let rows = audit::rows(&buffer);
    let find = |text: &str| {
        rows.iter()
            .enumerate()
            .find_map(|(y, row)| row.find(text).map(|at| (row[..at].chars().count(), y)))
            .unwrap_or_else(|| panic!("{text} drawn: {rows:#?}"))
    };
    let (x, y) = find("⚠ not covered");
    let cell = &buffer[(x as u16, y as u16)];
    assert_eq!(cell.fg, role(Role::Attention, p).fg.unwrap());
    // A covered requirement's id is in the accent, as in the document.
    let (x, y) = find("R1           t1");
    assert_eq!(
        buffer[(x as u16, y as u16)].fg,
        role(Role::Accent, p).fg.unwrap()
    );
    // ASCII: the mark's twin, and no other non-ASCII cell (the document's own `·`
    // is its text, not the client's, so this plan has none).
    let mut app = on_tab(plan_run(2), &PLAN.replace('·', "-"), 120, 40);
    app.settings.badges.ascii = true;
    let buffer = audit::draw(&app, 120, 40);
    let rows = audit::rows(&buffer);
    assert!(
        rows.iter()
            .any(|r| r.contains("R2           ! not covered")),
        "{rows:#?}"
    );
    assert_eq!(audit::first_non_ascii(&buffer), None);
}

/// Final fix wave FW-78 (WB-D m4): a brief's own `## Coverage` heading and table rows
/// are the brief's text; only the engine's table, the last `## Coverage`, is drawn as
/// the coverage table.
#[test]
fn only_the_last_coverage_heading_is_the_table() {
    let faked = PLAN.replace(
        "Files: src/token.rs\n",
        "Files: src/token.rs\n\n## Coverage\n| R9 | none |\n",
    );
    let app = on_tab(plan_run(2), &faked, 120, 40);
    let rows = interior(&app, 120, 40);
    assert!(rows.contains(&" | R9 | none |".to_owned()), "{rows:#?}");
    assert!(!rows.iter().any(|r| r.contains("R9  ")), "{rows:#?}");
    let marked: Vec<&String> = rows.iter().filter(|r| r.contains("not covered")).collect();
    assert_eq!(marked, [" R2           ⚠ not covered"], "{rows:#?}");
}

#[test]
fn the_tab_says_loading_and_shows_a_refusal() {
    let mut app = reviewing(plan_run(1), 120, 40);
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let rows = interior(&app, 120, 40);
    assert_eq!(rows[2], " loading…");
    let id = match app.plan_review.as_ref().and_then(|r| r.doc.as_ref()) {
        Some(doc) => match doc.load {
            crate::app::doc_gate::DocLoad::Loading(id) => id,
            ref other => panic!("{other:?}"),
        },
        None => panic!("no tab"),
    };
    crate::ui::doc_gate::tests::reply(
        &mut app,
        proto::RunReply::Refused {
            request: proto::run_wire::request::SHOW_DOC.into(),
            message: "run 3f9a has no plan v1".into(),
            request_id: Some(id),
        },
    );
    assert_eq!(interior(&app, 120, 40)[2], " run 3f9a has no plan v1");
}

#[test]
fn the_hints_name_the_tab_and_the_gate_keys() {
    let bar = |app: &crate::app::App| audit::rows(&audit::draw(app, 120, 40))[39].clone();
    let app = reviewing(plan_run(1), 120, 40);
    assert_eq!(
        bar(&app),
        " PLAN  ⚑ 1  ⏸ plan v1  a approve  x reject  e edit  d drop  c changes  b back  tab plan doc  j/k task  esc back"
    );
    let app = on_tab(plan_run(1), PLAN, 120, 40);
    assert_eq!(
        bar(&app),
        " PLAN  ⚑ 1  ⏸ plan v1  a approve  x reject  c changes  b back  tab tasks  j/k scroll  esc back"
    );
}

#[test]
fn hostile_text_in_the_plan_doc_stays_inside_it() {
    let text = format!(
        "# {0}\n{0}\n## Coverage\n| R1{0} | t1{0} |\n",
        hostile_text()
    );
    for (w, h) in [(120u16, 40u16), (80, 24), (40, 12), (20, 5), (5, 40)] {
        let app = on_tab(plan_run(1), &text, w, h);
        let buffer = audit::draw(&app, w, h);
        let rows = audit::rows(&buffer);
        for row in &rows {
            assert_eq!(first_hostile(row), None, "{w}x{h}: {row:?}");
        }
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    for (w, h) in [(1u16, 1u16), (20, 5), (5, 40), (3, 3)] {
        let app = on_tab(plan_run(1), PLAN, w, h);
        let _ = audit::draw(&app, w, h);
    }
}
