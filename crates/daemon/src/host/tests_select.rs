//! Task M9.2.12: decision 15's host choice, read from the daemon's environment.

use std::collections::HashMap;
use std::path::PathBuf;

use super::*;
use crate::host::HostError;

fn choice(vars: &[(&str, &str)]) -> (CodeHostChoice, Option<String>) {
    let vars: HashMap<&str, &str> = vars.iter().copied().collect();
    CodeHostChoice::from_vars(|key| vars.get(key).map(|v| v.to_string()))
}

#[test]
fn code_host_choice_reads_the_environment() {
    let gh = |bin: &str| CodeHostChoice::Gh {
        bin: PathBuf::from(bin),
    };
    // Unset, or empty: the user's own gh, found on PATH.
    assert_eq!(choice(&[]), (gh("gh"), None));
    assert_eq!(choice(&[("ANTHREX_CODE_HOST", "")]), (gh("gh"), None));
    assert_eq!(CodeHostChoice::default(), gh("gh"));
    // `gh`, with and without ANTHREX_GH_BIN; a fake directory is ignored.
    assert_eq!(choice(&[("ANTHREX_CODE_HOST", "gh")]), (gh("gh"), None));
    assert_eq!(
        choice(&[
            ("ANTHREX_CODE_HOST", "gh"),
            ("ANTHREX_GH_BIN", "/nonexistent/anthrex-test/gh"),
            ("ANTHREX_FAKE_HOST_DIR", "/tmp/ax-fake"),
        ]),
        (gh("/nonexistent/anthrex-test/gh"), None)
    );
    assert_eq!(
        choice(&[("ANTHREX_GH_BIN", "/opt/gh/bin/gh")]),
        (gh("/opt/gh/bin/gh"), None)
    );
    // `fake`, with its directory and without one.
    assert_eq!(
        choice(&[
            ("ANTHREX_CODE_HOST", "fake"),
            ("ANTHREX_FAKE_HOST_DIR", "/tmp/ax-fake"),
            ("ANTHREX_GH_BIN", "/nonexistent/anthrex-test/gh"),
        ]),
        (
            CodeHostChoice::Fake {
                dir: Some(PathBuf::from("/tmp/ax-fake"))
            },
            None
        )
    );
    assert_eq!(
        choice(&[("ANTHREX_CODE_HOST", "fake")]),
        (CodeHostChoice::Fake { dir: None }, None)
    );
    // Anything else: a warning, and gh (never the fake by accident).
    assert_eq!(
        choice(&[
            ("ANTHREX_CODE_HOST", "github"),
            ("ANTHREX_GH_BIN", "/nonexistent/anthrex-test/gh"),
        ]),
        (
            gh("/nonexistent/anthrex-test/gh"),
            Some("ANTHREX_CODE_HOST=github is not gh or fake; using gh".to_string())
        )
    );
    assert_eq!(
        choice(&[("ANTHREX_CODE_HOST", "FAKE")]).0,
        gh("gh"),
        "the value is matched exactly"
    );
}

/// A `fake` host with no directory answers every call with decision 15's text and runs
/// nothing.
#[test]
fn a_fake_host_without_its_directory_fails_every_call() {
    let host = build(&CodeHostChoice::Fake { dir: None });
    let tmp = tempfile::tempdir().unwrap();
    let repo = HostRepo {
        host: "github.com".into(),
        owner: "fake".into(),
        name: "app".into(),
        remote: "origin".into(),
        root: tmp.path().to_path_buf(),
    };
    let missing = Err::<(), _>(HostError::Missing(FAKE_NEEDS_DIR.to_string()));
    assert_eq!(host.retarget(&repo, 1, "main"), missing);
    assert_eq!(
        host.detect(tmp.path(), "origin").map(|_| ()),
        missing,
        "detection proposes nothing"
    );
    assert_eq!(
        host.view_pr(&repo, 1).map(|_| ()),
        missing,
        "nothing is read"
    );
}
