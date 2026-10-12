//! Milestone 9.14 (M9.14.8): protocol 23's admission messages, `ClientMsg::Admit`,
//! `DaemonMsg::Admitted` and `DaemonMsg::AdmitTimedOut` (decisions 16, 20, 21).

use serde_json::json;

use crate::delivery::tests::variant_names;
use crate::*;

fn both<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    let back: T = serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap();
    assert_eq!(&back, v);
    let back: T = rmp_serde::from_slice(&rmp_serde::to_vec_named(v).unwrap()).unwrap();
    assert_eq!(&back, v);
}

#[test]
fn admit_messages_round_trip() {
    for slots in [Some(2), None] {
        both(&ClientMsg::Admit {
            window_id: 7,
            tool_use_id: "toolu_01".into(),
            slots,
            wait_secs: 300,
            label: "agent w7 cargo test -q".into(),
        });
    }
    both(&DaemonMsg::Admitted {
        slots: 3,
        load: "0.75".into(),
    });
    both(&DaemonMsg::AdmitTimedOut);

    // Decision 20: an ask without `slots` (an interactive window's) is `None`.
    let wire = json!({"Admit": {"window_id": 3, "tool_use_id": "a", "wait_secs": 60,
        "label": "agent w3 cargo build"}});
    let expected = ClientMsg::Admit {
        window_id: 3,
        tool_use_id: "a".into(),
        slots: None,
        wait_secs: 60,
        label: "agent w3 cargo build".into(),
    };
    assert_eq!(
        serde_json::from_value::<ClientMsg>(wire.clone()).unwrap(),
        expected
    );
    let packed = rmp_serde::to_vec_named(&wire).unwrap();
    assert_eq!(
        rmp_serde::from_slice::<ClientMsg>(&packed).unwrap(),
        expected
    );
}

#[test]
fn admit_variants_are_appended_last() {
    assert_eq!(
        variant_names::<ClientMsg>().last().map(String::as_str),
        Some("Admit")
    );
    let replies = variant_names::<DaemonMsg>();
    assert_eq!(replies[replies.len() - 2..], ["Admitted", "AdmitTimedOut"]);
}
