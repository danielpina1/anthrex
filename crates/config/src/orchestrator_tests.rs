use std::collections::HashSet;

use super::*;
use crate::{Problem, parse};

fn keys(problems: &[Problem]) -> HashSet<String> {
    problems.iter().map(|p| p.key.clone()).collect()
}

#[test]
fn defaults_when_absent() {
    let (config, problems) = parse("");
    assert!(problems.is_empty());
    let o = &config.orchestrator;

    assert_eq!(o.max_writers, 3);
    assert_eq!(o.max_readers, 3);
    assert_eq!(o.max_bounces, 2);
    assert_eq!(o.max_tasks, 50);
    assert_eq!(o.max_windows, 60);
    assert_eq!(o.default_runtime, proto::Runtime::Claude);
    assert!(o.review_small);
    assert_eq!(
        o.budget_s,
        proto::Budget {
            tool_calls: 40,
            minutes: 15,
            tokens: None
        }
    );
    assert_eq!(
        o.budget_m,
        proto::Budget {
            tool_calls: 150,
            minutes: 60,
            tokens: None
        }
    );
    assert_eq!(
        o.budget_l,
        proto::Budget {
            tool_calls: 300,
            minutes: 120,
            tokens: None
        }
    );
    assert_eq!(o.stall_after_secs, 600);
    assert_eq!(o.rate_limit_retry_secs, 300);
    assert_eq!(o.denials_before_block, 3);
    assert_eq!(o.git_timeout_secs, 60);
    assert_eq!(o.worker_permission_mode, "acceptEdits");
    assert_eq!(
        o.worker_allowed_tools,
        vec![
            "Bash",
            "Edit",
            "Write",
            "Read",
            "Glob",
            "Grep",
            "Agent",
            "TodoWrite"
        ]
    );
    assert_eq!(o.worker_codex_sandbox, "workspace-write");
    assert!(o.worker_sandbox);
    assert_eq!(o.claude, ClaudeHeadless::default());
    assert_eq!(o.claude.auth, ClaudeAuth::Login);
    assert_eq!(o.claude.api_key_helper, None);
    assert!(o.builtin_models);
    assert_eq!(o.models, default_roster());
    assert_eq!(o.models.len(), 4);
    assert_eq!(o.models[0].runtime, proto::Runtime::Claude);
    assert_eq!(o.models[0].model, "claude-haiku-4-5");
    assert_eq!(o.models[0].strength, proto::Strength::Fast);
    assert_eq!(o.models[1].model, "claude-sonnet-5");
    assert_eq!(o.models[1].strength, proto::Strength::Standard);
    assert_eq!(o.models[2].model, "claude-opus-5");
    assert_eq!(o.models[2].strength, proto::Strength::Frontier);
    assert_eq!(o.models[3].runtime, proto::Runtime::Codex);
    assert_eq!(o.models[3].model, "");
    assert_eq!(o.models[3].strength, proto::Strength::Standard);
    assert_eq!(o.profile, proto::ProfileSpec::default());
}

#[test]
fn keys_are_read() {
    let text = r##"
[orchestrator]
max_writers = 5
max_readers = 2
max_bounces = 4
max_tasks = 77
max_windows = 88
default_runtime = "codex"
review.small = "off"
stall_after_secs = 650
rate_limit_retry_secs = 333
denials_before_block = 9
git_timeout_secs = 99
worker_permission_mode = "dontAsk"
worker_allowed_tools = ["Read", "Grep"]
worker_codex_sandbox = "read-only"
worker_sandbox = false
builtin_models = false

[orchestrator.budget.s]
tool_calls = 41
minutes = 16
tokens = 1001

[orchestrator.budget.m]
tool_calls = 151
minutes = 61
tokens = 2002

[orchestrator.budget.l]
tool_calls = 301
minutes = 121
tokens = 3003

[orchestrator.claude]
auth = "login"
api_key_helper = "helper.sh"

[[orchestrator.models]]
runtime = "codex"
model = "my-model"
strength = "frontier"
note = "custom"

[orchestrator.profile]
modules = ["a"]
hub = ["b"]
source = ["c"]
check = "cargo test"
check_timeout_secs = 111
single_test = "cargo test {test}"
test_passed = "ok"
setup = "cargo build"
generated = ["gen/**"]
protected = ["x/**"]

[orchestrator.profile.env]
FOO = "bar"
"##;
    let (config, problems) = parse(text);
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    let o = &config.orchestrator;

    assert_eq!(o.max_writers, 5);
    assert_eq!(o.max_readers, 2);
    assert_eq!(o.max_bounces, 4);
    assert_eq!(o.max_tasks, 77);
    assert_eq!(o.max_windows, 88);
    assert_eq!(o.default_runtime, proto::Runtime::Codex);
    assert!(!o.review_small);
    assert_eq!(
        o.budget_s,
        proto::Budget {
            tool_calls: 41,
            minutes: 16,
            tokens: Some(1001)
        }
    );
    assert_eq!(
        o.budget_m,
        proto::Budget {
            tool_calls: 151,
            minutes: 61,
            tokens: Some(2002)
        }
    );
    assert_eq!(
        o.budget_l,
        proto::Budget {
            tool_calls: 301,
            minutes: 121,
            tokens: Some(3003)
        }
    );
    assert_eq!(o.stall_after_secs, 650);
    assert_eq!(o.rate_limit_retry_secs, 333);
    assert_eq!(o.denials_before_block, 9);
    assert_eq!(o.git_timeout_secs, 99);
    assert_eq!(o.worker_permission_mode, "dontAsk");
    assert_eq!(o.worker_allowed_tools, vec!["Read", "Grep"]);
    assert_eq!(o.worker_codex_sandbox, "read-only");
    assert!(!o.worker_sandbox);
    assert_eq!(o.claude.auth, ClaudeAuth::Login);
    assert_eq!(o.claude.api_key_helper, Some("helper.sh".to_string()));
    assert!(!o.builtin_models);
    assert_eq!(
        o.models,
        vec![proto::ModelEntry {
            runtime: proto::Runtime::Codex,
            model: "my-model".to_string(),
            strength: proto::Strength::Frontier,
            note: "custom".to_string(),
        }]
    );
    assert_eq!(o.profile.modules, Some(vec!["a".to_string()]));
    assert_eq!(o.profile.hub, Some(vec!["b".to_string()]));
    assert_eq!(o.profile.source, Some(vec!["c".to_string()]));
    assert_eq!(o.profile.check, Some("cargo test".to_string()));
    assert_eq!(o.profile.check_timeout_secs, Some(111));
    assert_eq!(o.profile.single_test, Some("cargo test {test}".to_string()));
    assert_eq!(o.profile.test_passed, Some("ok".to_string()));
    assert_eq!(o.profile.setup, Some("cargo build".to_string()));
    assert_eq!(o.profile.generated, Some(vec!["gen/**".to_string()]));
    assert_eq!(o.profile.protected, Some(vec!["x/**".to_string()]));
    assert_eq!(
        o.profile.env,
        Some(std::collections::BTreeMap::from([(
            "FOO".to_string(),
            "bar".to_string()
        )]))
    );
}

#[test]
fn out_of_range_values_fall_back_with_problems() {
    let (config, problems) = parse(
        r#"
[orchestrator]
max_writers = 0
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.max_writers".to_string()])
    );
    assert_eq!(config.orchestrator.max_writers, 3);
    assert_eq!(
        problems[0].to_string(),
        "orchestrator.max_writers: must be between 1 and 8 (using 3)"
    );

    let (config, problems) = parse(
        r#"
[orchestrator]
max_writers = 9
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.max_writers".to_string()])
    );
    assert_eq!(config.orchestrator.max_writers, 3);

    let (config, problems) = parse(
        r#"
[orchestrator]
stall_after_secs = 1
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.stall_after_secs".to_string()])
    );
    assert_eq!(config.orchestrator.stall_after_secs, 600);

    let (config, problems) = parse(
        r#"
[orchestrator]
git_timeout_secs = 4
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.git_timeout_secs".to_string()])
    );
    assert_eq!(config.orchestrator.git_timeout_secs, 60);

    let (config, problems) = parse(
        r#"
[orchestrator]
worker_codex_sandbox = "yolo"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.worker_codex_sandbox".to_string()])
    );
    assert_eq!(config.orchestrator.worker_codex_sandbox, "workspace-write");

    let (config, problems) = parse(
        r#"
[orchestrator]
default_runtime = "shell"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.default_runtime".to_string()])
    );
    assert_eq!(config.orchestrator.default_runtime, proto::Runtime::Claude);
}

#[test]
fn unknown_orchestrator_keys_are_reported() {
    let (_, problems) = parse(
        r#"
[orchestrator]
max_parallel = 3
done_quiet_secs = 5

[orchestrator.budget]
xl = 5

[orchestrator.budget.x]
tool_calls = 10

[orchestrator.budget.y]
"#,
    );
    // Review E-M5 (F4): an unknown rung is reported itself, a scalar or an empty
    // table included.
    assert_eq!(
        keys(&problems),
        HashSet::from([
            "orchestrator.max_parallel".to_string(),
            "orchestrator.done_quiet_secs".to_string(),
            "orchestrator.budget.xl".to_string(),
            "orchestrator.budget.x".to_string(),
            "orchestrator.budget.y".to_string(),
        ])
    );
    for p in &problems {
        assert_eq!(p.message, "unknown key, ignored");
    }
}

#[test]
fn review_small_is_read_as_on_or_off() {
    let (config, problems) = parse(
        r#"
[orchestrator]
review.small = "off"
"#,
    );
    assert!(problems.is_empty());
    assert!(!config.orchestrator.review_small);

    let (config, problems) = parse(
        r#"
[orchestrator.review]
small = "off"
"#,
    );
    assert!(problems.is_empty());
    assert!(!config.orchestrator.review_small);

    let (config, problems) = parse(
        r#"
[orchestrator]
review.small = "on"
"#,
    );
    assert!(problems.is_empty());
    assert!(config.orchestrator.review_small);

    let (config, problems) = parse(
        r#"
[orchestrator]
review.small = false
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.review.small".to_string()])
    );
    assert!(config.orchestrator.review_small);
    assert_eq!(
        problems[0].to_string(),
        "orchestrator.review.small: must be \"on\" or \"off\" (using \"on\")"
    );

    let (config, problems) = parse(
        r#"
[orchestrator]
review.small = "no"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.review.small".to_string()])
    );
    assert!(config.orchestrator.review_small);

    let (_, problems) = parse(
        r#"
[orchestrator.review]
large = 1
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.review.large".to_string()])
    );
    assert_eq!(problems[0].message, "unknown key, ignored");
}

#[test]
fn claude_auth_and_helper_are_read() {
    let (config, problems) = parse(
        r#"
[orchestrator.claude]
auth = "login"
api_key_helper = "helper.sh"
"#,
    );
    assert!(problems.is_empty());
    assert_eq!(config.orchestrator.claude.auth, ClaudeAuth::Login);
    assert_eq!(
        config.orchestrator.claude.api_key_helper,
        Some("helper.sh".to_string())
    );

    let (config, problems) = parse(
        r#"
[orchestrator.claude]
auth = "token"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.claude.auth".to_string()])
    );
    assert_eq!(config.orchestrator.claude.auth, ClaudeAuth::Login);
}

#[test]
fn worker_tools_and_sandbox() {
    let (config, problems) = parse(
        r#"
[orchestrator]
worker_allowed_tools = ["Read", "Grep"]
"#,
    );
    assert!(problems.is_empty());
    assert_eq!(
        config.orchestrator.worker_allowed_tools,
        vec!["Read", "Grep"]
    );

    let (config, problems) = parse(
        r#"
[orchestrator]
worker_allowed_tools = []
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.worker_allowed_tools".to_string()])
    );
    assert_eq!(
        config.orchestrator.worker_allowed_tools,
        vec![
            "Bash",
            "Edit",
            "Write",
            "Read",
            "Glob",
            "Grep",
            "Agent",
            "TodoWrite"
        ]
    );

    let (config, problems) = parse(
        r#"
[orchestrator]
worker_codex_sandbox = "yolo"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.worker_codex_sandbox".to_string()])
    );
    assert_eq!(config.orchestrator.worker_codex_sandbox, "workspace-write");
}

#[test]
fn budget_tokens_is_optional() {
    let (config, problems) = parse("");
    assert!(problems.is_empty());
    assert_eq!(config.orchestrator.budget_s.tokens, None);
    assert_eq!(config.orchestrator.budget_m.tokens, None);
    assert_eq!(config.orchestrator.budget_l.tokens, None);

    let (config, problems) = parse(
        r#"
[orchestrator.budget.s]
tokens = 500
"#,
    );
    assert!(problems.is_empty());
    assert_eq!(config.orchestrator.budget_s.tokens, Some(500));

    let (config, problems) = parse(
        r#"
[orchestrator.budget.s]
tokens = 0
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.budget.s.tokens".to_string()])
    );
    assert_eq!(config.orchestrator.budget_s.tokens, None);
}

#[test]
fn worker_sandbox_defaults_to_true() {
    let (config, problems) = parse("");
    assert!(problems.is_empty());
    assert!(config.orchestrator.worker_sandbox);

    let (config, problems) = parse(
        r#"
[orchestrator]
worker_sandbox = false
"#,
    );
    assert!(problems.is_empty());
    assert!(!config.orchestrator.worker_sandbox);
}

#[path = "orchestrator_tests_roster.rs"]
mod roster;

#[path = "orchestrator_tests_confine.rs"]
mod confine;

/// Final fix batch F2 (C-I3), decision 50's recorded ruling: whether `--settings` hooks
/// and `--mcp-config` still apply under `--bare` is not verified, so `auth = "api_key"`
/// is refused at config load with a problem, and `login` is kept.
#[test]
fn claude_api_key_auth_is_refused_at_load() {
    let (config, problems) = parse(
        r#"
[orchestrator.claude]
auth = "api_key"
api_key_helper = "helper.sh"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.claude.auth".to_string()])
    );
    assert!(
        problems[0].message.contains("--bare"),
        "{}",
        problems[0].message
    );
    assert_eq!(problems[0].default, "login");
    assert_eq!(config.orchestrator.claude.auth, ClaudeAuth::Login);
    // The helper is still read, for the day `api_key` is verified.
    assert_eq!(
        config.orchestrator.claude.api_key_helper,
        Some("helper.sh".to_string())
    );
}
