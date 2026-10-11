use super::*;

/// Milestone 9.12 decision 12: the client's hand copy of the branch rules goes, and with
/// it the test that held the two copies together (`tui/src/dialog_tests.rs`'s
/// `check_branch_syntax_agrees_with_the_daemons_copy`). This holds what that test held:
/// its eleven inputs, each with the exact answer the daemon's copy gave on 2026-10-11, so
/// a changed rule or message fails here instead of reaching the client silently.
#[test]
fn the_four_rules_answer_as_the_daemon_did() {
    const REQUIRED: &str = "branch name is required";
    const SPACES: &str = "branch name cannot contain spaces";
    const DASH: &str = "branch name cannot start with '-'";
    const RESERVED: &str = "branches under anthrex/ are reserved for orchestration runs";
    let cases: [(&str, Result<(), &str>); 11] = [
        ("", Err(REQUIRED)),
        ("   ", Err(REQUIRED)),
        ("feat/api", Ok(())),
        ("  feat/api  ", Err(SPACES)),
        ("feat api", Err(SPACES)),
        ("feat\tapi", Err(SPACES)),
        ("-feat", Err(DASH)),
        ("anthrex/", Err(RESERVED)),
        ("anthrex/run-1", Err(RESERVED)),
        ("anthrexanthrex/not-actually-reserved", Ok(())),
        ("feat/anthrex/nested", Ok(())),
    ];
    for (branch, expected) in cases {
        assert_eq!(
            check_branch_syntax(branch),
            expected.map_err(str::to_string),
            "check_branch_syntax({branch:?})"
        );
    }
}
