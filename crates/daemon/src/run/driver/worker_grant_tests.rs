//! Task 5 of the Codex sandbox profiles plan: a worker's grant shape by runtime and
//! dialect, and (controller ruling R3) a Codex worker's read-only entries.

use std::path::{Path, PathBuf};

use super::{codex_read_only, grant_shape, grant_shape_on};
use crate::headless::codex_sandbox::CodexSandboxDialect;
use crate::run::git;

/// Final review M3: checked against a whole-directory host on every host (on macOS the
/// host's own shape is `Files`, which could not tell the runtimes apart).
#[test]
fn a_codex_worker_on_profiles_gets_the_host_shape() {
    use git::GrantShape::{Files, WholeDir};
    assert_eq!(
        grant_shape_on(false, CodexSandboxDialect::Profiles, WholeDir),
        WholeDir
    );
    assert_eq!(
        grant_shape_on(true, CodexSandboxDialect::Legacy, WholeDir),
        WholeDir
    );
    assert_eq!(
        grant_shape(false, CodexSandboxDialect::Profiles),
        grant_shape(true, CodexSandboxDialect::Profiles)
    );
    assert_eq!(
        grant_shape(false, CodexSandboxDialect::Profiles),
        git::GrantShape::host()
    );
    if cfg!(target_os = "linux") {
        assert_eq!(git::GrantShape::host(), WholeDir);
    }
    assert_eq!(
        grant_shape_on(true, CodexSandboxDialect::Profiles, Files),
        Files
    );
}

#[test]
fn legacy_dialect_keeps_the_files_grant() {
    assert_eq!(
        grant_shape_on(
            false,
            CodexSandboxDialect::Legacy,
            git::GrantShape::WholeDir
        ),
        git::GrantShape::Files
    );
    assert_eq!(
        grant_shape(false, CodexSandboxDialect::Legacy),
        git::GrantShape::Files
    );
}

const CWD: &str = "/w/t1";

fn p(tail: &str) -> PathBuf {
    Path::new(CWD).join(tail)
}

/// Every candidate exists.
fn all(_: &Path) -> bool {
    true
}

#[test]
fn the_protected_paths_and_git_come_first_then_the_grants_denials() {
    let deny = vec![PathBuf::from("/d/git/config"), PathBuf::from("/d/git/refs")];
    let got = codex_read_only(
        CodexSandboxDialect::Profiles,
        Path::new(CWD),
        &["a.txt".to_string()],
        deny,
        all,
    );
    assert_eq!(
        got,
        [
            p(".claude"),
            p(".codex"),
            p(".mcp.json"),
            p("CLAUDE.md"),
            p("AGENTS.md"),
            p(".git"),
            PathBuf::from("/d/git/config"),
            PathBuf::from("/d/git/refs"),
        ]
    );
}

#[test]
fn a_missing_protected_path_or_git_is_left_out() {
    let present = [p(".codex"), p("AGENTS.md")];
    let got = codex_read_only(
        CodexSandboxDialect::Profiles,
        Path::new(CWD),
        &[],
        vec![PathBuf::from("/d/git/config")],
        |path| present.iter().any(|x| x == path),
    );
    // The grant's denials are made to exist by the grant itself (`placeholders`), so
    // they are never filtered.
    assert_eq!(
        got,
        [p(".codex"), p("AGENTS.md"), PathBuf::from("/d/git/config")]
    );
}

#[test]
fn an_owned_literal_is_not_read_only() {
    let got = codex_read_only(
        CodexSandboxDialect::Profiles,
        Path::new(CWD),
        &["AGENTS.md".to_string(), ".codex/config.toml".to_string()],
        Vec::new(),
        all,
    );
    assert!(!got.contains(&p("AGENTS.md")), "{got:?}");
    assert!(!got.contains(&p(".codex")), "{got:?}");
    assert!(got.contains(&p(".git")), "{got:?}");
    assert!(got.contains(&p("CLAUDE.md")), "{got:?}");
}

#[test]
fn duplicates_are_kept_once_in_first_place() {
    let got = codex_read_only(
        CodexSandboxDialect::Profiles,
        Path::new(CWD),
        &[],
        vec![p(".git"), PathBuf::from("/d/x"), PathBuf::from("/d/x")],
        |path| path == p(".git"),
    );
    assert_eq!(got, [p(".git"), PathBuf::from("/d/x")]);
}

/// Legacy cannot express a read-only path inside a writable one (it would render the
/// whole session read-only), so nothing is filled.
#[test]
fn the_legacy_dialect_gets_no_read_only_entries() {
    let got = codex_read_only(
        CodexSandboxDialect::Legacy,
        Path::new(CWD),
        &[],
        vec![PathBuf::from("/d/git/config")],
        all,
    );
    assert!(got.is_empty(), "{got:?}");
}
