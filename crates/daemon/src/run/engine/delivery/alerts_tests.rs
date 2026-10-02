//! Ruling R-13: each `RunDelivery.alerts` key's kind, and that the attention lines and
//! the typed alerts are one list. The conditions themselves are driven in
//! `engine/tests/delivery_*.rs` (one assertion per kind beside its attention line).

use super::classify;
use proto::DeliveryAlertKind::*;

#[test]
fn every_key_has_its_kind_and_stage() {
    for (key, want) in [
        ("auth", (Some(GhLoggedOut), None)),
        ("2/closed", (Some(PrClosedUnmerged), Some(2))),
        ("1/ci/test::a", (Some(CiHandedToUser), Some(1))),
        ("3/cap/7:c5", (Some(ReviewRoundsOverCap), Some(3))),
        ("1/push", (Some(HostOpHeld), Some(1))),
        ("run/permission", (Some(HostOpHeld), None)),
        // Attention lines only.
        ("1/reply/7:c5", (None, None)),
        ("1/review/7:c5", (None, None)),
        ("1/page/reviews", (None, None)),
        ("odd", (None, None)),
        ("x/push", (None, None)),
    ] {
        assert_eq!(classify(key), want, "{key}");
    }
}

/// The typed alert's text is one line with no hidden character, whatever the engine
/// filed (its text is built from the host's): this list's own guarantee, not only the
/// callers' cleaning.
#[test]
fn an_alerts_text_is_one_clean_line() {
    let mut run = crate::run::test_support::run_ok(crate::run::test_support::EXAMPLE_PLAN);
    run.delivery.mode = proto::DeliveryMode::Pr;
    run.delivery.alerts.insert(
        "1/closed".into(),
        "PR #7 was cl\u{200D}osed\u{202E}\nby the host\u{1b}[2J".into(),
    );
    let alerts = super::alerts(&run);
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, PrClosedUnmerged);
    assert_eq!(alerts[0].text, "PR #7 was closed by the host [2J");
    // The attention line is the same line, as filed.
    assert_eq!(
        super::attention(&run),
        ["PR #7 was cl\u{200D}osed\u{202E}\nby the host\u{1b}[2J"]
    );
}
