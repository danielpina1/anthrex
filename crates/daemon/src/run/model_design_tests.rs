//! Task M9.6.3: the design flow's per-run fields persist through `run.json`
//! (`journal::save_run`, the driver's serializer, and `journal::load_all`): the frozen
//! mode (`Run.design_mode`), the frozen design limits (`OrchLimits.design`) and the
//! frozen `brainstorm` model list (carry M-4). A protocol-16 (9.5) `run.json` loads with
//! the mode `Off` and writes none of them back.

use proto::{DesignMode, Effort, Runtime, Strength};

use super::tuning::tests::{count, keys, save_and_load, tmp};
use super::{FrozenList, ListCandidate, ListPolicy, RouteListsFrozen, Run};
use crate::run::orch::DesignLimits;

/// A planned goal's `run.json` as milestone 9.5's code wrote it at the start (state
/// `planning`, an orchestrator record, a frozen `review` list), captured in task
/// M9.6.3 (commit 00ec0f18) before any of its changes.
const M95_RUN: &str = include_str!("../../tests/fixtures/run/m95-run.json");

/// Every key task M9.6.3 adds to the persisted run.
const NEW_KEYS: [&str; 3] = ["design_mode", "design", "brainstorm"];

fn old_run() -> Run {
    serde_json::from_str(M95_RUN).expect("m95-run.json parses")
}

fn candidate(model: &str, strength: Strength, effort: Option<Effort>) -> ListCandidate {
    ListCandidate {
        runtime: Runtime::Claude,
        model: model.into(),
        strength,
        effort,
    }
}

#[test]
fn an_old_run_json_loads_with_design_off() {
    let mut run = old_run();
    assert_eq!(run.state, proto::RunState::Planning);
    assert!(run.orch.orchestrator.is_some(), "a planned goal's run");
    assert_eq!(run.design_mode, DesignMode::Off);
    assert_eq!(run.limits.orch.design, DesignLimits::default());
    assert!(run.limits.route_lists.brainstorm.is_empty());
    assert!(!run.limits.route_lists.review.is_empty());

    // Written again, it is the JSON 9.5 wrote, so it has none of the new keys.
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded, run);
    let mut again: serde_json::Value = serde_json::from_str(&text).expect("run.json is JSON");
    let captured: serde_json::Value = serde_json::from_str(M95_RUN).expect("fixture");
    let written = keys(&again);
    for key in NEW_KEYS {
        assert_eq!(count(&written, key), 0, "run.json gained {key}");
    }
    // `save_and_load` moved the run's data directory under the temp dir.
    again["data_dir"] = captured["data_dir"].clone();
    assert_eq!(again, captured);
}

#[test]
fn a_design_run_round_trips() {
    let mut run = old_run();
    run.design_mode = DesignMode::Full;
    run.limits.orch.design = DesignLimits {
        docs_dir: String::new(),
        commit_brainstorm: true,
        max_questions: 2,
        phase_minutes: 90,
        brainstormer: proto::Budget {
            tool_calls: 60,
            minutes: 20,
            tokens: Some(1000),
        },
        doc_reviewer: proto::Budget {
            tool_calls: 10,
            minutes: 5,
            tokens: None,
        },
    };
    run.limits.route_lists.brainstorm = FrozenList {
        candidates: vec![
            candidate("claude-opus-5-5", Strength::Frontier, Some(Effort::High)),
            candidate("claude-sonnet-5", Strength::Standard, None),
        ],
        pick: ListPolicy::First,
    };

    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded, run);
    let written = keys(&serde_json::from_str(&text).expect("run.json is JSON"));
    for key in NEW_KEYS {
        assert!(count(&written, key) > 0, "run.json lacks {key}");
    }
    assert!(text.contains("\"design_mode\":\"full\""), "{text}");
}

/// Carry M-4: the `brainstorm` list is the ninth frozen list, frozen from config with
/// each candidate's roster strength, named last, and kept by a save and load.
#[test]
fn a_frozen_brainstorm_list_survives_save_and_load() {
    let (config, problems) = config::parse(
        r#"
[orchestrator.routes.brainstorm]
candidates = [
  { runtime = "claude", model = "claude-opus-5-5", effort = "high" },
  { runtime = "claude", model = "claude-sonnet-5" },
]
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let o = &config.orchestrator;
    let frozen = RouteListsFrozen::freeze(&o.tuning.routes, &o.models);
    assert_eq!(
        frozen.brainstorm,
        FrozenList {
            candidates: vec![
                candidate("claude-opus-5-5", Strength::Frontier, Some(Effort::High)),
                candidate("claude-sonnet-5", Strength::Standard, None),
            ],
            pick: ListPolicy::First,
        }
    );
    let named = frozen.named();
    assert_eq!(named.len(), 9);
    assert_eq!(named[8], ("brainstorm", &frozen.brainstorm));
    assert!(!frozen.is_empty(), "a brainstorm list alone is a list");

    let mut run = old_run();
    run.limits.route_lists = frozen.clone();
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded.limits.route_lists, frozen);
    assert!(text.contains("\"brainstorm\""), "{text}");
}
