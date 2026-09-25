//! Final fix batch F2 (C-I4): a plan's `[profile.env]` may not set a variable that a
//! launch scrubs or sets on purpose, or one that chooses which settings, config,
//! credentials or program an agent loads (`config::reserved_env`, the one list the
//! scrubs use too).

use crate::run::test_support::*;

fn plan_env(lines: &str) -> String {
    plan_with(
        &format!("{PROFILE}[profile.env]\n{lines}"),
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    )
}

#[test]
fn reserved_env_keys_are_refused() {
    let text = plan_env(
        "CLAUDE_CONFIG_DIR = \"{worktree}/.cfg\"\nCODEX_HOME = \"{worktree}/.codex\"\n\
         GIT_INDEX_FILE = \"x\"\nCLAUDE_CODE_ENTRYPOINT = \"x\"\nANTHREX_SOCKET = \"x\"\n\
         TMPDIR = \"x\"\nPATH = \"{worktree}/bin\"\nANTHROPIC_BASE_URL = \"x\"\n\
         BASH_ENV = \"{worktree}/env.sh\"\nCARGO_TARGET_DIR = \"{worktree}/target\"\n",
    );
    let refused: Vec<String> = errors_of(&text)
        .into_iter()
        .map(|e| {
            assert_eq!((e.task.as_deref(), e.field.as_str()), (None, "profile.env"));
            assert_eq!(e.rule, "profile");
            assert!(e.message.contains("may not be set"), "{}", e.message);
            e.message.split_whitespace().nth(1).unwrap().to_string()
        })
        .collect();
    // In the plan's key order (a TOML table is sorted).
    assert_eq!(
        refused,
        [
            "ANTHREX_SOCKET",
            "ANTHROPIC_BASE_URL",
            "BASH_ENV",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
            "GIT_INDEX_FILE",
            "PATH",
            "TMPDIR",
        ]
    );
}

#[test]
fn a_project_variable_is_accepted() {
    run_ok(&plan_env("CARGO_TARGET_DIR = \"{worktree}/target\"\n"));
}
