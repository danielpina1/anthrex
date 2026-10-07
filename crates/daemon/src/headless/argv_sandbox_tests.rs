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
    let args = codex(&confined_spec(), &new_session(), "m", &legacy);
    assert!(
        args.windows(2).any(|w| w == ["-s", "read-only"]),
        "{args:?}"
    );
    assert!(
        !args.iter().any(|a| a.contains("workspace-write")),
        "{args:?}"
    );
    // Without read-only entries it is today's legacy shape.
    let mut plain = confined_spec();
    plain.codex_read_only.clear();
    let args = codex(&plain, &new_session(), "m", &legacy);
    assert!(
        args.windows(2).any(|w| w == ["-s", "workspace-write"]),
        "{args:?}"
    );
}
