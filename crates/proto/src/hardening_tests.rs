use serde_json::json;

use crate::*;

fn both<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    let back: T = serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap();
    assert_eq!(&back, v);
    let back: T = rmp_serde::from_slice(&rmp_serde::to_vec_named(v).unwrap()).unwrap();
    assert_eq!(&back, v);
}

fn info() -> OrchestratorInfo {
    OrchestratorInfo {
        route: serde_json::from_value(json!({"runtime": "claude", "model": "", "effort": "high"}))
            .unwrap(),
        window_id: Some(9),
        live: true,
        started_at: 1,
        plan_submitted: true,
        summary: None,
        notes: vec![],
        wakes: 2,
        wake_held: false,
        stuck: None,
        ask: None,
        handled: vec![],
        handled_total: 0,
        untrusted: None,
    }
}

fn untrusted() -> UntrustedInfo {
    UntrustedInfo {
        at: 1_760_000_000,
        source: "review comments on stage 2's pull request".into(),
    }
}

#[test]
fn untrusted_info_round_trips_json_and_msgpack() {
    both(&untrusted());
    let mut with = info();
    with.untrusted = Some(untrusted());
    both(&with);
}

#[test]
fn orchestrator_info_without_untrusted_decodes_and_omits_it() {
    let mut value = serde_json::to_value(info()).unwrap();
    assert!(value.get("untrusted").is_none());
    value.as_object_mut().unwrap().remove("untrusted");
    let back: OrchestratorInfo = serde_json::from_value(value).unwrap();
    assert_eq!(back.untrusted, None);
}
