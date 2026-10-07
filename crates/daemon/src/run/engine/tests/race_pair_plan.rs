//! Milestone 9.5 task M9.5.14: `race` and `pair` set by the orchestrator mid-run go
//! through M9's gate holds and the project-trust check (ruling RR-6), and at dispatch a
//! racing task races (task M9.5.17a) and a paired one starts with its test writer (task
//! M9.5.16, `pair.rs`).

use proto::models::{ModelRef, Role, RoleChoice};
use proto::{AgentRole, ModelEntry, PlanEdit, Runtime, Strength};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::gate_holds::{held, in_epic};
use super::orch::{add, answer, edit_plan};
use crate::headless::McpTarget;
use crate::run::engine::{EventKind, OpKind};
use crate::run::validate::EditScope;

/// RR-6: a racing task the orchestrator adds to an epic still being decided waits under
/// the epic's hold, a paired one too, and a pair amend on a held task keeps its hold;
/// one added outside any open hold is not held, like any other addition.
#[test]
fn an_orchestrators_race_goes_through_a_gate_hold() {
    let mut fx = held(false);
    let mut racing = in_epic("t3", "mail3", "mail");
    racing["task"]["race"] = json!(true);
    let mut paired = in_epic("t4", "mail4", "mail");
    paired["task"]["pair"] = json!(true);
    paired["task"]["test_to_write"] = json!("mail4::works");
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [racing, paired]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "epic:mail");
    for id in ["t3", "t4"] {
        assert_eq!(
            fx.task(id).orch.gate_hold.as_deref(),
            Some("epic:mail"),
            "{id}"
        );
    }
    assert!(fx.task("t3").spec.race && fx.task("t4").spec.pair);
    let hold = &fx.run().orch.gate_holds[0];
    assert_eq!(hold.tasks, ["t2", "t3", "t4"]);

    let amend = json!({"op": "amend_task", "task_id": "t2", "pair": true});
    fx.task_mut("t2").spec.test_to_write = Some("mail::works".into());
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [amend]})));
    assert!(ok, "{value}");
    assert!(fx.task("t2").spec.pair);
    assert_eq!(fx.task("t2").orch.gate_hold.as_deref(), Some("epic:mail"));

    // The daemon's own bounds take both, and serde refuses a race that is not a bool.
    let mut forged = add("t6", "fax");
    forged["task"]["race"] = json!("yes");
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [forged]})));
    assert!(
        !ok && value.to_string().contains("expected a boolean"),
        "{value}"
    );

    let mut outside = add("t5", "sms");
    outside["task"]["race"] = json!(true);
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [outside]})));
    assert!(ok, "{value}");
    assert_eq!(fx.task("t5").orch.gate_hold, None);
}

fn entry(runtime: Runtime, model: &str, strength: Strength) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.to_string(),
        strength,
        note: String::new(),
    }
}

/// RR-6: a racing task that would reach a runtime the driver found a decision-53
/// refusal for is refused with that text, as any widening edit is.
#[test]
fn a_racing_task_that_widens_the_reach_is_refused_by_the_trust_check() {
    let config = config::Orchestrator {
        models: vec![
            entry(Runtime::Claude, "claude-sonnet-5", Strength::Standard),
            entry(Runtime::Claude, "claude-opus-5", Strength::Frontier),
            entry(Runtime::Codex, "gpt-5-codex", Strength::Standard),
        ],
        review_small: false,
        // Milestone 9.8: a Claude reviewer row, and the small row's racer on Codex.
        roles: proto::models::ModelTable {
            rows: [
                (Role::Reviewer, choice("claude:claude-opus-5", None)),
                (
                    Role::ImplementerSmall,
                    choice("claude:claude-sonnet-5", Some("codex:gpt-5-codex")),
                ),
                // Decision 31: `t1`'s model from its row, not a plan route.
                (
                    Role::ImplementerMedium,
                    choice("claude:claude-opus-5", None),
                ),
            ]
            .into(),
            brainstorm: None,
        },
        ..config::Orchestrator::default()
    };
    // Milestone 9.8 decision 29: an M task, whose row (Opus) has no fallback, so its
    // escalation stays on Claude; the S row's Codex fallback is the racer's.
    let plan = plan_with(PROFILE, &[task_toml("t1", "M", "[\"docs/a.md\"]", "")]);
    let mut fx = Fixture::with_config(&plan, config);
    fx.ready(false);
    assert_eq!(
        crate::run::reach::reachable_runtimes(fx.run()),
        [Runtime::Claude]
    );
    let refusal = "Codex's project settings".to_string();
    // On the S row's Sonnet, racing on its Codex fallback.
    let text = plan_with(PROFILE, &[task("t9", "S", "t9", "race = true")]);
    let racing = crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Edit {
        reply,
        run_id: RUN_ID.into(),
        edits: vec![PlanEdit::AddTask { task: racing }],
        scope: EditScope::Run,
        refusals: vec![(Runtime::Codex, refusal.clone())],
        submit: false,
    });
    assert_eq!(replies(&effects), vec![Err(refusal)]);
    assert_eq!(fx.run().tasks.len(), 1, "a refused edit changes nothing");
}

/// At dispatch (tasks M9.5.16 and M9.5.17a; the final fix wave removed the staging
/// gate, review D's M-5): a racing task starts two racers in its two lane checkouts, and
/// a paired task starts its test writer.
#[test]
fn race_tasks_race_and_paired_tasks_start_their_test_writer() {
    let plan = plan_with(
        &profile_with("max_writers = 3"),
        &[
            task("t1", "S", "a", "race = true"),
            task("t2", "S", "b", "pair = true\ntest_to_write = \"b::works\""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    fx.complete_prepares();
    let windows = fx.ops("CreateWindow");
    let mut seen: Vec<(String, AgentRole)> = windows
        .iter()
        .map(|(_, kind)| match kind {
            OpKind::CreateWindow { name, spec, .. } => match &spec.mcp {
                Some(McpTarget { role, .. }) => (name.clone(), *role),
                None => panic!("{name} has no MCP target"),
            },
            other => panic!("{other:?}"),
        })
        .collect();
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        seen,
        [
            (format!("{H4}/t1.aw1"), AgentRole::Racer),
            (format!("{H4}/t1.bw2"), AgentRole::Racer),
            (format!("{H4}/t2.t1"), AgentRole::TestWriter),
        ]
    );
    assert!(fx.task("t1").race.is_some());
    assert!(fx.task("t2").pair.is_some());
}

/// A row of `model`, falling back to `fallback`.
fn choice(model: &str, fallback: Option<&str>) -> RoleChoice {
    RoleChoice {
        model: ModelRef::parse(model).unwrap(),
        effort: None,
        fallback: fallback.map(|f| ModelRef::parse(f).unwrap()),
    }
}
