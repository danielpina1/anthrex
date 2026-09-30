//! Milestone 8b task 2: the scout role, the profile, scout, adaptation and history
//! types, and every new run request and reply.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::HISTORY_VERSION;
use crate::run_wire::{ProfileReply, ProfileRequest};
use crate::*;

/// M8a's round-trip fixtures, every field set. `run_tests.rs` loads the same file as its
/// own private module; loading it here too keeps both test modules private.
#[allow(clippy::duplicate_mod)]
#[path = "run_tests_fixtures.rs"]
mod fixtures;
use fixtures::{a_route, a_run_info, a_token_usage, a_tool_call};

/// Milestone 8b's builders.
#[path = "adapt_tests_fixtures.rs"]
mod adapt_fixtures;
use adapt_fixtures::*;

#[test]
fn agent_role_scout_serializes_as_scout() {
    assert_eq!(
        serde_json::to_string(&AgentRole::Scout).unwrap(),
        "\"scout\""
    );
    let back: AgentRole = serde_json::from_str("\"scout\"").unwrap();
    assert_eq!(back, AgentRole::Scout);
}

#[test]
fn repo_profile_parses_the_spec_example() {
    let p = a_profile();
    assert_eq!(p.languages, ["rust"]);
    assert_eq!(p.modules, ["crates/*"]);
    assert_eq!(p.hub, ["crates/proto/**"]);
    assert_eq!(p.generated, ["Cargo.lock"]);
    assert_eq!(p.protected, [".cursor/**"]);
    assert_eq!(p.setup.as_deref(), Some("cargo fetch"));
    assert_eq!(p.sample_test.as_deref(), Some("codec::round_trip"));
    assert_eq!(p.output_filter, OutputFilter::FailuresOnly);
    assert_eq!(p.filter_prefixes, ["cargo test", "cargo nextest"]);
    assert_eq!(p.conventions, ["AGENTS.md", "CLAUDE.md"]);
    assert_eq!(p.manifests, ["Cargo.toml", "crates/proto/Cargo.toml"]);
    assert_eq!(p.env["CARGO_TARGET_DIR"], "{worktree}/target");
    assert_eq!(p.test_passed.as_deref(), Some(r"test {test} \.\.\. ok"));
}

#[test]
fn repo_profile_rejects_unknown_keys() {
    let err = toml::from_str::<RepoProfile>("lint = \"x\"").unwrap_err();
    assert!(err.to_string().contains("lint"), "{err}");
}

#[test]
fn repo_profile_has_no_confinement_keys() {
    for (key, line) in [
        ("cache_dirs", "cache_dirs = [\"/tmp/c\"]"),
        ("confined_network", "confined_network = true"),
        (
            "confined_unix_sockets",
            "confined_unix_sockets = [\"/tmp/s\"]",
        ),
        (
            "confined_localhost_ports",
            "confined_localhost_ports = [5432]",
        ),
    ] {
        let err = toml::from_str::<RepoProfile>(line).unwrap_err();
        assert!(err.to_string().contains(key), "{key}: {err}");
    }
}

#[test]
fn repo_profile_protected_defaults_to_no_extras() {
    let p: RepoProfile = toml::from_str("check = \"make test\"").unwrap();
    assert!(p.protected.is_empty());
    assert_eq!(p.spec().protected, None);
}

#[test]
fn repo_profile_spec_maps_every_shared_key() {
    let mut p = a_profile();
    p.check_timeout_secs = Some(1800);
    let spec = p.spec();
    assert_eq!(spec.modules, Some(vec!["crates/*".to_string()]));
    assert_eq!(spec.hub, Some(vec!["crates/proto/**".to_string()]));
    assert_eq!(spec.source, Some(vec!["crates/*/src/**".to_string()]));
    assert_eq!(spec.generated, Some(vec!["Cargo.lock".to_string()]));
    assert_eq!(spec.protected, Some(vec![".cursor/**".to_string()]));
    assert_eq!(spec.setup.as_deref(), Some("cargo fetch"));
    assert_eq!(spec.check, p.check);
    assert_eq!(spec.check_timeout_secs, Some(1800));
    assert_eq!(spec.single_test, p.single_test);
    assert_eq!(spec.test_passed, p.test_passed);
    assert_eq!(spec.env, Some(p.env.clone()));

    let empty = RepoProfile::default().spec();
    assert_eq!(empty, ProfileSpec::default());
    assert_eq!(empty.modules, None);
    assert_eq!(empty.hub, None);
    assert_eq!(empty.source, None);
    assert_eq!(empty.generated, None);
    assert_eq!(empty.env, None);
}

#[test]
fn output_filter_kebab_case() {
    for (f, text) in [
        (OutputFilter::FailuresOnly, "\"failures-only\""),
        (OutputFilter::Tail, "\"tail\""),
        (OutputFilter::None, "\"none\""),
    ] {
        assert_eq!(serde_json::to_string(&f).unwrap(), text);
        assert_eq!(serde_json::from_str::<OutputFilter>(text).unwrap(), f);
    }
    assert_eq!(OutputFilter::default(), OutputFilter::FailuresOnly);
}

#[test]
fn scout_report_round_trips_json_and_msgpack() {
    let report = ScoutReport {
        id: "onboarding-1700000000".into(),
        kind: ScoutKind::Onboarding,
        run_id: None,
        question: "profile this repository".into(),
        summary: "a Rust workspace".into(),
        files: vec![ScoutFile {
            path: "Cargo.toml".into(),
            why: "the workspace manifest".into(),
        }],
        modules: vec!["crates/*".into()],
        interfaces: vec!["proto::PROTO_VERSION".into()],
        risks: vec!["no CI".into()],
        profile: Some(a_profile()),
        route: claude_route(),
        window_id: Some(12),
        started_at: 1_700_000_000,
        finished_at: 1_700_000_300,
        tool_calls: 14,
        usage: a_token_usage(),
    };
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"kind\":\"onboarding\""), "{json}");
    assert_eq!(serde_json::from_str::<ScoutReport>(&json).unwrap(), report);
    let packed = rmp_serde::to_vec_named(&report).unwrap();
    assert_eq!(
        rmp_serde::from_slice::<ScoutReport>(&packed).unwrap(),
        report
    );
    // The optional lists and the profile may be absent.
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    for key in ["modules", "interfaces", "risks", "profile"] {
        v.as_object_mut().unwrap().remove(key);
    }
    let short: ScoutReport = serde_json::from_value(v).unwrap();
    assert!(short.modules.is_empty() && short.risks.is_empty() && short.profile.is_none());
}

#[test]
fn history_lines_are_tagged() {
    let record = a_task_record();
    let line = HistoryLine::Task(record.clone());
    let json = serde_json::to_string(&line).unwrap();
    assert!(json.starts_with("{\"type\":\"task\""), "{json}");
    assert!(
        json.contains("\"outcome\":\"merged_without_approval\""),
        "{json}"
    );
    assert_eq!(serde_json::from_str::<HistoryLine>(&json).unwrap(), line);

    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v["added_by_a_later_milestone"] = serde_json::json!(7);
    let back: HistoryLine = serde_json::from_value(v).unwrap();
    assert_eq!(back, HistoryLine::Task(record));

    let run = HistoryLine::Run(RunRecord {
        v: HISTORY_VERSION,
        record_id: "run-a1b2".into(),
        at: 1_700_001_000,
        run_id: "run-a1b2".into(),
        goal: "Add password reset".into(),
        path: Some(RunPath::Fast),
        triage: Some(a_triage()),
        profile_source: Some(ProfileSource::Stored),
        outcome: "accepted".into(),
        base_branch: "main".into(),
        accepted_commit: Some("eeee5555".into()),
        tasks: 1,
        usage: Some(RunUsage::default()),
    });
    let revert = HistoryLine::Revert(RevertRecord {
        v: HISTORY_VERSION,
        record_id: "revert-ffff".into(),
        at: 1_700_002_000,
        run_id: "run-a1b2".into(),
        task_id: Some("t1".into()),
        reverted: "dddd4444".into(),
        revert_commit: "ffff6666".into(),
    });
    for (line, tag) in [(run, "run"), (revert, "revert")] {
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.starts_with(&format!("{{\"type\":\"{tag}\"")), "{json}");
        assert_eq!(serde_json::from_str::<HistoryLine>(&json).unwrap(), line);
    }
    assert_eq!(HISTORY_VERSION, 2);
}

#[test]
fn new_snapshot_fields_default_when_absent() {
    let mut run = a_run_info();
    run.tasks[0].last_check = Some(CheckInfo {
        at: 1_700_000_050,
        ok: false,
        code: Some(101),
        timed_out: false,
        secs: 30,
        summary: "test a ... FAILED".into(),
        on_candidate: false,
        decider_summary: Some("one test failed".into()),
        summary_source: Some(DeciderSource::Decider),
    });
    let mut v = serde_json::to_value(&run).unwrap();
    let r = v.as_object_mut().unwrap();
    for key in [
        "path",
        "triage",
        "promote_requested_at",
        "profile_source",
        "usage",
        "scouts",
    ] {
        assert!(r.remove(key).is_some(), "RunInfo.{key}");
    }
    let t = r["tasks"][0].as_object_mut().unwrap();
    for key in [
        "decider_usage",
        "size_check",
        "diff",
        "phases",
        "block_source",
    ] {
        assert!(t.remove(key).is_some(), "TaskInfo.{key}");
    }
    let c = t["last_check"].as_object_mut().unwrap();
    for key in ["decider_summary", "summary_source"] {
        assert!(c.remove(key).is_some(), "CheckInfo.{key}");
    }
    let back: RunInfo = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(back.path, None);
    assert_eq!(back.triage, None);
    assert_eq!(back.promote_requested_at, None);
    assert_eq!(back.profile_source, None);
    assert_eq!(back.usage, None);
    assert!(back.scouts.is_empty());
    let task = &back.tasks[0];
    assert_eq!(task.decider_usage, None);
    assert_eq!(task.size_check, None);
    assert_eq!(task.diff, None);
    assert_eq!(task.phases, None);
    assert_eq!(task.block_source, None);
    let check = task.last_check.as_ref().unwrap();
    assert_eq!(check.decider_summary, None);
    assert_eq!(check.summary_source, None);
    assert_eq!(check.summary, "test a ... FAILED");

    // The same milestone-8a shape over MessagePack, as the client reads a snapshot.
    let packed = rmp_serde::to_vec_named(&v).unwrap();
    let back: RunInfo = rmp_serde::from_slice(&packed).unwrap();
    assert!(back.scouts.is_empty() && back.tasks[0].phases.is_none());
}

#[test]
fn every_new_request_and_reply_round_trips() {
    let dir = PathBuf::from("/tmp/p");
    let requests = vec![
        RunRequest::StartGoal {
            goal: "fix the typo in README".into(),
            dir: dir.clone(),
            yes: true,
            trust_project: true,
            unconfined_checks: false,
            orchestrator: None,
        },
        RunRequest::Promote {
            run_id: "run-a1b2".into(),
            orchestrator: None,
        },
        RunRequest::Stats { dir: dir.clone() },
        RunRequest::Profile(ProfileRequest::Status { dir: dir.clone() }),
        RunRequest::Profile(ProfileRequest::Detect {
            dir: dir.clone(),
            trust_project: true,
            unconfined_checks: false,
        }),
        RunRequest::Profile(ProfileRequest::Show {
            dir: dir.clone(),
            proposed: true,
        }),
        RunRequest::Profile(ProfileRequest::Confirm {
            dir: dir.clone(),
            shown: Some("check = \"cargo test\"\n".into()),
        }),
        RunRequest::Profile(ProfileRequest::Confirm {
            dir: dir.clone(),
            shown: None,
        }),
        RunRequest::Profile(ProfileRequest::Reject { dir: dir.clone() }),
        RunRequest::Profile(ProfileRequest::Edit {
            dir: dir.clone(),
            key: "check".into(),
            value: Some("cargo test".into()),
            yes: true,
            unconfined_checks: false,
        }),
        RunRequest::Profile(ProfileRequest::Edit {
            dir: dir.clone(),
            key: "env.RUST_LOG".into(),
            value: None,
            yes: false,
            unconfined_checks: true,
        }),
    ];
    for request in requests {
        let msg = ClientMsg::Run(request);
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        assert_eq!(rmp_serde::from_slice::<ClientMsg>(&packed).unwrap(), msg);
    }

    let mut by_role = BTreeMap::new();
    by_role.insert("decider".to_string(), a_token_usage());
    let status = ProfileStatus {
        project: dir.clone(),
        repo_dir: PathBuf::from("/tmp/data/repos/p-1234"),
        source: ProfileSource::Stored,
        confirmed_at: Some(1_700_000_400),
        stale: vec!["Cargo.toml".into()],
        unparseable: Some("line 3: unknown key".into()),
        proposal: Some(a_proposal()),
        scout: Some(a_scout_info()),
        verify_confined: true,
    };
    let mut fingerprint = BTreeMap::new();
    fingerprint.insert("Cargo.toml".to_string(), "0123456789abcdef:42".to_string());
    fingerprint.insert("CLAUDE.md".to_string(), "missing".to_string());
    let replies = vec![
        RunReply::Triaged {
            triage: a_triage(),
            run_id: Some("run-a1b2".into()),
            message: "fast path".into(),
            request_id: None,
        },
        RunReply::profile(ProfileReply::Status(status)),
        RunReply::profile(ProfileReply::Shown {
            source: ProfileSource::None,
            toml: SPEC_PROFILE.into(),
            meta: Some(ProfileMeta {
                confirmed_at: 1_700_000_400,
                report: Some("onboarding-1700000000".into()),
                verification: Some(a_verification()),
                fingerprint,
                edited_keys: vec!["check".into()],
                project: Some("/work/app".into()),
            }),
            verification: Some(a_verification()),
            dropped: vec![a_dropped()],
        }),
        RunReply::profile(ProfileReply::Done {
            message: "stored".into(),
        }),
        RunReply::profile(ProfileReply::Refused {
            message: "no proposal".into(),
        }),
        RunReply::stats(a_stats()),
    ];
    for reply in replies {
        let msg = DaemonMsg::Run(reply);
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        assert_eq!(rmp_serde::from_slice::<DaemonMsg>(&packed).unwrap(), msg);
    }

    // Every proposal state and origin, and a snapshot carrying every new field.
    for state in [
        ProposalState::Preparing,
        ProposalState::Scouting,
        ProposalState::Verifying,
        ProposalState::Ready,
    ] {
        let p = ProposalRecord {
            state,
            ..a_proposal()
        };
        let packed = rmp_serde::to_vec_named(&p).unwrap();
        assert_eq!(rmp_serde::from_slice::<ProposalRecord>(&packed).unwrap(), p);
    }
    for origin in [
        ProposalOrigin::Detect,
        ProposalOrigin::Goal,
        ProposalOrigin::Edit {
            keys: vec!["check".into()],
        },
    ] {
        let p = ProposalRecord {
            origin,
            ..a_proposal()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<ProposalRecord>(&json).unwrap(), p);
    }
    let mut run = a_run_info();
    run.path = Some(RunPath::Fast);
    run.triage = Some(a_triage());
    run.promote_requested_at = Some(1_700_000_700);
    run.profile_source = Some(ProfileSource::Plan);
    run.usage = Some(RunUsage {
        total: a_token_usage(),
        by_role,
        decider_calls: 3,
        decider_fallbacks: 1,
    });
    run.scouts = vec![a_scout_info()];
    run.tasks[0].size_check = Some(a_size_check());
    run.tasks[0].decider_usage = Some(a_token_usage());
    run.tasks[0].diff = Some(DiffStats::default());
    run.tasks[0].phases = Some(PhaseSecs::default());
    run.tasks[0].block_source = Some(DeciderSource::Decider);
    let msg = DaemonMsg::Run(RunReply::Snapshot(RunsSnapshot {
        revision: 1,
        runs: vec![run],
        now: 0,
        proposals: Vec::new(),
    }));
    let packed = rmp_serde::to_vec_named(&msg).unwrap();
    assert_eq!(rmp_serde::from_slice::<DaemonMsg>(&packed).unwrap(), msg);

    // A tool call from a scout carries its id.
    let call = ToolCall {
        role: AgentRole::Scout,
        scout_id: Some("onboarding-1700000000".into()),
        ..a_tool_call()
    };
    let msg = ClientMsg::Run(RunRequest::Tool(call));
    let packed = rmp_serde::to_vec_named(&msg).unwrap();
    assert_eq!(rmp_serde::from_slice::<ClientMsg>(&packed).unwrap(), msg);
}

#[test]
fn every_new_enum_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&DeciderSource::Fallback).unwrap(),
        "\"fallback\""
    );
    assert_eq!(serde_json::to_string(&RunPath::Large).unwrap(), "\"large\"");
    assert_eq!(serde_json::to_string(&Scale::Plan).unwrap(), "\"plan\"");
    assert_eq!(
        serde_json::to_string(&ScoutState::Reported).unwrap(),
        "\"reported\""
    );
    assert_eq!(
        serde_json::to_string(&ProfileSource::Stored).unwrap(),
        "\"stored\""
    );
    assert_eq!(DeciderMode::default(), DeciderMode::Claude);
    assert_eq!(serde_json::to_string(&DeciderMode::Off).unwrap(), "\"off\"");
    let state = serde_json::to_string(&ProposalState::Failed { reason: "x".into() }).unwrap();
    assert_eq!(state, r#"{"state":"failed","reason":"x"}"#);
    let origin = serde_json::to_string(&ProposalOrigin::Detect).unwrap();
    assert_eq!(origin, r#"{"origin":"detect"}"#);
}

#[test]
fn tool_call_scout_id_defaults_to_none() {
    let mut v = serde_json::to_value(a_tool_call()).unwrap();
    v.as_object_mut().unwrap().remove("scout_id");
    let back: ToolCall = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(back.scout_id, None);
    let packed = rmp_serde::to_vec_named(&v).unwrap();
    let back: ToolCall = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(back.scout_id, None);
}

#[test]
fn token_usage_add_assign_is_field_wise() {
    let mut total = TokenUsage {
        input: 1,
        output: 20,
        cache_read: 300,
        cache_write: 4000,
    };
    total += TokenUsage {
        input: 50_000,
        output: 600_000,
        cache_read: 7_000_000,
        cache_write: 80_000_000,
    };
    assert_eq!(
        total,
        TokenUsage {
            input: 50_001,
            output: 600_020,
            cache_read: 7_000_300,
            cache_write: 80_004_000,
        }
    );
}

/// M8b.15 review: `+=` saturates field by field, so a huge sum never panics (debug) or
/// wraps (release).
#[test]
fn token_usage_add_assign_saturates() {
    let mut total = TokenUsage {
        input: u64::MAX - 1,
        output: 5,
        cache_read: u64::MAX,
        cache_write: 1,
    };
    total += TokenUsage {
        input: 10,
        output: 7,
        cache_read: u64::MAX,
        cache_write: u64::MAX,
    };
    assert_eq!(
        total,
        TokenUsage {
            input: u64::MAX,
            output: 12,
            cache_read: u64::MAX,
            cache_write: u64::MAX,
        }
    );
}

/// M8b.11 review (I4, m3): a meta written before `project` existed, and a `Confirm`
/// without `shown`, still read.
#[test]
fn meta_project_and_confirm_shown_default_when_absent() {
    let meta: crate::ProfileMeta = serde_json::from_str(
        r#"{"confirmed_at":1,"report":null,"verification":null,"fingerprint":{},"edited_keys":[]}"#,
    )
    .unwrap();
    assert_eq!(meta.project, None);
    let confirm: crate::ProfileRequest =
        serde_json::from_str(r#"{"Confirm":{"dir":"/work/app"}}"#).unwrap();
    assert_eq!(
        confirm,
        crate::ProfileRequest::Confirm {
            dir: "/work/app".into(),
            shown: None
        }
    );
}

/// Decision 33a: a task record keeps its routing decisions through JSON and MessagePack,
/// and a line written before them reads as none.
#[test]
fn routing_decisions_round_trip_and_default_to_none() {
    let record = a_task_record();
    assert_eq!(record.routing_decisions.len(), 1);
    let packed = rmp_serde::to_vec_named(&record).unwrap();
    assert_eq!(
        rmp_serde::from_slice::<TaskRecord>(&packed).unwrap(),
        record
    );
    let mut v = serde_json::to_value(HistoryLine::Task(record.clone())).unwrap();
    assert_eq!(
        v["routing_decisions"][0]["pick_policy"],
        serde_json::Value::Null
    );
    v.as_object_mut().unwrap().remove("routing_decisions");
    let HistoryLine::Task(old) = serde_json::from_value(v).unwrap() else {
        panic!("a task line");
    };
    assert!(old.routing_decisions.is_empty());
    let mut decision = serde_json::to_value(a_routing_decision()).unwrap();
    decision.as_object_mut().unwrap().remove("pick_policy");
    let back: RoutingDecision = serde_json::from_value(decision).unwrap();
    assert_eq!(back, a_routing_decision());
}
