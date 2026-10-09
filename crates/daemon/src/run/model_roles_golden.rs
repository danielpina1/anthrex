//! Milestone 9.8 task 1 (MR §8): what each role resolves to with today's code, for a
//! matrix of old configs, pinned in `model_roles_golden.json` before anything changes.
//! Task M9.8.3 adds the comparison with `config::models::resolve`. Task M9.8.13 deleted
//! `today` (since M9.8.7b every launcher it called took its row) and its test; the JSON
//! stays the record.

use std::collections::BTreeMap;

/// A cell whose plan does not validate today (decision 16).
pub(crate) const UNRESOLVED: &str = "unresolved";

const GOLDEN: &str = include_str!("model_roles_golden.json");

const CODEX_ROWS: &str = r#"
[[orchestrator.models]]
runtime = "codex"
model = "gpt-6-luna"
strength = "fast"

[[orchestrator.models]]
runtime = "codex"
model = "gpt-6-sol"
strength = "standard"

[[orchestrator.models]]
runtime = "codex"
model = "gpt-6.1-sol"
strength = "frontier"
"#;

const ROUTES: &str = r#"
[[orchestrator.models]]
runtime = "codex"
model = "gpt-6-sol"
strength = "standard"

[[orchestrator.models]]
runtime = "codex"
model = "gpt-6.1-sol"
strength = "frontier"

[orchestrator.routes.s]
candidates = [{ runtime = "codex", model = "gpt-6-sol", effort = "low" }, { runtime = "claude", model = "claude-sonnet-5" }]

[orchestrator.routes.m]
candidates = [{ runtime = "claude", model = "claude-sonnet-5", effort = "medium" }, { runtime = "codex", model = "gpt-6-sol" }]

[orchestrator.routes.hub]
candidates = [{ runtime = "claude", model = "claude-opus-5-5", effort = "high" }, { runtime = "codex", model = "gpt-6.1-sol" }]

[orchestrator.routes.review]
candidates = [{ runtime = "codex", model = "gpt-6.1-sol", effort = "high" }, { runtime = "claude", model = "claude-opus-5-5" }]

[orchestrator.routes.scout]
candidates = [{ runtime = "claude", model = "claude-sonnet-5", effort = "medium" }]

[orchestrator.routes.decider]
candidates = [{ runtime = "claude", model = "claude-sonnet-5" }]

[orchestrator.routes.planner]
candidates = [{ runtime = "codex", model = "gpt-6.1-sol", effort = "high" }]

[orchestrator.routes.orchestrator]
candidates = [{ runtime = "codex", model = "gpt-6.1-sol", effort = "high" }, { runtime = "claude", model = "claude-opus-5-5" }]

[orchestrator.routes.brainstorm]
candidates = [{ runtime = "codex", model = "gpt-6-sol" }, { runtime = "claude", model = "claude-sonnet-5" }]
"#;

/// The matrix (MR §8): each name and its `config.toml`. Built at test time so the
/// roster rows are written once.
pub(crate) fn matrix() -> Vec<(&'static str, String)> {
    vec![
        ("default", String::new()),
        ("default_runtime_codex", "[orchestrator]\ndefault_runtime = \"codex\"\n".into()),
        ("routes", ROUTES.into()),
        ("agent", "[orchestrator.agent]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"\neffort = \"medium\"\n".into()),
        ("planners", "[orchestrator.planners]\nruntime = \"codex\"\nstrength = \"standard\"\neffort = \"medium\"\n".into()),
        ("scouts", "[orchestrator.scouts]\nruntime = \"claude\"\nstrength = \"standard\"\neffort = \"high\"\n".into()),
        ("deciders", "[orchestrator.deciders]\nmode = \"codex\"\nstrength = \"standard\"\neffort = \"medium\"\n".into()),
        ("custom_roster", format!("[orchestrator]\ndefault_runtime = \"codex\"\n{CODEX_ROWS}")),
        ("no_builtin_models", format!("[orchestrator]\ndefault_runtime = \"codex\"\nbuiltin_models = false\n{CODEX_ROWS}")),
    ]
}

pub(crate) fn golden() -> BTreeMap<String, BTreeMap<String, String>> {
    serde_json::from_str(GOLDEN).expect("model_roles_golden.json parses")
}

/// What the role table resolves each golden row to (task M9.8.3).
fn resolved(cfg: &config::Orchestrator) -> BTreeMap<String, String> {
    use config::models::{Role, resolve, resolve_brainstorm};
    let t = &cfg.roles;
    let mut out: BTreeMap<String, String> = [
        ("orchestrator", Role::Orchestrator),
        ("planner", Role::Planner),
        ("implementer.small", Role::ImplementerSmall),
        ("implementer.medium", Role::ImplementerMedium),
        ("implementer.hub", Role::ImplementerHub),
        ("test_writer", Role::TestWriter),
        ("reviewer", Role::Reviewer),
        ("research", Role::Research),
        ("helpers", Role::Helpers),
    ]
    .into_iter()
    .map(|(key, role)| (key.to_string(), resolve(role, None, t).model.label()))
    .collect();
    let b = resolve_brainstorm(None, t);
    out.insert("brainstorm.first".into(), b.first.label());
    out.insert("brainstorm.second".into(), b.second.label());
    out
}

#[test]
fn the_role_table_resolves_every_old_config_as_today() {
    let builtin = resolved(&config::Orchestrator::default());
    for (name, text) in &matrix() {
        let (config, _) = config::parse(text);
        let new = resolved(&config.orchestrator);
        for (row, model) in &golden()[*name] {
            let want = if model == UNRESOLVED {
                &builtin[row]
            } else {
                model
            };
            assert_eq!(&new[row], want, "{name}: {row}");
        }
    }
}
