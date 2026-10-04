//! `[orchestrator.design]`, `[orchestrator.design.budget.*]` and the
//! `[orchestrator.routes.brainstorm]` list (milestone 9.6 task M9.6.3, DF §1 and §2.2).

use proto::{Budget, DesignMode, Effort, Runtime};

use super::*;
use crate::{Candidate, Pick, Problem, RouteList, parse};

fn design_of(text: &str) -> (DesignConfig, Vec<Problem>) {
    let (config, problems) = parse(text);
    (config.orchestrator.design, problems)
}

fn problem(key: &str, message: &str, default: &str) -> Problem {
    Problem {
        key: key.to_string(),
        message: message.to_string(),
        default: default.to_string(),
    }
}

fn budget(tool_calls: u32, minutes: u32, tokens: Option<u64>) -> Budget {
    Budget {
        tool_calls,
        minutes,
        tokens,
    }
}

/// DF §1's table and §2.2's budgets: `full`, `docs/anthrex`, `false`, `5`, `60`,
/// `40 calls / 15 min` for a brainstormer and `20 calls / 10 min` for a reviewer.
#[test]
fn the_defaults_are_the_spec_defaults() {
    let (design, problems) = design_of("");
    assert!(problems.is_empty(), "{problems:?}");
    let expected = DesignConfig {
        default: DesignMode::Full,
        docs_dir: "docs/anthrex".to_string(),
        commit_brainstorm: false,
        max_questions: 5,
        phase_minutes: 60,
        budget: DesignBudget {
            brainstormer: budget(40, 15, None),
            doc_reviewer: budget(20, 10, None),
        },
    };
    assert_eq!(design, expected);
    assert_eq!(DesignConfig::default(), expected);
    assert_eq!(config_default().orchestrator.design, expected);
    // No brainstorm list: decision 10's runtime defaults apply.
    let (config, _) = parse("");
    assert_eq!(
        config.orchestrator.tuning.routes.brainstorm,
        RouteList::default()
    );
}

fn config_default() -> crate::Config {
    crate::Config::default()
}

#[test]
fn every_design_key_is_read() {
    let (design, problems) = design_of(
        r#"
[orchestrator.design]
default = "off"
docs_dir = "docs/design"
commit_brainstorm = true
max_questions = 0
phase_minutes = 90

[orchestrator.design.budget.brainstormer]
tool_calls = 60
minutes = 20
tokens = 500000

[orchestrator.design.budget.doc_reviewer]
tool_calls = 10
minutes = 5
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        design,
        DesignConfig {
            default: DesignMode::Off,
            docs_dir: "docs/design".to_string(),
            commit_brainstorm: true,
            max_questions: 0,
            phase_minutes: 90,
            budget: DesignBudget {
                brainstormer: budget(60, 20, Some(500000)),
                doc_reviewer: budget(10, 5, None),
            },
        }
    );
}

/// `docs_dir = ""` means never commit (DF §1); it is a value, not a problem.
#[test]
fn an_empty_docs_dir_is_kept() {
    let (design, problems) = design_of("[orchestrator.design]\ndocs_dir = \"\"\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(design.docs_dir, "");
}

/// 9.5's unknown-key reporting: `[orchestrator.design]`, its budget table, each
/// budget's keys, and `[orchestrator.routes.brainstorm]`'s keys.
#[test]
fn unknown_design_keys_are_reported() {
    let (config, problems) = parse(
        r#"
[orchestrator.design]
questions = 3
max_questions = 2

[orchestrator.design.budget]
planner = { tool_calls = 3 }

[orchestrator.design.budget.brainstormer]
calls = 3
minutes = 9

[orchestrator.routes.brainstorm]
candidates = [{ runtime = "claude", model = "claude-opus-5-5", lens = "A" }]
lenses = 2
"#,
    );
    let unknown = |key: &str| problem(key, "unknown key, ignored", "nothing");
    let mut keys: Vec<Problem> = problems;
    keys.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(
        keys,
        vec![
            unknown("orchestrator.design.budget.brainstormer.calls"),
            unknown("orchestrator.design.budget.planner"),
            unknown("orchestrator.design.questions"),
            unknown("orchestrator.routes.brainstorm.candidates[0].lens"),
            unknown("orchestrator.routes.brainstorm.lenses"),
        ]
    );
    assert_eq!(
        keys[2].to_string(),
        "orchestrator.design.questions: unknown key, ignored (using nothing)"
    );
    // The known keys beside them are still read.
    let design = &config.orchestrator.design;
    assert_eq!(design.max_questions, 2);
    assert_eq!(design.budget.brainstormer, budget(40, 9, None));
}

#[test]
fn bad_design_values_keep_their_defaults() {
    let (design, problems) = design_of(
        r#"
[orchestrator.design]
default = "sometimes"
docs_dir = 7
commit_brainstorm = "yes"
max_questions = 99
phase_minutes = 0

[orchestrator.design.budget.brainstormer]
tool_calls = 0
minutes = -1
tokens = 0

[orchestrator.design.budget.doc_reviewer]
minutes = "ten"
"#,
    );
    assert_eq!(design, DesignConfig::default());
    let keys: Vec<&str> = problems.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "orchestrator.design.default",
            "orchestrator.design.docs_dir",
            "orchestrator.design.commit_brainstorm",
            "orchestrator.design.max_questions",
            "orchestrator.design.phase_minutes",
            "orchestrator.design.budget.brainstormer.tool_calls",
            "orchestrator.design.budget.brainstormer.minutes",
            "orchestrator.design.budget.brainstormer.tokens",
            "orchestrator.design.budget.doc_reviewer.minutes",
        ]
    );
    assert_eq!(
        problems[0],
        problem(
            "orchestrator.design.default",
            "must be \"full\" or \"off\"",
            "\"full\""
        )
    );
    assert_eq!(
        problems[3],
        problem(
            "orchestrator.design.max_questions",
            "must be between 0 and 20",
            "5"
        )
    );
    assert_eq!(
        problems[4],
        problem(
            "orchestrator.design.phase_minutes",
            "must be between 5 and 1440",
            "60"
        )
    );
    assert_eq!(
        problems[5],
        problem(
            "orchestrator.design.budget.brainstormer.tool_calls",
            "must be at least 1",
            "40"
        )
    );
}

/// The documents are committed into the repository (decision 24), so `docs_dir` is a
/// relative path that stays inside it.
#[test]
fn a_docs_dir_outside_the_repository_is_refused() {
    for dir in ["/etc/anthrex", "../elsewhere", "docs/../../up", "docs\\win"] {
        let text = format!("[orchestrator.design]\ndocs_dir = {dir:?}\n");
        let (design, problems) = design_of(&text);
        assert_eq!(design.docs_dir, "docs/anthrex", "{dir}");
        assert_eq!(
            problems,
            vec![problem(
                "orchestrator.design.docs_dir",
                "must be a relative path inside the repository, or \"\"",
                "\"docs/anthrex\""
            )],
            "{dir}"
        );
    }
}

#[test]
fn bad_design_table_shapes_are_problems() {
    let (design, problems) = design_of("[orchestrator]\ndesign = 3\n");
    assert_eq!(design, DesignConfig::default());
    assert_eq!(
        problems,
        vec![problem(
            "orchestrator.design",
            "expected a table",
            "table of defaults"
        )]
    );
    let (design, problems) = design_of("[orchestrator.design]\nbudget = 3\n");
    assert_eq!(design, DesignConfig::default());
    let keys: Vec<&str> = problems.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["orchestrator.design.budget"]);
    let (_, problems) = design_of("[orchestrator.design.budget]\nbrainstormer = 3\n");
    let keys: Vec<&str> = problems.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["orchestrator.design.budget.brainstormer"]);
}

/// Decision 10: `[orchestrator.routes.brainstorm]` is a 9.5 route list, read like the
/// others and checked against the roster.
#[test]
fn the_brainstorm_route_list_is_read() {
    let (config, problems) = parse(
        r#"
[orchestrator.routes.brainstorm]
candidates = [
  { runtime = "claude", model = "claude-opus-5-5", effort = "high" },
  { runtime = "claude", model = "claude-sonnet-5" },
  { runtime = "claude", model = "not-in-the-roster" },
]
"#,
    );
    assert_eq!(
        problems.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(),
        ["orchestrator.routes.brainstorm.candidates[2]"]
    );
    assert_eq!(
        config.orchestrator.tuning.routes.brainstorm,
        RouteList {
            candidates: vec![
                Candidate {
                    runtime: Runtime::Claude,
                    model: "claude-opus-5-5".to_string(),
                    effort: Some(Effort::High),
                },
                Candidate {
                    runtime: Runtime::Claude,
                    model: "claude-sonnet-5".to_string(),
                    effort: None,
                },
            ],
            pick: Pick::First,
        }
    );
}
