//! The confinement and profile keys of `[orchestrator]` (M8a final fix batch F1d):
//! absolute `cache_dirs` entries only, `confined_network` keyed by repository root,
//! `unconfined_checks`, and the profile's `generated` and `protected` (moved here in
//! F4). Split out of `orchestrator_tests.rs` to keep that file under the 600-line rule.

use super::*;

#[test]
fn a_relative_cache_dirs_entry_is_refused_at_load() {
    // R3: a relative entry meant "in the checkout", which the run can rewrite.
    let (config, problems) = parse(
        r#"
[orchestrator.cache_dirs]
"/Users/me/work/anthrex" = ["/opt/cache", ".cache/pip"]
"/Users/me/other" = ["~/.cargo/registry"]
"#,
    );
    let problem = problems
        .iter()
        .find(|p| p.key == "orchestrator.cache_dirs.\"/Users/me/work/anthrex\"")
        .unwrap_or_else(|| panic!("{problems:?}"));
    assert!(
        problem.message.contains("not an absolute path"),
        "{problem:?}"
    );
    assert!(problem.message.contains(".cache/pip"), "{problem:?}");
    assert_eq!(
        config.orchestrator.cache_dirs.get("/Users/me/work/anthrex"),
        None
    );
    assert_eq!(
        config.orchestrator.cache_dirs.get("/Users/me/other"),
        Some(&vec!["~/.cargo/registry".to_string()])
    );
}

#[test]
fn confined_network_is_read_keyed_by_repo_root_and_off_by_default() {
    // R4: network for confined checks is the user's per-repository choice.
    let (config, problems) = parse("");
    assert!(problems.is_empty());
    assert!(config.orchestrator.confined_network.is_empty());
    let (config, problems) = parse(
        r#"
[orchestrator.confined_network]
"/Users/me/work/anthrex" = true
"/Users/me/other" = "yes"
"#,
    );
    assert_eq!(
        config
            .orchestrator
            .confined_network
            .get("/Users/me/work/anthrex"),
        Some(&true)
    );
    assert_eq!(
        config.orchestrator.confined_network.get("/Users/me/other"),
        None
    );
    assert!(
        problems
            .iter()
            .any(|p| p.key == "orchestrator.confined_network.\"/Users/me/other\""),
        "{problems:?}"
    );
    // Not a profile key: the repo profile and a plan cannot set it.
    let (_, problems) = parse("[orchestrator.profile]\nconfined_network = true\n");
    assert!(
        problems
            .iter()
            .any(|p| p.key == "orchestrator.profile.confined_network"),
        "{problems:?}"
    );
}

#[test]
fn confined_unix_sockets_and_localhost_ports_are_read_keyed_by_repo_root() {
    // F1d round 2 (S1): the only ways from a confined check to a local service.
    let (config, problems) = parse(
        r#"
[orchestrator.confined_unix_sockets]
"/r/a" = ["/tmp/.s.PGSQL.5432", "~/.colima/default/docker.sock"]
"/r/b" = ["docker.sock"]

[orchestrator.confined_localhost_ports]
"/r/a" = [5432, 6379]
"/r/b" = [0]
"/r/c" = [70000]
"#,
    );
    assert_eq!(
        config.orchestrator.confined_unix_sockets.get("/r/a"),
        Some(&vec![
            "/tmp/.s.PGSQL.5432".to_string(),
            "~/.colima/default/docker.sock".to_string()
        ])
    );
    assert_eq!(config.orchestrator.confined_unix_sockets.get("/r/b"), None);
    assert_eq!(
        config.orchestrator.confined_localhost_ports.get("/r/a"),
        Some(&vec![5432, 6379])
    );
    for key in [
        "orchestrator.confined_unix_sockets.\"/r/b\"",
        "orchestrator.confined_localhost_ports.\"/r/b\"",
        "orchestrator.confined_localhost_ports.\"/r/c\"",
    ] {
        assert!(problems.iter().any(|p| p.key == key), "{key}: {problems:?}");
    }
    assert_eq!(problems.len(), 3, "{problems:?}");
}

#[test]
fn profile_generated_is_read() {
    let (config, problems) = parse(
        r#"
[orchestrator.profile]
generated = ["gen/**", "vendor/**"]
"#,
    );
    assert!(problems.is_empty());
    assert_eq!(
        config.orchestrator.profile.generated,
        Some(vec!["gen/**".to_string(), "vendor/**".to_string()])
    );
}

#[test]
fn profile_protected_is_read_as_additions() {
    // Decision 56: the config crate cannot see `daemon::run::plan::BUILTIN_PROTECTED`,
    // so this reads back exactly what was written; `resolve_profile` (M8a.5) adds the
    // built-ins (Ruling Q12 of the brief refresh, tested there as
    // `plan_protected_adds_to_config_and_builtins`).
    let (config, problems) = parse(
        r#"
[orchestrator.profile]
protected = ["x/**"]
"#,
    );
    assert!(problems.is_empty());
    assert_eq!(
        config.orchestrator.profile.protected,
        Some(vec!["x/**".to_string()])
    );
}

#[test]
fn cache_dirs_are_read_keyed_by_repo_root() {
    // M8a final fix batch F1c round 3 (N3): `[orchestrator.cache_dirs]`, per repository.
    let (config, problems) = parse(
        r#"
[orchestrator.cache_dirs]
"/Users/me/work/anthrex" = ["~/.cache/sccache", "/opt/cache"]
"/Users/me/other" = ["~/.cargo/registry"]
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        config.orchestrator.cache_dirs.get("/Users/me/work/anthrex"),
        Some(&vec![
            "~/.cache/sccache".to_string(),
            "/opt/cache".to_string()
        ])
    );
    assert_eq!(
        config.orchestrator.cache_dirs.get("/Users/me/other"),
        Some(&vec!["~/.cargo/registry".to_string()])
    );
    // It is not a profile key any more.
    let (_, problems) = parse("[orchestrator.profile]\ncache_dirs = [\"x\"]\n");
    assert!(
        problems
            .iter()
            .any(|p| p.key == "orchestrator.profile.cache_dirs"),
        "{problems:?}"
    );
}

#[test]
fn unconfined_checks_is_read_and_off_by_default() {
    // M8a final fix batch F1c round 2.
    let (config, problems) = parse("");
    assert!(problems.is_empty());
    assert!(!config.orchestrator.unconfined_checks);
    let (config, problems) = parse("[orchestrator]\nunconfined_checks = true\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert!(config.orchestrator.unconfined_checks);
}
