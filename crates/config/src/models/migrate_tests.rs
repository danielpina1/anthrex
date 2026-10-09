// Mounted from `models/mod.rs` as `#[cfg(test)] mod migrate_tests;`.
use super::*;

fn parsed(text: &str) -> crate::Orchestrator {
    crate::parse(text).0.orchestrator
}

fn row(o: &crate::Orchestrator, role: Role) -> (String, Option<String>, Option<String>) {
    let c = &o.roles.rows[&role];
    (
        c.model.label(),
        c.effort.clone(),
        c.fallback.as_ref().map(ModelRef::label),
    )
}

const ROSTER: &str = "[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6.1-sol\"\nstrength = \"frontier\"\n";

#[test]
fn a_default_config_migrates_to_nothing() {
    let o = parsed("");
    assert!(o.roles.is_empty(), "{:?}", o.roles);
    assert!(o.roles_notes.is_empty(), "{:?}", o.roles_notes);
}

#[test]
fn a_route_list_gives_the_rows_model_and_fallback() {
    let o = parsed(&format!(
        "{ROSTER}\n[orchestrator.routes.review]\ncandidates = [{{ runtime = \"codex\", model = \"gpt-6.1-sol\", effort = \"high\" }}, {{ runtime = \"claude\", model = \"claude-opus-5-5\" }}]\n"
    ));
    assert_eq!(
        row(&o, Role::Reviewer),
        (
            "codex:gpt-6.1-sol".into(),
            Some("high".into()),
            Some("claude:claude-opus-5-5".into())
        )
    );
    assert!(o.roles_notes.contains(&"config: [orchestrator.routes.review] is replaced by [models.reviewer]; it is migrated until you save in C-b S".to_string()), "{:?}", o.roles_notes);
    assert!(o.roles_notes.contains(&"config: [[orchestrator.models]] is replaced by the models your CLIs report (C-b S); it is migrated until you save in C-b S".to_string()), "{:?}", o.roles_notes);
}

#[test]
fn the_agent_table_migrates_to_the_orchestrator_row() {
    let o = parsed(
        "[orchestrator.agent]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"\neffort = \"medium\"\n",
    );
    assert_eq!(
        row(&o, Role::Orchestrator),
        ("claude:claude-sonnet-5".into(), Some("medium".into()), None)
    );
    assert_eq!(o.roles_notes.len(), 3, "{:?}", o.roles_notes);
    assert!(o.roles_notes.contains(&"config: orchestrator.agent.model is replaced by [models.orchestrator]; it is migrated until you save in C-b S".to_string()));
}

#[test]
fn planners_scouts_and_deciders_migrate_by_strength() {
    let o = parsed(
        "[orchestrator.planners]\nruntime = \"codex\"\nstrength = \"standard\"\neffort = \"medium\"\n\n[orchestrator.scouts]\nruntime = \"claude\"\nstrength = \"standard\"\neffort = \"high\"\n\n[orchestrator.deciders]\nmode = \"codex\"\nstrength = \"standard\"\neffort = \"medium\"\n",
    );
    assert_eq!(
        row(&o, Role::Planner),
        ("codex:default".into(), Some("medium".into()), None)
    );
    assert_eq!(
        row(&o, Role::Research),
        ("claude:claude-sonnet-5".into(), Some("high".into()), None)
    );
    assert_eq!(
        row(&o, Role::Helpers),
        ("codex:default".into(), Some("medium".into()), None)
    );
    // `mode` stays a live key (decision 17): no note for it.
    assert!(
        o.roles_notes.iter().all(|n| !n.contains("deciders.mode")),
        "{:?}",
        o.roles_notes
    );
}

#[test]
fn default_runtime_moves_the_implementer_rows_it_can_and_derives_writer_and_reviewer() {
    let o = parsed("[orchestrator]\ndefault_runtime = \"codex\"\n");
    assert_eq!(row(&o, Role::ImplementerSmall).0, "codex:default");
    assert_eq!(row(&o, Role::ImplementerMedium).0, "codex:default");
    assert!(!o.roles.rows.contains_key(&Role::ImplementerHub));
    assert!(o.roles_notes.contains(&"config: orchestrator.default_runtime = \"codex\" has no codex model at frontier strength in the roster; implementer.hub keeps claude:claude-opus-5-5".to_string()), "{:?}", o.roles_notes);
    assert_eq!(row(&o, Role::TestWriter).0, "claude:claude-sonnet-5");
    assert_eq!(row(&o, Role::Reviewer).0, "claude:claude-sonnet-5");
}

#[test]
fn an_explicit_models_row_beats_a_migrated_one() {
    let o = parsed(&format!(
        "{ROSTER}\n[orchestrator.routes.planner]\ncandidates = [{{ runtime = \"codex\", model = \"gpt-6.1-sol\" }}]\n\n[models.planner]\nmodel = \"claude:claude-opus-5-5\"\n"
    ));
    assert_eq!(
        row(&o, Role::Planner),
        ("claude:claude-opus-5-5".into(), None, None)
    );
}
