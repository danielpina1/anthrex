//! Task M9.16.2: protocol 25's brake types (decisions 2, 3, 15, 17, 30, 33, 42) and
//! the fields they add, round-tripped through JSON and MessagePack.

use serde_json::{Value, json};

use crate::brakes::*;
use crate::delivery::tests::variant_names;
use crate::run_wire::request;
use crate::settings::{SettingsBrakes, key};
use crate::*;

fn both<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    let back: T = serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap();
    assert_eq!(&back, v);
    let back: T = rmp_serde::from_slice(&rmp_serde::to_vec_named(v).unwrap()).unwrap();
    assert_eq!(&back, v);
}

fn claude_route() -> Route {
    Route {
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        effort: Effort::HIGH,
    }
}

fn full_brakes() -> BrakesInfo {
    BrakesInfo {
        hold: Some(DispatchHold {
            since: 1_791_890_000,
            by: "user".into(),
        }),
        quota_brake: true,
        quota: vec![
            QuotaInfo {
                runtime: Runtime::Claude,
                state: QuotaState::Braked,
                windows: vec![
                    QuotaWindowInfo {
                        window: QuotaWindow::FiveHour,
                        used: Permille(940),
                        resets_at: Some(1_791_900_000),
                    },
                    QuotaWindowInfo {
                        window: QuotaWindow::SevenDay,
                        used: Permille(530),
                        resets_at: None,
                    },
                ],
                read_at: Some(1_791_889_000),
                trip: Some(BrakeTripInfo {
                    window: QuotaWindow::FiveHour,
                    used: Permille(940),
                    threshold_percent: 90,
                    until: 1_791_900_000,
                }),
            },
            QuotaInfo {
                runtime: Runtime::Codex,
                state: QuotaState::Unmeasured,
                windows: vec![],
                read_at: None,
                trip: None,
            },
        ],
    }
}

fn history_lines(text: &str) -> Vec<HistoryLine> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

#[test]
fn permille_from_fraction_and_percent() {
    assert_eq!(Permille::from_fraction(0.53), Some(Permille(530)));
    assert_eq!(Permille::from_fraction(0.94), Some(Permille(940)));
    assert_eq!(Permille::from_percent(99.0), Some(Permille(990)));
    assert_eq!(Permille::from_percent(14.0), Some(Permille(140)));
    assert_eq!(Permille::from_fraction(1.7), Some(Permille::FULL));
    assert_eq!(Permille::FULL, Permille(1000));
    assert_eq!(Permille::from_fraction(-0.1), None);
    assert_eq!(Permille::from_fraction(f64::NAN), None);
    assert_eq!(Permille::from_percent(f64::INFINITY), None);
    assert!(Permille(905).at_least(90));
    assert!(!Permille(899).at_least(90));
    assert_eq!(Permille(949).percent(), 94);
    // Transparent on the wire: a bare number.
    assert_eq!(serde_json::to_value(Permille(530)).unwrap(), json!(530));
}

#[test]
fn quota_window_maps_minutes_and_keys() {
    assert_eq!(
        QuotaWindow::from_codex_minutes(300),
        Some(QuotaWindow::FiveHour)
    );
    assert_eq!(
        QuotaWindow::from_codex_minutes(10080),
        Some(QuotaWindow::SevenDay)
    );
    assert_eq!(QuotaWindow::from_codex_minutes(60), None);
    assert_eq!(
        QuotaWindow::from_claude_key("five_hour"),
        Some(QuotaWindow::FiveHour)
    );
    assert_eq!(
        QuotaWindow::from_claude_key("seven_day"),
        Some(QuotaWindow::SevenDay)
    );
    assert_eq!(QuotaWindow::from_claude_key("seven_day_opus"), None);
    assert_eq!(QuotaWindow::FiveHour.label(), "5h");
    assert_eq!(QuotaWindow::SevenDay.label(), "7d");
    assert_eq!(QuotaWindow::FiveHour.secs(), FIVE_HOUR_SECS);
    assert_eq!(QuotaWindow::SevenDay.secs(), SEVEN_DAY_SECS);
    assert_eq!((FIVE_HOUR_SECS, SEVEN_DAY_SECS), (18_000, 604_800));
    assert_eq!(
        serde_json::to_value(QuotaWindow::FiveHour).unwrap(),
        json!("five_hour")
    );
    assert_eq!(
        serde_json::to_value(QuotaWindow::SevenDay).unwrap(),
        json!("seven_day")
    );
}

#[test]
fn brakes_info_round_trips_and_is_left_out_when_empty() {
    let full = full_brakes();
    assert!(!full.is_empty());
    both(&full);
    assert!(BrakesInfo::default().is_empty());

    let mut snapshot = RunsSnapshot {
        revision: 3,
        runs: vec![],
        now: 1_791_890_100,
        proposals: vec![],
        idle_orchestrators: vec![],
        queued_goals: vec![],
        brakes: BrakesInfo::default(),
    };
    let wire = serde_json::to_value(&snapshot).unwrap();
    assert!(wire.get("brakes").is_none(), "{wire}");
    both(&snapshot);
    snapshot.brakes = full;
    let wire = serde_json::to_value(&snapshot).unwrap();
    assert!(wire.get("brakes").is_some(), "{wire}");
    both(&snapshot);
}

#[test]
fn run_info_handoff_round_trips_and_is_optional() {
    let mut info: RunInfo = serde_json::from_str(include_str!("m912_run_info.json")).unwrap();
    assert_eq!(info.handoff, None);
    let wire = serde_json::to_value(&info).unwrap();
    assert!(wire.get("handoff").is_none(), "{wire}");
    both(&info);
    info.handoff = Some(HandoffInfo {
        from_run: "add-a-ff9f".into(),
        fill_tokens: 162_000,
        window_tokens: Some(200_000),
        at: 1_791_890_200,
    });
    both(&info);
    info.handoff = Some(HandoffInfo {
        from_run: "add-a-ff9f".into(),
        fill_tokens: 310_000,
        window_tokens: None,
        at: 1_791_890_200,
    });
    both(&info);
}

#[test]
fn hold_request_and_held_reply_are_appended_last() {
    let requests = variant_names::<RunRequest>();
    assert_eq!(requests.last().map(String::as_str), Some("Hold"));
    assert_eq!(requests[requests.len() - 2], "Lessons");
    let replies = variant_names::<RunReply>();
    assert_eq!(replies.last().map(String::as_str), Some("Held"));
    assert_eq!(replies[replies.len() - 2], "LessonsGraph");
    assert_eq!(request::HOLD, "hold");

    both(&RunRequest::Hold { on: true });
    both(&RunRequest::Hold { on: false });
    let held = RunReply::Held {
        hold: Some(DispatchHold {
            since: 1_791_890_000,
            by: "user".into(),
        }),
        message: "dispatch held daemon-wide".into(),
        request_id: None,
    };
    both(&held);
    let released = RunReply::Held {
        hold: None,
        message: "dispatch released".into(),
        request_id: Some(4),
    };
    both(&released);
    let tagged = held.tagged(Some(7));
    assert_eq!(tagged.request_id(), Some(7));
    both(&tagged);
}

#[test]
fn settings_doc_brakes_default_when_missing() {
    let without: Value = json!({
        "limits": {
            "budget_s": {"tool_calls": 40, "minutes": 15},
            "budget_m": {"tool_calls": 80, "minutes": 30},
            "budget_l": {"tool_calls": 160, "minutes": 60},
            "stall_after_secs": 600,
            "max_writers": 3,
            "max_readers": 4,
            "max_bounces": 2
        }
    });
    let doc: SettingsDoc = serde_json::from_value(without.clone()).unwrap();
    assert_eq!(
        doc.brakes,
        SettingsBrakes {
            quota_brake: true,
            quota_pause_5h_percent: 90,
            quota_pause_7d_percent: 95,
            quota_fresh_secs: 600,
        }
    );
    assert_eq!(doc.brakes, SettingsBrakes::default());
    let doc: SettingsDoc =
        rmp_serde::from_slice(&rmp_serde::to_vec_named(&without).unwrap()).unwrap();
    assert_eq!(doc.brakes, SettingsBrakes::default());

    let mut doc = doc;
    doc.brakes = SettingsBrakes {
        quota_brake: false,
        quota_pause_5h_percent: 80,
        quota_pause_7d_percent: 99,
        quota_fresh_secs: 3600,
    };
    both(&doc);

    assert_eq!(SETTINGS_KEYS.len(), 15);
    assert_eq!(
        SETTINGS_KEYS[11..],
        [
            key::QUOTA_BRAKE,
            key::QUOTA_PAUSE_5H_PERCENT,
            key::QUOTA_PAUSE_7D_PERCENT,
            key::QUOTA_FRESH_SECS,
        ]
    );
    assert_eq!(key::QUOTA_BRAKE, "orchestrator.brakes.quota_brake");
    assert_eq!(
        key::QUOTA_PAUSE_5H_PERCENT,
        "orchestrator.brakes.quota_pause_5h_percent"
    );
    assert_eq!(
        key::QUOTA_PAUSE_7D_PERCENT,
        "orchestrator.brakes.quota_pause_7d_percent"
    );
    assert_eq!(
        key::QUOTA_FRESH_SECS,
        "orchestrator.brakes.quota_fresh_secs"
    );
}

#[test]
fn run_record_handoff_is_optional_and_history_version_stays() {
    assert_eq!(HISTORY_VERSION, 6);
    let mut runs = history_lines(include_str!("m914_history.jsonl"))
        .into_iter()
        .filter_map(|line| match line {
            HistoryLine::Run(record) => Some(record),
            _ => None,
        });
    let mut record = runs.next().expect("m914_history.jsonl has a run line");
    assert_eq!(record.handoff, None);
    let wire = serde_json::to_value(&record).unwrap();
    assert!(wire.get("handoff").is_none(), "{wire}");
    both(&record);
    record.handoff = Some(HandoffInfo {
        from_run: "add-a-ff9f".into(),
        fill_tokens: 162_000,
        window_tokens: Some(200_000),
        at: 1_791_890_200,
    });
    both(&HistoryLine::Run(record));
}

#[test]
fn routing_decisions_failover_is_optional() {
    let mut task_decisions = vec![];
    let mut role_decisions = vec![];
    for text in [
        include_str!("m9_history_v2.jsonl"),
        include_str!("m914_history.jsonl"),
        include_str!("m93_history.jsonl"),
    ] {
        for line in history_lines(text) {
            match line {
                HistoryLine::Task(task) => task_decisions.extend(task.routing_decisions),
                HistoryLine::RoleRoute(role) => role_decisions.push(role),
                _ => {}
            }
        }
    }
    assert!(task_decisions.len() >= 3, "{}", task_decisions.len());
    assert!(!role_decisions.is_empty());
    assert!(task_decisions.iter().all(|d| d.failover.is_none()));
    assert!(role_decisions.iter().all(|d| d.failover.is_none()));

    let mut decision = task_decisions.remove(0);
    let wire = serde_json::to_value(&decision).unwrap();
    assert!(wire.get("failover").is_none(), "{wire}");
    both(&decision);
    decision.failover = Some(claude_route());
    both(&decision);

    let mut decision = role_decisions.remove(0);
    let wire = serde_json::to_value(&decision).unwrap();
    assert!(wire.get("failover").is_none(), "{wire}");
    both(&decision);
    decision.failover = Some(claude_route());
    both(&HistoryLine::RoleRoute(decision));
}

/// C24: no snapshot fixture is checked in, so the old shape is built here; the run
/// half (`m95-run.json`) is the daemon's `a_format_4_run_loads`.
#[test]
fn old_snapshots_and_runs_still_decode() {
    let info: RunInfo = serde_json::from_str(include_str!("m912_run_info.json")).unwrap();
    assert_eq!(info.handoff, None);
    let old = json!({
        "revision": 9,
        "runs": [serde_json::from_str::<Value>(include_str!("m912_run_info.json")).unwrap()],
        "now": 1_791_890_100u64,
        "proposals": [],
    });
    let snapshot: RunsSnapshot = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(snapshot.brakes, BrakesInfo::default());
    assert_eq!(snapshot.runs[0].handoff, None);
    let snapshot: RunsSnapshot =
        rmp_serde::from_slice(&rmp_serde::to_vec_named(&old).unwrap()).unwrap();
    assert_eq!(snapshot.brakes, BrakesInfo::default());
}
