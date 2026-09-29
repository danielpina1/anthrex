//! Milestone 9.1 task M9.1.5: the onboarding scout proposes the tier keys (decision 12).

use proto::{ModuleNames, ScoutKind};
use serde_json::{Value, json};

use super::report::validate;

fn report(profile: Value) -> Value {
    json!({
        "summary": "a Rust workspace",
        "files": [{"path": "Cargo.toml", "why": "the workspace"}],
        "profile": profile,
    })
}

#[test]
fn onboarding_report_accepts_the_new_keys_and_refuses_unknown_ones() {
    let ok = validate(
        &report(json!({
            "check": "cargo test",
            "build_check": "cargo build --workspace --all-targets",
            "module_test": "cargo test -p {module} {filter:-E %}",
            "module_tests": "cargo test {modules:-p %}",
            "module_graph": "cargo",
            "module_names": "cargo",
            "full_triggers": ["Cargo.lock", ".github/**"],
            "slow_tests": "test(e2e)",
            "timing_tests": "test(timing)",
            "skip_markers": ["#[ignore", "x".repeat(64)],
            "test_paths": ["**/tests/**"],
            "full_shards": 16,
            "toolchain_id": "rustc -Vv",
        })),
        ScoutKind::Onboarding,
    )
    .unwrap();
    let p = ok.profile.unwrap();
    assert_eq!(
        p.build_check.as_deref(),
        Some("cargo build --workspace --all-targets")
    );
    assert_eq!(
        p.module_test.as_deref(),
        Some("cargo test -p {module} {filter:-E %}")
    );
    assert_eq!(p.module_tests.as_deref(), Some("cargo test {modules:-p %}"));
    assert_eq!(p.module_graph.as_deref(), Some("cargo"));
    assert_eq!(p.module_names, Some(ModuleNames::Cargo));
    assert_eq!(p.full_triggers, ["Cargo.lock", ".github/**"]);
    assert_eq!(p.slow_tests.as_deref(), Some("test(e2e)"));
    assert_eq!(p.timing_tests.as_deref(), Some("test(timing)"));
    assert_eq!(p.skip_markers.len(), 2);
    assert_eq!(p.test_paths, ["**/tests/**"]);
    assert_eq!(p.full_shards, Some(16));
    assert_eq!(p.toolchain_id.as_deref(), Some("rustc -Vv"));

    let many = vec!["x"; 33];
    let cases = [
        (
            json!({"module_names": "crate"}),
            "profile.module_names: expected cargo or dir",
        ),
        (
            json!({"full_shards": 0}),
            "profile.full_shards: must be between 1 and 16",
        ),
        (
            json!({"full_shards": 17}),
            "profile.full_shards: must be between 1 and 16",
        ),
        (
            json!({"full_shards": "2"}),
            "profile.full_shards: expected an integer",
        ),
        (
            json!({"build_check": "x".repeat(2001)}),
            "profile.build_check: must be at most 2000 characters",
        ),
        (
            json!({"skip_markers": ["x".repeat(65)]}),
            "profile.skip_markers[0]: must be at most 64 characters",
        ),
        (
            json!({"skip_markers": many}),
            "profile.skip_markers: must have at most 32 items",
        ),
        (
            json!({"module_graph": ""}),
            "profile.module_graph: must not be empty",
        ),
        (
            json!({"module_tier": "x"}),
            "profile.module_tier: not in the schema",
        ),
        (
            json!({"full_trigger": ["Cargo.lock"]}),
            "profile.full_trigger: not in the schema",
        ),
    ];
    for (profile, message) in cases {
        assert_eq!(
            validate(&report(profile.clone()), ScoutKind::Onboarding),
            Err(format!("invalid arguments: {message}")),
            "{profile}"
        );
    }
}

#[test]
fn onboarding_contract_is_exact() {
    assert_eq!(
        super::contract::ONBOARDING_CONTRACT,
        "You are the onboarding scout of anthrex, a tool that runs coding agents on this repository. You work out how it is set up, built and tested. You never change anything.
1. This directory is a disposable copy of the repository at its current commit. Your session is read-only: nothing you run can write a file or reach the network. Commands that only read (listing files, printing a Makefile's targets, showing git history) work; builds and tests do not, and you should not try them.
2. Read manifests, lock files, CI configuration, READMEs and agent instruction files (AGENTS.md, CLAUDE.md and similar) before source code.
3. Find: the languages; what counts as one module (globs); hub paths that many modules depend on; where behaviour lives (source globs); generated files that builds rewrite on their own, taken from the lock files you find (for example Cargo.lock, package-lock.json, yarn.lock, pnpm-lock.yaml, poetry.lock, uv.lock, go.sum); protected files that configure or instruct coding agents: always .claude/**, .mcp.json, .codex/**, **/CLAUDE.md and **/AGENTS.md, plus any other agent configuration, hook, MCP server or instruction file you find (for example .cursor/**, .github/copilot-instructions.md, GEMINI.md); a setup command to run once in a fresh copy; one check command that builds, tests and lints everything CI checks; a command that runs one named test, with {test} where the name goes; a regular expression, with {test} where the name goes, that matches a line of that command's output only when that test ran and passed; the name of one existing test that passes; the manifests and convention files you relied on; environment variables every copy needs, with {worktree} for the copy's path.
4. Also find, when the test runner allows it: a command that builds and lints everything without running tests (build_check); a command that runs one module's tests, with {module} where the module's name goes and {filter:<flag> %} where a test filter expression goes (module_test), or several modules' at once with {modules:<flag> %} (module_tests); how modules depend on each other (module_graph: cargo for a Cargo workspace, otherwise a command that prints JSON mapping each module to the modules it depends on, or none); paths whose change must run every test, such as root manifests, lock files, CI configuration, toolchain files and build scripts (full_triggers); a filter expression that selects slow tests, such as end-to-end and terminal tests (slow_tests), and one for timing-sensitive tests (timing_tests); the markers that skip or disable a test in this repository's languages (skip_markers); the globs of test files (test_paths); into how many parts the check can be split with {shard} and {shards} (full_shards); and a command that prints the toolchain's version (toolchain_id). Propose cargo nextest commands only if cargo nextest --version succeeds here; nothing requires it.
5. Propose the commands CI would run. anthrex runs each of them itself afterwards, in a fresh copy and under the same restrictions its runs use, and proposes only the ones that pass.
6. Call the anthrex tool submit_scout_report (in Claude: mcp__anthrex__submit_scout_report) exactly once, with a short summary, the files that matter, and the profile. Then stop."
    );
}
