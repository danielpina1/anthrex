//! Milestone 9.1 task 3: the stage, tier, origin, profile-key and history types, and
//! that what milestone 9 wrote (protocol 10, history version 2) still loads.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::HISTORY_VERSION;
use crate::history::{BisectLine, FlakyProposal, FlakyRecord, TierRunRecord};
use crate::messages::DaemonMsg;
use crate::profile::ModuleNames;
use crate::run_wire::RunReply;
use crate::tiers::*;
use crate::*;

/// M8a's round-trip fixtures, loaded as this module's own private copy. `a_route` is
/// imported for `adapt_fixtures`, which reads it through `use super::*`.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "run_tests_fixtures.rs"]
mod fixtures;
use fixtures::{a_route, a_run_info, a_token_usage};

/// Milestone 8b's builders, for its profile and history records.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "adapt_tests_fixtures.rs"]
mod adapt_fixtures;
use adapt_fixtures::{SPEC_PROFILE, a_command_check, a_stats, a_task_record, a_verification};

fn both_ways<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let packed = rmp_serde::to_vec_named(value).unwrap();
    let back: T = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(&back, value, "MessagePack");
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value, "JSON");
}

fn a_full_info() -> FullInfo {
    FullInfo {
        state: FullState::Red,
        at: Some(1_700_000_100),
        secs: Some(2_460),
        commit: Some("cccc3333".into()),
        shards: 4,
        flaky: vec!["codec::slow_path".into()],
        failing: vec!["daemon::run::merges".into()],
        bisect_fixes: 1,
        note: Some("no single culprit: two merges".into()),
    }
}

fn a_stage(head: Option<&str>) -> StageInfo {
    StageInfo {
        n: 2,
        branch: "anthrex/run-a1b2/stage-2".into(),
        head: head.map(str::to_string),
        tasks: 5,
        merged: 4,
        full: a_full_info(),
        fix_tasks: vec!["fix1".into()],
        propagate_red: Some("bbbb2222".into()),
        pr: None,
    }
}

fn a_tier_info() -> TierInfo {
    TierInfo {
        tier: 1,
        affected: "3 modules (a, b, c)".into(),
        steps: 3,
        cached: 1,
        ok: true,
        secs: 130,
        flaky: vec!["t_x".into()],
    }
}

fn a_signal() -> SignalInfo {
    SignalInfo {
        id: "W1".into(),
        kind: "skip_marker".into(),
        path: "crates/a/src/lib.rs".into(),
        line: Some(42),
        text: "#[ignore]".into(),
        answered: Some("accepted: the test needs a network".into()),
    }
}

#[test]
fn tiers_types_round_trip() {
    for (origin, text) in [
        (TaskOrigin::Plan, "plan"),
        (TaskOrigin::Bisect, "bisect"),
        (TaskOrigin::Sync, "sync"),
        (TaskOrigin::Ci, "ci"),
        (TaskOrigin::Review, "review"),
    ] {
        both_ways(&origin);
        assert_eq!(
            serde_json::to_string(&origin).unwrap(),
            format!("\"{text}\"")
        );
    }
    assert_eq!(TaskOrigin::default(), TaskOrigin::Plan);
    for (state, text) in [
        (FullState::None, "none"),
        (FullState::Running, "running"),
        (FullState::Green, "green"),
        (FullState::Red, "red"),
        (FullState::Bisecting, "bisecting"),
    ] {
        both_ways(&state);
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            format!("\"{text}\"")
        );
    }
    assert_eq!(FullState::default(), FullState::None);
    both_ways(&a_stage(Some("aaaa1111")));
    both_ways(&a_stage(None));
    both_ways(&StageInfo {
        full: FullInfo::default(),
        ..a_stage(None)
    });
    both_ways(&a_tier_info());
    both_ways(&a_signal());
    both_ways(&SignalInfo {
        line: None,
        answered: None,
        ..a_signal()
    });
}

/// A plan task as milestone 9 wrote it: no `stage`, `atomic` or `atomic_reason`.
const M9_PLAN: &str = r#"
goal = "Add password reset"

[[task]]
id = "t1"
title = "Reset token model"
size = "M"
owns = ["crates/auth/src/token.rs"]
brief = "..."
acceptance = ["..."]
review_target = "main..feature"
"#;

#[test]
fn plan_task_stage_defaults_to_one() {
    let plan: Plan = toml::from_str(M9_PLAN).expect("an M9 plan file parses");
    let task = &plan.tasks[0];
    assert_eq!(task.stage, 1);
    assert!(!task.atomic);
    assert_eq!(task.atomic_reason, None);

    // The same task as M9's JSON (an `edit_plan` argument or a `run.json` spec).
    let mut json = serde_json::to_value(task).unwrap();
    let map = json.as_object_mut().unwrap();
    for key in ["stage", "atomic", "atomic_reason"] {
        assert!(map.remove(key).is_some(), "{key} is a field");
    }
    let old: PlanTask = serde_json::from_value(json).unwrap();
    assert_eq!((old.stage, old.atomic), (1, false));

    let staged = PlanTask {
        stage: 3,
        atomic: true,
        atomic_reason: Some("a protocol bump updates every client".into()),
        ..task.clone()
    };
    both_ways(&staged);
    let text = toml::to_string(&Plan {
        tasks: vec![staged.clone()],
        ..plan.clone()
    })
    .unwrap();
    assert_eq!(toml::from_str::<Plan>(&text).unwrap().tasks[0], staged);
}

fn an_amend(stage: Option<u16>) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: "t3".into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage,
    }
}

#[test]
fn amend_task_stage_round_trips() {
    let edit = an_amend(Some(2));
    both_ways(&edit);
    let file = EditFile { edits: vec![edit] };
    let text = toml::to_string(&file).unwrap();
    assert!(text.contains("stage = 2"), "{text}");
    assert_eq!(toml::from_str::<EditFile>(&text).unwrap(), file);

    // Milestone 9's amend, with no `stage` key.
    let m9 = r#"{"op":"amend_task","task_id":"t3","priority":5}"#;
    let PlanEdit::AmendTask {
        stage, priority, ..
    } = serde_json::from_str(m9).unwrap()
    else {
        panic!("an amend_task edit");
    };
    assert_eq!((stage, priority), (None, Some(5)));
}

/// Every new key of decision 5, each to a value no sibling of the same type has.
const TIER_KEYS: &str = r##"
build_check = "cargo build --workspace --all-targets"
module_test = "cargo nextest run -p {module} {filter:-E %}"
module_tests = "cargo nextest run {modules:-p %} {filter:-E %}"
module_graph = "cargo"
module_names = "cargo"
full_triggers = ["Cargo.lock", "rust-toolchain.toml"]
slow_tests = "test(/slow/)"
timing_tests = "test(/timing/)"
skip_markers = ["#[ignore]", "@pytest.mark.skip"]
test_paths = ["crates/*/tests/**"]
full_shards = 4
toolchain_id = "rustc -vV"
"##;

#[test]
fn profile_keys_round_trip() {
    // The stored profile: M8b's keys, then every new one.
    let stored = SPEC_PROFILE.replacen("[env]\n", &format!("{TIER_KEYS}[env]\n"), 1);
    let profile: RepoProfile = toml::from_str(&stored).expect("a tiered profile parses");
    assert_eq!(
        profile.build_check.as_deref(),
        Some("cargo build --workspace --all-targets")
    );
    assert_eq!(profile.module_graph.as_deref(), Some("cargo"));
    assert_eq!(profile.module_names, Some(ModuleNames::Cargo));
    assert_eq!(profile.full_triggers, ["Cargo.lock", "rust-toolchain.toml"]);
    assert_eq!(profile.slow_tests.as_deref(), Some("test(/slow/)"));
    assert_eq!(profile.timing_tests.as_deref(), Some("test(/timing/)"));
    assert_eq!(profile.skip_markers, ["#[ignore]", "@pytest.mark.skip"]);
    assert_eq!(profile.test_paths, ["crates/*/tests/**"]);
    assert_eq!(profile.full_shards, Some(4));
    assert_eq!(profile.toolchain_id.as_deref(), Some("rustc -vV"));
    both_ways(&profile);
    let text = toml::to_string(&profile).unwrap();
    assert_eq!(toml::from_str::<RepoProfile>(&text).unwrap(), profile);

    // `spec()` carries every key to the plan's shape.
    let spec = profile.spec();
    assert_eq!(spec.build_check, profile.build_check);
    assert_eq!(spec.module_test, profile.module_test);
    assert_eq!(spec.module_tests, profile.module_tests);
    assert_eq!(spec.module_graph, profile.module_graph);
    assert_eq!(spec.module_names, Some(ModuleNames::Cargo));
    assert_eq!(spec.full_triggers.as_ref(), Some(&profile.full_triggers));
    assert_eq!(spec.slow_tests, profile.slow_tests);
    assert_eq!(spec.timing_tests, profile.timing_tests);
    assert_eq!(spec.skip_markers.as_ref(), Some(&profile.skip_markers));
    assert_eq!(spec.test_paths.as_ref(), Some(&profile.test_paths));
    assert_eq!(spec.full_shards, Some(4));
    assert_eq!(spec.toolchain_id, profile.toolchain_id);
    both_ways(&spec);

    // A plan's `[profile]` with every new key.
    let plan = format!("goal = \"g\"\ntask = []\n[profile]\n{TIER_KEYS}");
    let plan: Plan = toml::from_str(&plan).expect("a plan with every new key parses");
    let tier_keys_only = ProfileSpec {
        modules: None,
        hub: None,
        source: None,
        check: None,
        single_test: None,
        test_passed: None,
        setup: None,
        generated: None,
        protected: None,
        env: None,
        ..spec.clone()
    };
    assert_eq!(plan.profile, tier_keys_only);

    // M8b's profile still decodes, every new key absent; its spec sets none.
    let old: RepoProfile = toml::from_str(SPEC_PROFILE).expect("an M8b profile parses");
    assert_eq!(old.build_check, None);
    assert_eq!(old.module_names, None);
    assert!(old.full_triggers.is_empty() && old.skip_markers.is_empty());
    assert!(old.test_paths.is_empty());
    assert_eq!(old.full_shards, None);
    let old_spec = old.spec();
    assert_eq!((old_spec.full_triggers, old_spec.test_paths), (None, None));
    assert_eq!(old_spec.skip_markers, None);

    // An unknown key is still refused, in both shapes.
    let bogus = format!("{SPEC_PROFILE}\nbogus_tier_key = 1\n").replacen(
        "[env]\n",
        "bogus_tier_key = 1\n[env]\n",
        1,
    );
    let err = toml::from_str::<RepoProfile>(&bogus).unwrap_err();
    assert!(err.to_string().contains("bogus_tier_key"), "{err}");
    let err = toml::from_str::<ProfileSpec>("module_shards = 2\n").unwrap_err();
    assert!(err.to_string().contains("module_shards"), "{err}");

    for (names, text) in [(ModuleNames::Cargo, "cargo"), (ModuleNames::Dir, "dir")] {
        both_ways(&names);
        assert_eq!(
            serde_json::to_string(&names).unwrap(),
            format!("\"{text}\"")
        );
    }
}

#[test]
fn verification_records_round_trip() {
    let full = ProfileVerification {
        build_check: Some(a_command_check("cargo build", true)),
        module_graph: Some(a_command_check("cargo metadata", true)),
        module_test: Some(a_command_check("cargo nextest run -p a", false)),
        module_tests: Some(a_command_check("cargo nextest run -p a -p b", true)),
        toolchain_id: Some(a_command_check("rustc -vV", true)),
        ..a_verification()
    };
    both_ways(&full);
    let meta = ProfileMeta {
        confirmed_at: 1_700_000_600,
        report: None,
        verification: Some(full.clone()),
        fingerprint: BTreeMap::new(),
        edited_keys: vec!["module_test".into()],
        project: Some(PathBuf::from("/tmp/p")),
    };
    both_ways(&meta);

    // M8b's record: none of the five keys.
    let mut json = serde_json::to_value(a_verification()).unwrap();
    let map = json.as_object_mut().unwrap();
    for key in [
        "build_check",
        "module_graph",
        "module_test",
        "module_tests",
        "toolchain_id",
    ] {
        assert!(map.remove(key).is_some(), "{key} is a field");
    }
    let old: ProfileVerification = serde_json::from_value(json).unwrap();
    assert_eq!(old, a_verification());
    assert_eq!(old.build_check, None);
}

/// `m9_run_info.json` is milestone 9's `orchestrator_snapshot_fields_round_trip` run,
/// serialized by milestone 9's own `RunInfo` before milestone 9.1 changed it. It
/// decodes with every milestone-9.1 field at its default.
#[test]
fn old_run_info_still_decodes() {
    let run: RunInfo =
        serde_json::from_str(include_str!("m9_run_info.json")).expect("an M9 RunInfo decodes");
    assert_eq!(run.run_id, a_run_info().run_id);
    assert_eq!(run.digest_revision, 17, "an M9 field survives");
    assert!(run.stages.is_empty());
    assert_eq!(run.test_slots, 0);
    let task = &run.tasks[0];
    assert_eq!(task.message_count, 3, "an M9 field survives");
    assert_eq!(task.stage, 1);
    assert_eq!(task.origin, TaskOrigin::Plan);
    assert_eq!(task.fixes, None);
    assert_eq!(task.tier, None);
    assert!(task.weakening.is_empty());
    // Ruling C-28 (5): the plan review's stage facts default off.
    assert_eq!((task.atomic, task.interface_change), (false, false));
    assert_eq!(task.atomic_reason, None);
    // An M9 `CheckInfo`, with no `tier` key.
    let m9_check = r#"{"at":1700000050,"ok":false,"code":101,"timed_out":false,"secs":30,
        "summary":"test a ... FAILED","on_candidate":true,"decider_summary":null,
        "summary_source":null}"#;
    let check: CheckInfo = serde_json::from_str(m9_check).expect("an M9 CheckInfo decodes");
    assert_eq!((check.code, check.tier), (Some(101), None));

    // The same run with every new field set survives the wire, by name.
    let mut run = run;
    run.stages = vec![a_stage(Some("aaaa1111")), a_stage(None)];
    run.test_slots = 8;
    let task = &mut run.tasks[0];
    task.stage = 2;
    task.origin = TaskOrigin::Bisect;
    task.fixes = Some("bisect of t4".into());
    task.tier = Some(a_tier_info());
    task.weakening = vec![a_signal()];
    task.atomic = true;
    task.atomic_reason = Some("a protocol bump updates every client".into());
    task.interface_change = true;
    task.last_check = Some(CheckInfo {
        tier: Some(TierInfo {
            tier: 2,
            ..a_tier_info()
        }),
        ..check
    });
    let snapshot = RunsSnapshot {
        revision: 9,
        runs: vec![run],
        now: 1_700_001_000,
        proposals: Vec::new(),
    };
    let msg = DaemonMsg::Run(RunReply::Snapshot(snapshot));
    both_ways(&msg);
    let DaemonMsg::Run(RunReply::Snapshot(back)) =
        rmp_serde::from_slice(&rmp_serde::to_vec_named(&msg).unwrap()).unwrap()
    else {
        panic!("must decode back to RunReply::Snapshot");
    };
    let (run, task) = (&back.runs[0], &back.runs[0].tasks[0]);
    assert_eq!(run.test_slots, 8);
    assert_eq!(run.stages[0].head.as_deref(), Some("aaaa1111"));
    assert_eq!(run.stages[1].head, None);
    assert_eq!((task.stage, task.origin), (2, TaskOrigin::Bisect));
    assert_eq!(task.fixes.as_deref(), Some("bisect of t4"));
    assert_eq!(task.tier.as_ref().unwrap().tier, 1);
    assert_eq!(
        task.last_check
            .as_ref()
            .unwrap()
            .tier
            .as_ref()
            .unwrap()
            .tier,
        2
    );
    assert_eq!(task.weakening[0].id, "W1");
    assert!(task.atomic && task.interface_change);
    assert_eq!(
        task.atomic_reason.as_deref(),
        Some("a protocol bump updates every client")
    );
}

fn a_tier_record() -> TierRunRecord {
    TierRunRecord {
        v: HISTORY_VERSION,
        record_id: "run-a1b2/tier/op-17".into(),
        at: 1_700_000_700,
        run_id: "run-a1b2".into(),
        task_id: Some("t1".into()),
        stage: 2,
        tier: 1,
        secs: 130,
        affected: 3,
        full_reason: None,
        cache_hit: false,
        cached_steps: 1,
        steps: 3,
        ok: true,
        flaky: vec!["t_x".into()],
    }
}

fn a_flaky_record() -> FlakyRecord {
    FlakyRecord {
        v: HISTORY_VERSION,
        record_id: "run-a1b2/flaky/op-17/t_x".into(),
        at: 1_700_000_710,
        run_id: "run-a1b2".into(),
        task_id: None,
        tier: 3,
        test: "t_x".into(),
    }
}

fn a_bisect_line() -> BisectLine {
    BisectLine {
        v: HISTORY_VERSION,
        record_id: "run-a1b2/bisect/2/1".into(),
        at: 1_700_000_800,
        run_id: "run-a1b2".into(),
        stage: 2,
        head: "cccc3333".into(),
        tests: vec!["daemon::run::merges".into()],
        range: 7,
        probes: 3,
        culprit: Some("t4".into()),
        reason: None,
        fix_task: Some("fix1".into()),
    }
}

#[test]
fn history_v3_lines_round_trip() {
    assert_eq!(HISTORY_VERSION, 4);
    for (line, tag) in [
        (HistoryLine::Tier(a_tier_record()), "tier"),
        (HistoryLine::Flaky(a_flaky_record()), "flaky"),
        (HistoryLine::Bisect(a_bisect_line()), "bisect"),
    ] {
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.starts_with(&format!(r#"{{"type":"{tag}""#)), "{json}");
        both_ways(&line);
    }
    both_ways(&HistoryLine::Bisect(BisectLine {
        culprit: None,
        reason: Some("no single culprit: two merges".into()),
        fix_task: None,
        ..a_bisect_line()
    }));

    // A task line gains its stage and origin.
    let record = TaskRecord {
        stage: 2,
        origin: TaskOrigin::Sync,
        ..a_task_record()
    };
    both_ways(&HistoryLine::Task(record));

    // `run stats` gains the flaky proposals and the window they were read with.
    let stats = HistoryStats {
        flaky_proposals: vec![FlakyProposal {
            test: "t_x".into(),
            runs: 3,
            last_at: 1_700_000_900,
        }],
        window_days: 14,
        quarantine_after: 3,
        ..a_stats()
    };
    both_ways(&stats);
    let mut json = serde_json::to_value(&stats).unwrap();
    let map = json.as_object_mut().unwrap();
    for key in ["flaky_proposals", "window_days", "quarantine_after"] {
        assert!(map.remove(key).is_some(), "{key} is a field");
    }
    assert_eq!(
        serde_json::from_value::<HistoryStats>(json).unwrap(),
        a_stats()
    );

    // A version-2 file, as milestone 9 wrote it, still decodes line by line.
    let lines: Vec<HistoryLine> = include_str!("m9_history_v2.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).expect("a version-2 line decodes"))
        .collect();
    let [HistoryLine::Task(task), HistoryLine::RoleRoute(role)] = &lines[..] else {
        panic!("a task line and a role_route line: {lines:?}");
    };
    assert_eq!((task.v, role.v), (2, 2));
    assert_eq!(task.task_id, "t1");
    assert_eq!(task.stage, 0, "a line written before stages has none");
    assert_eq!(task.origin, TaskOrigin::Plan);
    assert_eq!(task.worker_usage, a_token_usage());
    assert_eq!(role.chosen.model, "claude-opus-5");
}
