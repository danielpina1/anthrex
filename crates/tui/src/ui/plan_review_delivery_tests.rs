//! Ruling R-13 (task M9.2.15's fix round): a `pr` run's gate says how it is delivered,
//! `delivered as <n> pull requests to <owner>/<name>`, under the summary row; a local
//! run's and a hold's review do not.

use super::*;
use crate::theme::{Role, role};
use crate::tree::pr_fixtures::delivery;

/// `plan()` delivered in `pr` mode, `t4` and `t5` in stage 2 when `staged`.
fn pr_plan(staged: bool) -> RunsSnapshot {
    let mut snap = plan();
    let run = &mut snap.runs[0];
    run.delivery = Some(delivery(false));
    for task in &mut run.tasks {
        if staged && (task.id == "t4" || task.id == "t5") {
            task.stage = 2;
        }
    }
    snap
}

/// The header's second row, at 80×24 and 120×40, drawn plain: never `Attention`, which
/// is the overlap warnings'.
#[test]
fn a_pr_gate_says_how_it_is_delivered() {
    for (staged, want) in [
        (true, "delivered as 2 pull requests to fake/app"),
        (false, "delivered as 1 pull request to fake/app"),
    ] {
        let mut app = app_with(pr_plan(staged), ReviewTarget::Gate);
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = draw(&mut app, w, h);
            let got = rows(&buffer);
            assert!(
                got[1].starts_with("│ ") && got[1].contains(" calls"),
                "{got:#?}"
            );
            assert_eq!(
                got[2].trim_end_matches('│').trim(),
                format!("│ {want}"),
                "{w}x{h}"
            );
            let fg = buffer[(2, 2)].fg;
            assert_ne!(
                fg,
                role(Role::Attention, app.palette()).fg.unwrap(),
                "{w}x{h}"
            );
            assert_eq!(&got[3][..3], "├", "the rule follows: {got:#?}");
            assert_spans_clean(&app, Rect::new(0, 0, w, h - 1), "pr gate");
        }
    }
}

/// A local run's gate and a `pr` run's hold review have no delivery row.
#[test]
fn a_local_gate_and_a_hold_review_say_nothing_of_delivery() {
    let mut app = app_with(plan(), ReviewTarget::Gate);
    assert!(
        !rows(&draw(&mut app, 120, 40))
            .join("\n")
            .contains("delivered")
    );
    let mut snap = hold_plan();
    snap.runs[0].delivery = Some(delivery(false));
    let mut app = app_with(snap, ReviewTarget::Hold("epic:mail".into()));
    assert!(
        !rows(&draw(&mut app, 120, 40))
            .join("\n")
            .contains("delivered")
    );
}

/// The hostile-text rule: the repo is the host's, cleaned (and folded in ASCII); an
/// unknown one is left out.
#[test]
fn the_delivery_rows_repo_is_cleaned() {
    let mut snap = pr_plan(true);
    if let Some(d) = snap.runs[0].delivery.as_mut() {
        d.repo = "fa\u{200D}ke/a\u{202E}pp\u{1b}[2J".into();
    }
    for ascii in [false, true] {
        let mut app = app_with(snap.clone(), ReviewTarget::Gate);
        app.settings.badges.ascii = ascii;
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = draw(&mut app, w, h);
            let got = rows(&buffer).join("\n");
            assert!(
                got.contains("delivered as 2 pull requests to fake/app [2J"),
                "{got}"
            );
            assert_clean(&buffer, "pr gate");
            assert_spans_clean(&app, Rect::new(0, 0, w, h - 1), "pr gate");
            if ascii {
                assert!(got.is_ascii(), "{got}");
            }
        }
    }
    if let Some(d) = snap.runs[0].delivery.as_mut() {
        d.repo = " ".into();
    }
    let mut app = app_with(snap, ReviewTarget::Gate);
    let got = rows(&draw(&mut app, 120, 40));
    assert!(
        got[2].contains("delivered as 2 pull requests ") && !got[2].contains(" to"),
        "{got:#?}"
    );
}
