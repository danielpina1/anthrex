//! Task 5's review, I1 (the controller's ruling at task M9.15.6): a known failure's life
//! holds only the hits from its own formation window onward, over the fixtures of
//! `consolidate_tests.rs`.

use super::*;

/// Hits on days 0, 200 and 201 form a life on day 201 from days 200 and 201 alone: the
/// day-0 hit expired long before, so it is not counted, not its start, and its task's
/// files are not the life's (nor its clean-run area).
#[test]
fn a_life_holds_only_the_hits_of_its_formation_window() {
    let store = Store::default();
    let stale = line(owning(
        failed(task(&rid(1), "t1", T0, TaskOutcome::Blocked), FP, HEADLINE),
        &["crates/old/**"],
    ));
    let history = [
        stale,
        hit(2, T0 + 200 * DAY, FP),
        hit(3, T0 + 201 * DAY, FP),
    ];
    let got = known(&history, &store, T0 + 202 * DAY, &cfg());
    assert_eq!(ids(&got), vec![kf(FP)]);
    let life = &got[0];
    assert_eq!(life.created_at, T0 + 200 * DAY);
    assert_eq!(life.last_seen, T0 + 201 * DAY);
    assert_eq!(life.runs, vec![rid(2), rid(3)]);
    assert_eq!(life.files, vec!["crates/a/**".to_string()]);
    assert!(
        life.body.starts_with("Seen 2 times in 2 runs"),
        "{}",
        life.body
    );
}

/// The ruling's second half: before any life formed, a hit followed by a gap longer than
/// `expiry_days` has expired, so it cannot help form one even inside a window that is
/// wider than the expiry.
#[test]
fn an_expired_hit_never_helps_form_a_life() {
    let store = Store::default();
    let wide = Lessons {
        known_window_days: 60,
        expiry_days: 30,
        ..cfg()
    };
    let history = [hit(1, T0, FP), hit(2, T0 + 40 * DAY, FP)];
    assert!(known(&history, &store, T0 + 41 * DAY, &wide).is_empty());
    // Within the expiry, the same window forms it from both.
    let close = [hit(1, T0, FP), hit(2, T0 + 20 * DAY, FP)];
    let got = known(&close, &store, T0 + 21 * DAY, &wide);
    assert_eq!(ids(&got), vec![kf(FP)]);
    assert_eq!(got[0].created_at, T0);
}
