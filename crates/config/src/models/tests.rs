use super::*;
use proto::Runtime;

fn choice(model: &str, effort: Option<&str>, fallback: Option<&str>) -> RoleChoice {
    RoleChoice {
        model: ModelRef::parse(model).unwrap(),
        effort: effort.map(str::to_string),
        fallback: fallback.map(|f| ModelRef::parse(f).unwrap()),
    }
}

fn table(rows: &[(Role, RoleChoice)]) -> ModelTable {
    ModelTable {
        rows: rows.iter().cloned().collect(),
        brainstorm: None,
    }
}

#[test]
fn model_refs_parse_and_print() {
    assert_eq!(
        ModelRef::parse("claude:claude-opus-5-5").unwrap(),
        ModelRef {
            runtime: Runtime::Claude,
            id: Some("claude-opus-5-5".into())
        }
    );
    assert_eq!(
        ModelRef::parse("codex:default").unwrap(),
        ModelRef::default_of(Runtime::Codex)
    );
    assert_eq!(
        ModelRef::parse("codex:gpt-6-sol").unwrap().label(),
        "codex:gpt-6-sol"
    );
    assert_eq!(
        ModelRef::default_of(Runtime::Claude).label(),
        "claude:default"
    );
    let long = format!("codex:{}", "x".repeat(101));
    for bad in [
        "",
        "opus",
        "shell:x",
        "claude:",
        "claude:a b",
        "claude:a\u{202e}b",
        "claude:a\nb",
        long.as_str(),
    ] {
        assert!(ModelRef::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_models_table_reads_every_row_shape() {
    let (config, problems) = crate::parse(
        r#"
[models.orchestrator]
model = "claude:claude-opus-5-5"
effort = "high"

[models.implementer.medium]
model = "codex:gpt-6-sol"
effort = "medium"
fallback = "claude:claude-opus-5-5"

[models.helpers]
model = "claude:claude-haiku-4-5"

[models.helpers.run_name]
model = "claude:claude-sonnet-5"

[models.brainstorm]
first = "claude:claude-opus-5-5"
second = "codex:default"
effort = "high"
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let t = &config.orchestrator.roles;
    assert_eq!(
        t.rows[&Role::Orchestrator],
        choice("claude:claude-opus-5-5", Some("high"), None)
    );
    assert_eq!(
        t.rows[&Role::ImplementerMedium],
        choice(
            "codex:gpt-6-sol",
            Some("medium"),
            Some("claude:claude-opus-5-5")
        )
    );
    assert_eq!(
        t.rows[&Role::Helpers],
        choice("claude:claude-haiku-4-5", None, None)
    );
    assert_eq!(
        t.rows[&Role::Helper(HelperKind::RunName)],
        choice("claude:claude-sonnet-5", None, None)
    );
    assert_eq!(
        t.brainstorm,
        Some(BrainstormChoice {
            first: ModelRef::parse("claude:claude-opus-5-5").unwrap(),
            second: ModelRef::default_of(Runtime::Codex),
            effort: Some("high".into()),
        })
    );
    assert_eq!(t.rows.len(), 4);
}

#[test]
fn a_bad_row_is_a_problem_and_the_rest_is_kept() {
    let (config, problems) = crate::parse(
        r#"
[models.reviewer]
model = "opus"

[models.planner]
effort = "high"

[models.research]
model = "claude:claude-haiku-4-5"
effort = "Very High"

[models.test_writer]
model = "codex:default"

[models.nope]
model = "codex:default"

[models.helpers.nope]
model = "codex:default"
"#,
    );
    let keys: Vec<&str> = problems.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "models.helpers.nope",
            "models.nope",
            "models.planner.model",
            "models.research.effort",
            "models.reviewer.model"
        ],
        "{problems:?}"
    );
    assert_eq!(
        config.orchestrator.roles.rows.keys().collect::<Vec<_>>(),
        [&Role::TestWriter]
    );
}

#[test]
fn resolution_takes_the_repository_row_then_the_global_then_the_builtin() {
    let global = table(&[
        (
            Role::Reviewer,
            choice("claude:claude-opus-5-5", Some("high"), None),
        ),
        (Role::Planner, choice("codex:gpt-6-sol", None, None)),
    ]);
    let repo = table(&[(Role::Reviewer, choice("codex:gpt-6.1-sol", None, None))]);
    // A row is overridden whole: the repository's row has no effort, and none is inherited.
    assert_eq!(
        resolve(Role::Reviewer, Some(&repo), &global),
        choice("codex:gpt-6.1-sol", None, None)
    );
    assert_eq!(
        resolve(Role::Planner, Some(&repo), &global),
        choice("codex:gpt-6-sol", None, None)
    );
    assert_eq!(
        resolve(Role::Research, Some(&repo), &global),
        builtin_choice(Role::Research)
    );
}

#[test]
fn a_helper_kind_takes_kind_then_helpers_repository_first() {
    let triage = Role::Helper(HelperKind::Triage);
    let run_name = Role::Helper(HelperKind::RunName);
    let global = table(&[
        (Role::Helpers, choice("claude:claude-sonnet-5", None, None)),
        (triage, choice("codex:default", None, None)),
    ]);
    let repo_helpers = table(&[(Role::Helpers, choice("codex:gpt-6-luna", None, None))]);
    let repo_triage = table(&[(triage, choice("claude:claude-opus-5-5", None, None))]);
    assert_eq!(
        resolve(triage, Some(&repo_triage), &global).model.label(),
        "claude:claude-opus-5-5"
    );
    // The global kind beats the repository's helpers (MR §3.4's order).
    assert_eq!(
        resolve(triage, Some(&repo_helpers), &global).model.label(),
        "codex:default"
    );
    assert_eq!(
        resolve(run_name, Some(&repo_helpers), &global)
            .model
            .label(),
        "codex:gpt-6-luna"
    );
    assert_eq!(
        resolve(run_name, None, &global).model.label(),
        "claude:claude-sonnet-5"
    );
    assert_eq!(
        resolve(run_name, None, &ModelTable::default()),
        builtin_choice(Role::Helpers)
    );
}

#[test]
fn the_builtin_table_is_the_specs_with_research_corrected() {
    let rows: Vec<(Role, String, Option<&str>, Option<&str>)> = vec![
        (
            Role::Orchestrator,
            "claude:claude-opus-5-5".into(),
            Some("high"),
            None,
        ),
        (
            Role::Planner,
            "claude:claude-opus-5-5".into(),
            Some("high"),
            None,
        ),
        (
            Role::ImplementerSmall,
            "claude:claude-sonnet-5".into(),
            Some("low"),
            None,
        ),
        (
            Role::ImplementerMedium,
            "claude:claude-sonnet-5".into(),
            Some("medium"),
            None,
        ),
        (
            Role::ImplementerHub,
            "claude:claude-opus-5-5".into(),
            Some("high"),
            None,
        ),
        (
            Role::TestWriter,
            "codex:default".into(),
            Some("medium"),
            None,
        ),
        (
            Role::Reviewer,
            "codex:default".into(),
            Some("high"),
            Some("claude:claude-opus-5-5"),
        ),
        (
            Role::Research,
            "claude:claude-haiku-4-5".into(),
            Some("low"),
            None,
        ),
        (Role::Helpers, "claude:claude-haiku-4-5".into(), None, None),
    ];
    for (role, model, effort, fallback) in rows {
        assert_eq!(
            builtin_choice(role),
            choice(&model, effort, fallback),
            "{role:?}"
        );
    }
    let b = builtin_brainstorm();
    assert_eq!(
        (b.first.label(), b.second.label(), b.effort.as_deref()),
        (
            "claude:claude-opus-5-5".into(),
            "codex:default".into(),
            Some("high")
        )
    );
}

#[test]
fn a_table_renders_and_reads_back() {
    let mut t = table(&[
        (
            Role::ImplementerHub,
            choice(
                "claude:claude-opus-5-5",
                Some("max"),
                Some("codex:gpt-6.1-sol"),
            ),
        ),
        (
            Role::Helper(HelperKind::CiSummary),
            choice("codex:default", None, None),
        ),
    ]);
    t.brainstorm = Some(builtin_brainstorm());
    let text = render(&t);
    assert!(text.contains("[models.implementer.hub]\nmodel = \"claude:claude-opus-5-5\"\neffort = \"max\"\nfallback = \"codex:gpt-6.1-sol\"\n"), "{text}");
    assert!(
        text.contains("[models.helpers.ci_summary]\nmodel = \"codex:default\"\n"),
        "{text}"
    );
    let (back, problems) = parse_text(&text).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(back, t);
}

#[test]
fn a_repository_file_loads_and_a_broken_one_is_ignored_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(REPO_FILE);
    assert_eq!(load_repo(&path), (None, vec![]));
    std::fs::write(&path, "[models.reviewer]\nmodel = \"codex:gpt-6.1-sol\"\n").unwrap();
    let (table, warnings) = load_repo(&path);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        table.unwrap().rows[&Role::Reviewer].model.label(),
        "codex:gpt-6.1-sol"
    );
    std::fs::write(&path, "[models.reviewer\n").unwrap();
    let (table, warnings) = load_repo(&path);
    assert_eq!(table, None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with(&format!("{}: not valid TOML", path.display())),
        "{warnings:?}"
    );
}

#[test]
fn saving_an_empty_repository_table_removes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(REPO_FILE);
    let t = table(&[(Role::Planner, choice("codex:default", None, None))]);
    save_repo(&path, &t).unwrap();
    assert_eq!(load_repo(&path).0, Some(t));
    save_repo(&path, &ModelTable::default()).unwrap();
    assert!(!path.exists());
}

#[test]
fn an_empty_config_has_an_empty_role_table() {
    let (config, problems) = crate::parse("");
    assert!(problems.is_empty(), "{problems:?}");
    assert!(config.orchestrator.roles.is_empty());
    assert!(config.orchestrator.roles_notes.is_empty());
}

/// Fix round 1 (M9): a repository save keeps the file's mode and a symlinked
/// `models.toml` stays a link, as `config::settings::save` does.
#[cfg(unix)]
#[test]
fn a_repository_save_keeps_the_mode_and_the_link() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.toml");
    std::fs::write(&real, "[models.planner]\nmodel = \"codex:default\"\n").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    let path = dir.path().join(REPO_FILE);
    std::os::unix::fs::symlink(&real, &path).unwrap();
    let t = table(&[(Role::Reviewer, choice("codex:default", None, None))]);
    save_repo(&path, &t).unwrap();
    assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    assert_eq!(load_repo(&real).0, Some(t));
    let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
