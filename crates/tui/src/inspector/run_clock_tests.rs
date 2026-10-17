//! Milestone 9.19 decision 3: every time the inspector prints is the viewer's local
//! clock against the daemon's `now`, labelled with its offset; one on another day names
//! it.

use super::run_tests::{app_of, inspect_node, value};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{GEMINI_DAY, GEMINI_NOW, RUN_ID};

/// Every `HH:MM` in `text` followed by ` <label>`: no bare clock time.
fn every_clock_labelled(text: &str, label: &str) -> bool {
    let b = text.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    // A label's own `+02:00` is not a clock time.
    let sign = |i: usize| i >= 3 && matches!(b[i - 3], b'+' | b'-');
    (0..b.len())
        .filter(|&i| b[i] == b':' && i >= 2 && digit(i - 1) && digit(i - 2) && digit(i + 1))
        .filter(|&i| !sign(i))
        .all(|i| text[i + 3..].starts_with(&format!(" {label}")))
}

/// Milestone 9.19 decision 3: every time the inspector prints is the viewer's local
/// clock against the daemon's `now`, labelled with the offset; one on another day names
/// it. `GEMINI_NOW` is 12:40 UTC on a Friday.
#[test]
fn every_inspector_time_is_local_and_labelled() {
    // An approved run, today.
    let mut app = super::run_tests::gemini_with(|run| {
        run.approved_at = Some(GEMINI_DAY + 12 * 3600 + 30 * 60)
    });
    app.utc_offset_secs = 7200;
    let inspection = inspect_node(&app, &NodeKey::Run("r1".into()));
    let gate = value(&inspection, "gate").unwrap();
    assert!(gate.starts_with("plan approved 14:30 +02:00 · "), "{gate}");
    for field in &inspection.fields {
        assert!(
            every_clock_labelled(&field.value, "+02:00"),
            "{}",
            field.value
        );
    }

    // A PR opened two days earlier: a Wednesday.
    let (mut snapshot, windows) = crate::tree::pr_fixtures::pr_fixture();
    snapshot.now = GEMINI_NOW;
    let pr = snapshot.runs[0].stages[1].pr.as_mut().unwrap();
    pr.opened_at = GEMINI_DAY - 2 * 86_400 + 12 * 3600 + 30 * 60;
    let mut app = app_of((snapshot, windows));
    app.utc_offset_secs = 7200;
    let stage = NodeKey::Stage {
        run: RUN_ID.into(),
        n: 2,
    };
    let pr = value(&inspect_node(&app, &stage), "pr").unwrap().to_owned();
    assert!(pr.ends_with(", opened Wed 14:30 +02:00"), "{pr}");

    // A failed round's retry, later today.
    let mut app = super::run_round_tests::t7_on_window(|round| {
        round.rate_limited = false;
        round.rate_limited_until = None;
        round.rate_limited_since = None;
        round.failed_error = Some("overloaded".into());
        round.failed_until = Some(GEMINI_NOW + 600);
    });
    app.utc_offset_secs = 7200;
    let key = super::run_round_tests::round_key("t7", proto::AgentRole::Worker, 1, 1);
    let right = inspect_node(&app, &key).right.unwrap();
    assert!(right.contains(" · retries 14:50 +02:00 · "), "{right}");
    assert!(every_clock_labelled(&right, "+02:00"), "{right}");
}

#[test]
fn a_bare_clock_time_is_caught() {
    assert!(every_clock_labelled(
        "at 14:30 +02:00, then 15:00 +02:00",
        "+02:00"
    ));
    assert!(!every_clock_labelled("at 14:30", "+02:00"));
    assert!(!every_clock_labelled("at 14:30 UTC", "+02:00"));
    assert!(every_clock_labelled("no time here: 4:3", "+02:00"));
}
