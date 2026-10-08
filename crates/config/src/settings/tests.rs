use super::*;
use proto::settings::key;
use proto::{Runtime, Strength};

fn default_doc() -> SettingsDoc {
    doc_of(&crate::Orchestrator::default())
}

#[test]
fn every_builtin_model_is_shipped() {
    for m in crate::default_roster() {
        let shipped = SHIPPED_CLAUDE
            .iter()
            .chain(SHIPPED_CODEX.iter())
            .find(|s| s.runtime == m.runtime && s.model == m.model);
        let shipped = shipped.unwrap_or_else(|| panic!("{m:?} is not shipped"));
        assert_eq!(shipped.strength, m.strength, "{m:?}");
    }
}

#[test]
fn shipped_lists_are_the_spec_s() {
    let claude: Vec<_> = SHIPPED_CLAUDE
        .iter()
        .map(|s| (s.runtime, s.model, s.strength, s.label))
        .collect();
    assert_eq!(
        claude,
        [
            (
                Runtime::Claude,
                "claude-haiku-4-5",
                Strength::Fast,
                "claude-haiku-4-5"
            ),
            (
                Runtime::Claude,
                "claude-sonnet-5",
                Strength::Standard,
                "claude-sonnet-5"
            ),
            (
                Runtime::Claude,
                "claude-opus-5-5",
                Strength::Frontier,
                "claude-opus-5-5"
            ),
        ]
    );
    let codex: Vec<_> = SHIPPED_CODEX
        .iter()
        .map(|s| (s.runtime, s.model, s.strength))
        .collect();
    assert_eq!(
        codex,
        [
            (Runtime::Codex, "gpt-6.1-sol", Strength::Frontier),
            (Runtime::Codex, "gpt-6-sol", Strength::Standard),
            (Runtime::Codex, "gpt-6-astra", Strength::Standard),
            (Runtime::Codex, "gpt-6-luna", Strength::Fast),
            (Runtime::Codex, "gpt-5.6-sol", Strength::Standard),
            (Runtime::Codex, "gpt-5.6-terra", Strength::Standard),
            (Runtime::Codex, "gpt-5.6-luna", Strength::Fast),
            (Runtime::Codex, "", Strength::Standard),
        ]
    );
    assert_eq!(SHIPPED_CODEX[7].label, "Codex default");
    assert!(SHIPPED_CODEX[..7].iter().all(|s| s.label == s.model));
}

#[test]
fn doc_of_a_default_config_is_the_empty_table_and_defaults() {
    let doc = default_doc();
    assert!(doc.roles.is_empty(), "a default config migrates to no row");
    let budget = |tool_calls, minutes| proto::BudgetLimit {
        tool_calls,
        minutes,
    };
    assert_eq!(
        doc.limits,
        proto::SettingsLimits {
            budget_s: budget(40, 15),
            budget_m: budget(150, 60),
            budget_l: budget(300, 120),
            stall_after_secs: 600,
            max_writers: 3,
            max_readers: 3,
            max_bounces: 2,
        }
    );
    assert!(validate(&doc).is_empty(), "{:?}", validate(&doc));
}

#[test]
fn origin_marks_only_what_the_file_sets() {
    let text = "[orchestrator]\nmax_writers = 2\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n";
    let origin = origin_of(&text.parse().unwrap());
    assert_eq!(origin.len(), 11);
    for k in proto::SETTINGS_KEYS {
        let want = if k == key::MAX_WRITERS || k == key::ROLES {
            proto::Origin::File
        } else {
            proto::Origin::Default
        };
        assert_eq!(origin[k], want, "{k}");
    }
    // Milestone 9.8 (M9.8.12): `models` is the `[models]` table or any old key it replaced.
    for text in [
        "[orchestrator]\nbuiltin_models = true\n",
        "[orchestrator]\ndefault_runtime = \"codex\"\n",
        "[orchestrator.agent]\nmodel = \"claude-opus-5-5\"\n",
        "[orchestrator.routes.s]\ncandidates = []\n",
        "[models.planner]\nmodel = \"codex:default\"\n",
    ] {
        let origin = origin_of(&text.parse().unwrap());
        assert_eq!(origin[key::ROLES], proto::Origin::File, "{text}");
    }
    for text in [
        "[orchestrator.deciders]\nmode = \"off\"\n",
        "[orchestrator.agent]\nmax_tool_calls = 3\n",
    ] {
        let origin = origin_of(&text.parse().unwrap());
        assert_eq!(origin[key::ROLES], proto::Origin::Default, "{text}");
    }
    let nested = "[orchestrator.budget.m]\nminutes = 9\n";
    let nested = origin_of(&nested.parse().unwrap());
    assert_eq!(nested[key::BUDGET_M_MINUTES], proto::Origin::File);
    assert_eq!(nested[key::BUDGET_M_CALLS], proto::Origin::Default);
}

#[test]
fn validate_accepts_the_default_doc() {
    assert_eq!(validate(&default_doc()), Vec::<String>::new());
}

#[test]
fn validate_refuses_each_limit_out_of_range() {
    let mut doc = default_doc();
    doc.limits.max_writers = 9;
    doc.limits.max_readers = 0;
    doc.limits.max_bounces = 6;
    doc.limits.stall_after_secs = 4;
    doc.limits.budget_m.tool_calls = 0;
    doc.limits.budget_l.minutes = 0;
    assert_eq!(
        validate(&doc),
        [
            "orchestrator.budget.m.tool_calls: must be at least 1",
            "orchestrator.budget.l.minutes: must be at least 1",
            "orchestrator.stall_after_secs: must be between 5 and 7200",
            "orchestrator.max_writers: must be between 1 and 8",
            "orchestrator.max_readers: must be between 1 and 8",
            "orchestrator.max_bounces: must be between 1 and 5",
        ]
    );
}

/// For every limit, a value one past each end of its range is refused by `validate` and
/// by `config::parse` with the same message.
#[test]
fn the_screen_and_the_parser_agree_on_ranges() {
    type Set = fn(&mut proto::SettingsLimits, u64);
    let cases: [(&str, &str, u64, Option<u64>, Set); 10] = [
        ("[orchestrator]", "max_writers", 1, Some(8), |l, v| {
            l.max_writers = v as u8
        }),
        ("[orchestrator]", "max_readers", 1, Some(8), |l, v| {
            l.max_readers = v as u8
        }),
        ("[orchestrator]", "max_bounces", 1, Some(5), |l, v| {
            l.max_bounces = v as u8
        }),
        (
            "[orchestrator]",
            "stall_after_secs",
            5,
            Some(7200),
            |l, v| l.stall_after_secs = v,
        ),
        ("[orchestrator.budget.s]", "tool_calls", 1, None, |l, v| {
            l.budget_s.tool_calls = v as u32
        }),
        ("[orchestrator.budget.s]", "minutes", 1, None, |l, v| {
            l.budget_s.minutes = v as u32
        }),
        ("[orchestrator.budget.m]", "tool_calls", 1, None, |l, v| {
            l.budget_m.tool_calls = v as u32
        }),
        ("[orchestrator.budget.m]", "minutes", 1, None, |l, v| {
            l.budget_m.minutes = v as u32
        }),
        ("[orchestrator.budget.l]", "tool_calls", 1, None, |l, v| {
            l.budget_l.tool_calls = v as u32
        }),
        ("[orchestrator.budget.l]", "minutes", 1, None, |l, v| {
            l.budget_l.minutes = v as u32
        }),
    ];
    let mut checked = 0;
    for (header, name, low, high, set) in cases {
        let mut outside = vec![low - 1];
        outside.extend(high.map(|h| h + 1));
        for value in outside {
            let (_, problems) = crate::parse(&format!("{header}\n{name} = {value}\n"));
            assert_eq!(problems.len(), 1, "{header} {name} = {value}: {problems:?}");
            let parser = format!("{}: {}", problems[0].key, problems[0].message);
            let mut doc = default_doc();
            set(&mut doc.limits, value);
            assert_eq!(validate(&doc), [parser], "{header} {name} = {value}");
            checked += 1;
        }
        for value in [low, high.unwrap_or(low + 1000)] {
            let (_, problems) = crate::parse(&format!("{header}\n{name} = {value}\n"));
            assert!(problems.is_empty(), "{header} {name} = {value}");
            let mut doc = default_doc();
            set(&mut doc.limits, value);
            assert!(validate(&doc).is_empty(), "{header} {name} = {value}");
        }
    }
    assert_eq!(checked, 14);
}

#[test]
fn apply_owned_copies_only_owned_fields() {
    let (from, _) = crate::parse(
        "[orchestrator]\nmax_writers = 5\nmax_readers = 6\nmax_bounces = 4\nstall_after_secs = 90\ngit_timeout_secs = 9\nbuiltin_models = false\n[orchestrator.agent]\nruntime = \"codex\"\nmodel = \"\"\neffort = \"low\"\n[orchestrator.budget.s]\ntool_calls = 7\nminutes = 8\ntokens = 99\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n",
    );
    let from = from.orchestrator;
    let mut live = crate::Orchestrator::default();
    apply_owned(&mut live, &from);
    assert_eq!(doc_of(&live), doc_of(&from));
    assert!(!live.builtin_models);
    assert_eq!(live.roles, from.roles, "the role table is owned");
    assert!(!live.roles.is_empty());
    assert_eq!(live.git_timeout_secs, 60, "not owned");
    assert_eq!(live.agent.agent.effort, proto::Effort::HIGH, "not owned");
    assert_eq!(live.budget_s.tokens, None, "budget tokens are not owned");
}

#[test]
fn load_with_origin_of_a_missing_or_broken_file_is_all_default() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("config.toml");
    let (config, problems, origin) = load_with_origin(&missing);
    assert_eq!(config, crate::Config::default());
    assert!(problems.is_empty());
    assert!(origin.values().all(|o| *o == proto::Origin::Default));
    assert_eq!(origin.len(), 11);
    std::fs::write(&missing, "[orchestrator\nmax_writers = 2\n").unwrap();
    let (_, problems, origin) = load_with_origin(&missing);
    assert_eq!(problems.len(), 1);
    assert!(origin.values().all(|o| *o == proto::Origin::Default));
}

/// Milestone 9.8 (M9.8.12): a row's model and fallback must parse back to themselves,
/// and its effort be an effort name; each problem names the row's key.
#[test]
fn validate_refuses_a_bad_row() {
    use crate::models::{BrainstormChoice, ModelRef, Role, RoleChoice};
    let mut doc = default_doc();
    let good = ModelRef::parse("claude:claude-opus-5-5").unwrap();
    doc.roles.rows.insert(
        Role::ImplementerSmall,
        RoleChoice {
            model: good.clone(),
            effort: Some("High".into()),
            fallback: Some(ModelRef {
                runtime: Runtime::Shell,
                id: None,
            }),
        },
    );
    doc.roles.rows.insert(
        Role::Helper(crate::models::HelperKind::Triage),
        RoleChoice {
            model: ModelRef {
                runtime: Runtime::Codex,
                id: Some("a b".into()),
            },
            effort: None,
            fallback: None,
        },
    );
    doc.roles.brainstorm = Some(BrainstormChoice {
        first: good.clone(),
        second: good.clone(),
        effort: Some(String::new()),
    });
    let problems = validate(&doc);
    let keys: Vec<&str> = problems
        .iter()
        .map(|p| p.split(':').next().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "models.implementer.small.effort",
            "models.implementer.small.fallback",
            "models.helpers.triage.model",
            "models.brainstorm.effort",
        ],
        "{problems:?}"
    );
    doc.roles.rows.clear();
    doc.roles.brainstorm = None;
    doc.roles.rows.insert(
        Role::Reviewer,
        RoleChoice {
            model: good,
            effort: Some("xhigh".into()),
            fallback: Some(ModelRef::parse("codex:default").unwrap()),
        },
    );
    assert!(validate(&doc).is_empty(), "{:?}", validate(&doc));
}

/// Ruling T18-2: the settings name `[orchestrator.design].default`, which the goal
/// dialog's `configured` shows; the product default is `full` (DF §1).
#[test]
fn doc_of_names_the_design_default() {
    assert_eq!(default_doc().design_default, Some(proto::DesignMode::Full));
    let mut o = crate::Orchestrator::default();
    o.design.default = proto::DesignMode::Off;
    assert_eq!(doc_of(&o).design_default, Some(proto::DesignMode::Off));
}
