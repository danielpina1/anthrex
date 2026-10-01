//! Ruling C-28 (5): a Multi plan's review shows each task's stage, and its atomic hub
//! (with the plan's reason) and interface change when they are set. A Single plan's
//! review is unchanged.

use super::*;

/// The plan with `t4` and `t5` in stage 2 and `t4` stage 2's atomic hub, changing an
/// interface, for `reason`.
fn staged(reason: &str) -> RunsSnapshot {
    let mut snap = plan();
    for task in &mut snap.runs[0].tasks {
        if task.id == "t4" || task.id == "t5" {
            task.stage = 2;
        }
        if task.id == "t4" {
            task.atomic = true;
            task.atomic_reason = Some(reason.to_owned());
            task.interface_change = true;
        }
    }
    snap
}

/// The right pane's rows below the title, at 120x40.
fn detail(app: &mut App) -> Vec<String> {
    let buffer = draw(app, 120, 40);
    (1..39).map(|y| right_of(&buffer, y)).collect()
}

/// Whether `rows` holds `want` as consecutive rows.
fn holds(rows: &[String], want: &[&str]) -> bool {
    rows.windows(want.len()).any(|w| w == want)
}

#[test]
fn a_multi_plan_review_shows_the_stage_and_the_hub() {
    let mut app = app_with(
        staged("the token type changes for every client"),
        ReviewTarget::Gate,
    );
    // t2 (t1's 40-line brief would push its sections off the screen).
    press(&mut app, KeyCode::Char('j'));
    let t2 = detail(&mut app);
    assert!(t2[0].starts_with("t2  reset endpoint"), "{t2:#?}");
    assert!(holds(&t2, &["stage", "  1 of 2"]), "{t2:#?}");
    assert!(
        !t2.iter().any(|r| r == "atomic" || r == "interface change"),
        "{t2:#?}"
    );
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    let t4 = detail(&mut app);
    assert!(t4[0].starts_with("t4  mail sender"), "{t4:#?}");
    assert!(
        holds(
            &t4,
            &[
                "stage",
                "  2 of 2",
                "",
                "atomic",
                "  the token type changes for every client",
                "",
                "interface change",
                "  yes",
            ]
        ),
        "{t4:#?}"
    );
    // After the test mode, before the deps.
    let at = |label: &str| t4.iter().position(|r| r == label).unwrap();
    assert!(at("test mode") < at("stage") && at("interface change") < at("deps"));
}

#[test]
fn an_atomic_hub_without_a_reason_reads_yes() {
    for reason in [None, Some(" \t ".to_owned())] {
        let mut snap = staged("unused");
        snap.runs[0].tasks[3].atomic_reason = reason.clone();
        let mut app = app_with(snap, ReviewTarget::Gate);
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('j'));
        }
        let t4 = detail(&mut app);
        assert!(holds(&t4, &["atomic", "  yes"]), "{reason:?}: {t4:#?}");
    }
}

#[test]
fn the_stage_facts_are_sanitised() {
    let reason = format!("hub {}", hostile_text());
    for (width, height) in [(80, 24), (120, 40)] {
        let mut app = app_with(staged(&reason), ReviewTarget::Gate);
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('j'));
        }
        for page in 0..6 {
            let buffer = draw(&mut app, width, height);
            let what = format!("{width}x{height} page {page}");
            assert_clean(&buffer, &what);
            assert_spans_clean(&app, Rect::new(0, 0, width, height - 1), &what);
            press(&mut app, KeyCode::PageDown);
        }
    }
    let mut app = app_with(staged(&reason), ReviewTarget::Gate);
    for _ in 0..3 {
        press(&mut app, KeyCode::Char('j'));
    }
    let t4 = detail(&mut app);
    let row = t4
        .iter()
        .position(|r| r == "atomic")
        .expect("the atomic row");
    assert!(t4[row + 1].starts_with("  hub "), "{:?}", t4[row + 1]);
}

/// A Single plan shows no stage, atomic or interface row, even for a task that sets
/// them: its review is the one `review_renders_at_120x40` pins.
#[test]
fn a_single_plan_review_is_unchanged() {
    let mut single = plan();
    let t3 = &mut single.runs[0].tasks[2];
    t3.atomic = true;
    t3.atomic_reason = Some("a hub".into());
    t3.interface_change = true;
    let mut plain = app_with(plan(), ReviewTarget::Gate);
    let mut set = app_with(single, ReviewTarget::Gate);
    for app in [&mut plain, &mut set] {
        press(app, KeyCode::Char('j'));
        press(app, KeyCode::Char('j'));
    }
    assert_eq!(
        rows(&draw(&mut set, 120, 40)),
        rows(&draw(&mut plain, 120, 40))
    );
    let rows = detail(&mut set);
    assert!(
        !rows
            .iter()
            .any(|r| ["stage", "atomic", "interface change"].contains(&r.as_str())),
        "{rows:#?}"
    );
}
