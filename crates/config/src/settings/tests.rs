use super::*;
use proto::settings::key;
use proto::{ModelEntry, Runtime, Strength};

fn entry(runtime: Runtime, model: &str, strength: Strength) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.into(),
        strength,
        note: String::new(),
    }
}

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
fn doc_of_a_default_config_is_the_builtin_roster_and_defaults() {
    let doc = default_doc();
    assert_eq!(doc.models, crate::default_roster());
    assert_eq!(
        doc.orchestrator,
        proto::OrchestratorDefault {
            runtime: None,
            model: String::new()
        }
    );
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
    assert_eq!(origin.len(), 13);
    for k in proto::SETTINGS_KEYS {
        let want = if k == key::MAX_WRITERS || k == key::MODELS {
            proto::Origin::File
        } else {
            proto::Origin::Default
        };
        assert_eq!(origin[k], want, "{k}");
    }
    let builtin = origin_of(&"[orchestrator]\nbuiltin_models = true\n".parse().unwrap());
    assert_eq!(builtin[key::MODELS], proto::Origin::File);
    let nested =
        "[orchestrator.agent]\nruntime = \"codex\"\n[orchestrator.budget.m]\nminutes = 9\n";
    let nested = origin_of(&nested.parse().unwrap());
    assert_eq!(nested[key::AGENT_RUNTIME], proto::Origin::File);
    assert_eq!(nested[key::AGENT_MODEL], proto::Origin::Default);
    assert_eq!(nested[key::BUDGET_M_MINUTES], proto::Origin::File);
    assert_eq!(nested[key::BUDGET_M_CALLS], proto::Origin::Default);
}

#[test]
fn validate_accepts_the_default_doc() {
    assert_eq!(validate(&default_doc()), Vec::<String>::new());
}

#[test]
fn validate_refuses_no_enabled_model() {
    let mut doc = default_doc();
    doc.models.clear();
    assert_eq!(validate(&doc), ["enable at least one model"]);
}

#[test]
fn validate_refuses_a_claude_model_without_a_name() {
    let mut doc = default_doc();
    doc.models.push(entry(Runtime::Claude, "", Strength::Fast));
    assert_eq!(validate(&doc), ["claude models need a name"]);
}

#[test]
fn validate_refuses_a_model_listed_twice() {
    let mut doc = default_doc();
    doc.models.push(entry(
        Runtime::Claude,
        "claude-sonnet-5",
        Strength::Frontier,
    ));
    doc.models.push(entry(Runtime::Codex, "", Strength::Fast));
    assert_eq!(
        validate(&doc),
        [
            "claude claude-sonnet-5 is listed twice",
            "codex (default) is listed twice"
        ]
    );
}

#[test]
fn validate_refuses_a_long_note() {
    let mut doc = default_doc();
    doc.models[0].note = "n".repeat(81);
    assert_eq!(
        validate(&doc),
        ["note of claude-haiku-4-5: at most 80 characters"]
    );
    doc.models[0].note = "é".repeat(80);
    assert!(validate(&doc).is_empty());
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

#[test]
fn validate_refuses_an_orchestrator_model_without_a_runtime() {
    let mut doc = default_doc();
    doc.orchestrator.model = "claude-opus-5-5".into();
    assert_eq!(
        validate(&doc),
        ["orchestrator.agent.model: choose a runtime first"]
    );
}

#[test]
fn validate_refuses_an_orchestrator_model_that_is_not_enabled() {
    let mut doc = default_doc();
    doc.orchestrator.runtime = Some(Runtime::Codex);
    doc.orchestrator.model = "claude-opus-5-5".into();
    assert_eq!(
        validate(&doc),
        ["orchestrator.agent.model: claude-opus-5-5 is not an enabled codex model"]
    );
    doc.orchestrator.runtime = Some(Runtime::Claude);
    assert!(validate(&doc).is_empty());
    doc.orchestrator.model = String::new();
    doc.orchestrator.runtime = Some(Runtime::Codex);
    assert!(
        validate(&doc).is_empty(),
        "an empty model is the runtime's default"
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
fn warnings_name_a_strength_on_one_runtime() {
    assert_eq!(
        warnings(&default_doc()),
        [
            "no codex model is fast: the cross-runtime reviewer cannot be chosen for fast tasks",
            "no codex model is frontier: the cross-runtime reviewer cannot be chosen for frontier tasks",
        ]
    );
    let mut doc = default_doc();
    doc.models = vec![
        entry(Runtime::Codex, "gpt-6-luna", Strength::Fast),
        entry(Runtime::Claude, "claude-haiku-4-5", Strength::Fast),
        entry(Runtime::Codex, "gpt-6-sol", Strength::Standard),
    ];
    assert_eq!(
        warnings(&doc),
        [
            "no claude model is standard: the cross-runtime reviewer cannot be chosen for standard tasks"
        ]
    );
    assert!(validate(&doc).is_empty(), "a warning never refuses");
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
    assert_eq!(live.git_timeout_secs, 60, "not owned");
    assert_eq!(live.agent.agent.effort, proto::Effort::High, "not owned");
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
    assert_eq!(origin.len(), 13);
    std::fs::write(&missing, "[orchestrator\nmax_writers = 2\n").unwrap();
    let (_, problems, origin) = load_with_origin(&missing);
    assert_eq!(problems.len(), 1);
    assert!(origin.values().all(|o| *o == proto::Origin::Default));
}

/// Controller ruling (fix round 1): a control character in a model name or note refuses,
/// naming the row; a newline would otherwise break the file the screen writes.
#[test]
fn validate_refuses_control_characters_in_names_and_notes() {
    for bad in [
        "a\nb",
        "a\tb",
        "a\x1b[31mb",
        "a\u{2028}b",
        "a\u{2029}b",
        "a\u{202E}b",
    ] {
        let mut doc = default_doc();
        doc.models[1].model = bad.into();
        assert_eq!(
            validate(&doc),
            ["model 2 (claude): its name holds a control character"],
            "{bad:?}"
        );
        let mut doc = default_doc();
        doc.models[3].note = bad.into();
        assert_eq!(
            validate(&doc),
            ["model 4 (codex): its note holds a control character"],
            "{bad:?}"
        );
    }
}
