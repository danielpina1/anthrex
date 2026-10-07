use super::*;
use std::path::PathBuf;

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn confined() -> SandboxPlan {
    SandboxPlan::new(
        Mode::Confined,
        p("/w/task"),
        vec![p("/d/git"), p("/d/tmp")],
        vec![p("/d/git/config"), p("/d/git/refs")],
    )
}

#[test]
fn version_table_picks_the_dialect() {
    assert_eq!(
        CodexSandboxDialect::for_version(Some((0, 159, 9))),
        CodexSandboxDialect::Legacy
    );
    assert_eq!(
        CodexSandboxDialect::for_version(Some((0, 160, 0))),
        CodexSandboxDialect::Profiles
    );
    assert_eq!(
        CodexSandboxDialect::for_version(Some((1, 0, 0))),
        CodexSandboxDialect::Profiles
    );
}

#[test]
fn unknown_version_is_profiles() {
    assert_eq!(
        CodexSandboxDialect::for_version(None),
        CodexSandboxDialect::Profiles
    );
}

#[test]
fn mode_maps_the_configured_values() {
    assert_eq!(Mode::from_codex_sandbox("workspace-write"), Mode::Confined);
    assert_eq!(
        Mode::from_codex_sandbox("danger-full-access"),
        Mode::FullAccess
    );
    assert_eq!(Mode::from_codex_sandbox("read-only"), Mode::ReadOnly);
    assert_eq!(Mode::from_codex_sandbox("bogus"), Mode::ReadOnly);
}

#[test]
fn profiles_render_a_confined_plan() {
    let args = CodexSandboxDialect::Profiles
        .render(&confined(), false, false)
        .unwrap();
    assert_eq!(
        args,
        [
            "-c",
            "default_permissions=\"anthrex\"",
            "-c",
            "permissions.anthrex.extends=\":read-only\"",
            "-c",
            "permissions.anthrex.network.enabled=false",
            "-c",
            "permissions.anthrex.filesystem={\"/w/task\"=\"write\",\"/d/git\"=\"write\",\"/d/tmp\"=\"write\",\"/d/git/config\"=\"read\",\"/d/git/refs\"=\"read\"}",
        ]
    );
}

#[test]
fn profiles_render_the_resume_identically() {
    let first = CodexSandboxDialect::Profiles
        .render(&confined(), false, false)
        .unwrap();
    let resume = CodexSandboxDialect::Profiles
        .render(&confined(), true, false)
        .unwrap();
    assert_eq!(first, resume);
}

#[test]
fn profiles_render_read_only_and_full_access() {
    let ro = SandboxPlan::new(Mode::ReadOnly, p("/w"), vec![], vec![]);
    assert_eq!(
        CodexSandboxDialect::Profiles
            .render(&ro, false, false)
            .unwrap(),
        ["-c", "default_permissions=\":read-only\""]
    );
    let full = SandboxPlan::new(Mode::FullAccess, p("/w"), vec![], vec![]);
    assert_eq!(
        CodexSandboxDialect::Profiles
            .render(&full, false, false)
            .unwrap(),
        ["-c", "default_permissions=\":danger-full-access\""]
    );
}

#[test]
fn profiles_never_mix_legacy_flags() {
    for mode in [Mode::ReadOnly, Mode::Confined, Mode::FullAccess] {
        let plan = SandboxPlan::new(mode, p("/w"), vec![p("/r")], vec![]);
        for resuming in [false, true] {
            let args = CodexSandboxDialect::Profiles
                .render(&plan, resuming, false)
                .unwrap();
            assert!(
                !args.iter().any(|a| a == "-s"
                    || a.starts_with("sandbox_mode=")
                    || a.starts_with("sandbox_workspace_write.")),
                "{args:?}"
            );
        }
    }
}

#[test]
fn equal_write_and_read_only_paths_keep_read_only() {
    let plan = SandboxPlan::new(
        Mode::Confined,
        p("/w"),
        vec![p("/d/x"), p("/d/y")],
        vec![p("/d/x")],
    );
    assert_eq!(plan.write, vec![p("/d/y")]);
    assert_eq!(plan.read_only, vec![p("/d/x")]);
}

#[test]
fn profile_keys_are_quoted_toml_strings() {
    let plan = SandboxPlan::new(Mode::Confined, p("/w/a \"b\"\\c"), vec![], vec![]);
    let args = CodexSandboxDialect::Profiles
        .render(&plan, false, false)
        .unwrap();
    assert!(
        args[7].contains(r#"{"/w/a \"b\"\\c"="write"}"#),
        "{}",
        args[7]
    );
}

#[test]
fn legacy_renders_todays_flags() {
    let plan = SandboxPlan::new(
        Mode::Confined,
        p("/w"),
        vec![p("/d/git"), p("/d/tmp")],
        vec![],
    );
    assert_eq!(
        CodexSandboxDialect::Legacy
            .render(&plan, false, false)
            .unwrap(),
        [
            "-s",
            "workspace-write",
            "-c",
            "sandbox_workspace_write.network_access=false",
            "-c",
            "sandbox_workspace_write.exclude_tmpdir_env_var=true",
            "-c",
            "sandbox_workspace_write.exclude_slash_tmp=true",
            "-c",
            "sandbox_workspace_write.writable_roots=[\"/d/git\",\"/d/tmp\"]",
        ]
    );
    let resumed = CodexSandboxDialect::Legacy
        .render(&plan, true, false)
        .unwrap();
    assert_eq!(resumed[..2], ["-c", "sandbox_mode=\"workspace-write\""]);
    let resumed_s = CodexSandboxDialect::Legacy
        .render(&plan, true, true)
        .unwrap();
    assert_eq!(resumed_s[..2], ["-s", "workspace-write"]);
}

#[test]
fn legacy_refuses_read_only_paths() {
    assert!(
        CodexSandboxDialect::Legacy
            .render(&confined(), false, false)
            .is_err()
    );
    assert!(!CodexSandboxDialect::Legacy.expresses_read_only());
    assert!(CodexSandboxDialect::Profiles.expresses_read_only());
}
