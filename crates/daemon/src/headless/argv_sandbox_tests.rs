//! A headless Codex session's sandbox flags come from the plan (`codex_sandbox`).

use super::*;
use crate::headless::codex_sandbox::{CodexSandboxDialect, DialectChoice};

fn profiles() -> CliCaps {
    CliCaps {
        codex_sandbox: DialectChoice::Fixed(CodexSandboxDialect::Profiles),
        ..CLI_CAPS
    }
}

fn legacy() -> CliCaps {
    CliCaps {
        codex_sandbox: DialectChoice::Fixed(CodexSandboxDialect::Legacy),
        ..CLI_CAPS
    }
}

fn confined_spec() -> HeadlessSpec {
    let mut spec = worker(Runtime::Codex);
    spec.codex_sandbox = "workspace-write".into();
    spec.codex_writable_roots = vec!["/d/git".into(), "/d/tmp".into()];
    spec.codex_read_only = vec!["/d/git/config".into()];
    spec.codex_grant_dialect = Some(CodexSandboxDialect::Profiles);
    spec
}

#[test]
fn a_worker_gets_its_profile_with_read_only_entries() {
    let args = codex(&confined_spec(), &new_session(), "m", &profiles());
    let fs = args
        .iter()
        .find(|a| a.starts_with("permissions.anthrex.filesystem="))
        .unwrap();
    assert!(fs.contains("\"/d/git\"=\"write\""), "{fs}");
    assert!(fs.contains("\"/d/git/config\"=\"read\""), "{fs}");
    assert!(args.iter().any(|a| a == "default_permissions=\"anthrex\""));
    assert!(
        !args.iter().any(|a| a == "-s" || a.starts_with("sandbox_")),
        "{args:?}"
    );
}

#[test]
fn resume_carries_the_same_profile() {
    let spec = confined_spec();
    let first = codex(&spec, &new_session(), "m", &profiles());
    let resume = codex(
        &spec,
        &SessionArg::Resume {
            session_id: "s".into(),
        },
        "m",
        &profiles(),
    );
    let pick = |a: &[String]| {
        a.iter()
            .filter(|x| x.starts_with("default_permissions=") || x.starts_with("permissions."))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(pick(&first), pick(&resume));
    assert!(!pick(&first).is_empty());
}

#[test]
fn a_read_only_role_gets_the_builtin_read_only_profile() {
    let spec = reviewer(Runtime::Codex);
    assert_eq!(spec.codex_sandbox, "read-only");
    let args = codex(&spec, &new_session(), "m", &profiles());
    assert!(
        args.iter()
            .any(|a| a == "default_permissions=\":read-only\""),
        "{args:?}"
    );
}

#[test]
fn the_legacy_dialect_falls_back_to_read_only_for_entries_it_cannot_express() {
    let legacy = legacy();
    let mut spec = confined_spec();
    spec.codex_grant_dialect = Some(CodexSandboxDialect::Legacy);
    let args = codex(&spec, &new_session(), "m", &legacy);
    assert!(
        args.windows(2).any(|w| w == ["-s", "read-only"]),
        "{args:?}"
    );
    assert!(
        !args.iter().any(|a| a.contains("workspace-write")),
        "{args:?}"
    );
    // Without read-only entries it is today's legacy shape.
    let mut plain = spec;
    plain.codex_read_only.clear();
    let args = codex(&plain, &new_session(), "m", &legacy);
    assert!(
        args.windows(2).any(|w| w == ["-s", "workspace-write"]),
        "{args:?}"
    );
}

/// Final review I2 (ruling R7): a confined spec whose grant names no dialect (persisted
/// by a daemon before profiles, or granted under Legacy) renders Legacy flags, whose
/// `workspace-write` keeps `.git` and `.codex` read-only by itself, even when the CLI now
/// speaks profiles: a profile would silently drop that protection.
#[test]
fn a_grant_without_a_dialect_renders_legacy_even_on_profiles() {
    let mut spec = confined_spec();
    spec.codex_read_only.clear();
    spec.codex_grant_dialect = None;
    for session in [
        new_session(),
        SessionArg::Resume {
            session_id: "s".into(),
        },
    ] {
        let args = codex(&spec, &session, "m", &profiles());
        assert!(
            args.iter().any(|a| a.contains("workspace-write")),
            "{args:?}"
        );
        assert!(
            args.iter()
                .any(|a| a == "sandbox_workspace_write.writable_roots=[\"/d/git\",\"/d/tmp\"]"),
            "{args:?}"
        );
        assert!(
            !args
                .iter()
                .any(|a| a.starts_with("default_permissions=") || a.starts_with("permissions.")),
            "{args:?}"
        );
    }
}

/// Ruling R7: a grant computed under profiles renders its profile even when the
/// current caps say Legacy (the grant and its argv never disagree).
#[test]
fn a_grant_renders_in_its_own_dialect_not_the_current_one() {
    let args = codex(&confined_spec(), &new_session(), "m", &legacy());
    assert!(
        args.iter().any(|a| a == "default_permissions=\"anthrex\""),
        "{args:?}"
    );
    assert!(
        args.iter()
            .any(|a| a.starts_with("permissions.anthrex.filesystem=")
                && a.contains("\"/d/git/config\"=\"read\"")),
        "{args:?}"
    );
    assert!(!args.iter().any(|a| a == "-s"), "{args:?}");
}

/// A spec persisted before `codex_grant_dialect` existed loads with none (Legacy).
#[test]
fn a_persisted_spec_without_a_grant_dialect_loads_as_none() {
    let mut spec = confined_spec();
    spec.codex_grant_dialect = None;
    let mut json = serde_json::to_value(&spec).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("codex_grant_dialect")
        .expect("the field is written");
    assert_eq!(serde_json::from_value::<HeadlessSpec>(json).unwrap(), spec);
    let granted = confined_spec();
    let json = serde_json::to_value(&granted).unwrap();
    assert_eq!(json["codex_grant_dialect"], "Profiles");
    assert_eq!(
        serde_json::from_value::<HeadlessSpec>(json).unwrap(),
        granted
    );
}
