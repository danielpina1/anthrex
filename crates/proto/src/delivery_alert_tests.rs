//! Task M9.2.15's fix round (ruling R-13): `DeliveryInfo.alerts`, its keys and both
//! encodings, and that a snapshot without it still loads.

use super::*;
use serde_json::json;

fn info(alerts: Vec<DeliveryAlert>) -> DeliveryInfo {
    DeliveryInfo {
        mode: DeliveryMode::Pr,
        remote: "origin".into(),
        repo: "fake/app".into(),
        watching: true,
        delivering: true,
        poll_secs: 60,
        skipped_stages: Vec::new(),
        alerts,
    }
}

fn every_kind() -> Vec<DeliveryAlert> {
    [
        (DeliveryAlertKind::CiHandedToUser, Some(1)),
        (DeliveryAlertKind::PrClosedUnmerged, Some(2)),
        (DeliveryAlertKind::GhLoggedOut, None),
        (DeliveryAlertKind::HostOpHeld, Some(3)),
        (DeliveryAlertKind::ReviewRoundsOverCap, Some(4)),
    ]
    .into_iter()
    .map(|(kind, stage)| DeliveryAlert {
        kind,
        stage,
        text: format!("{kind:?}"),
        user_only: false,
    })
    .collect()
}

#[test]
fn delivery_alerts_round_trip_named_and_in_json() {
    let value = info(every_kind());
    let packed = rmp_serde::to_vec_named(&value).unwrap();
    let back: DeliveryInfo = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(back, value, "MessagePack");
    let back: DeliveryInfo = serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
    assert_eq!(back, value, "JSON");
    let kinds: Vec<serde_json::Value> = every_kind()
        .iter()
        .map(|a| serde_json::to_value(a.kind).unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            json!("ci_handed_to_user"),
            json!("pr_closed_unmerged"),
            json!("gh_logged_out"),
            json!("host_op_held"),
            json!("review_rounds_over_cap"),
        ]
    );
    assert_eq!(
        serde_json::to_value(&every_kind()[2]).unwrap(),
        json!({"kind": "gh_logged_out", "stage": null, "text": "GhLoggedOut"})
    );
}

/// Left out when empty, so a delivery with nothing to act on encodes as before, and
/// a snapshot written before the field loads with none.
#[test]
fn no_alerts_are_left_out_and_an_old_snapshot_loads() {
    let json = serde_json::to_value(info(Vec::new())).unwrap();
    assert!(json.get("alerts").is_none(), "{json}");
    let old = json!({
        "mode": "pr", "remote": "origin", "repo": "fake/app", "watching": true,
        "delivering": false, "poll_secs": 60, "skipped_stages": [],
    });
    let back: DeliveryInfo = serde_json::from_value(old.clone()).unwrap();
    assert!(back.alerts.is_empty());
    let packed = rmp_serde::to_vec_named(&old).unwrap();
    let back: DeliveryInfo = rmp_serde::from_slice(&packed).unwrap();
    assert!(back.alerts.is_empty());
}
