//! Decision 24's slug and paths (task M9.6.12). Pure.

use super::{repo_path, slug};

/// The slug: the spec's `# ` title, lowercase ASCII alphanumerics and `-`, runs
/// collapsed, at most 48 characters; empty, the run id.
#[test]
fn the_slug_comes_from_the_spec_title() {
    let cases = [
        ("# Password reset\n\nbody", "password-reset"),
        (
            "intro\n# Fix: the UI's `run show` -- v2!!\n",
            "fix-the-ui-s-run-show-v2",
        ),
        ("#  Ünïcode   Café ünd Tea\n", "n-code-caf-nd-tea"),
        ("## Not a title\n# Real\n", "real"),
        ("# ???\n", "run-7"),
        ("no title at all\n", "run-7"),
        ("#Title without a space\n", "run-7"),
    ];
    for (text, want) in cases {
        assert_eq!(slug(text, "run-7"), want, "{text:?}");
    }
    let long = format!("# {}\n", "word ".repeat(30));
    let cut = slug(&long, "run-7");
    assert!(cut.len() <= 48, "{cut}");
    assert!(!cut.ends_with('-'), "{cut}");
    assert_eq!(cut, "word-word-word-word-word-word-word-word-word-wor");
}

/// `<docs_dir>/<folder>/<YYYY-MM-DD>-<slug>.md`.
#[test]
fn a_document_path_is_folder_date_and_slug() {
    assert_eq!(
        repo_path("docs/anthrex", "specs", "2026-10-05", "password-reset"),
        "docs/anthrex/specs/2026-10-05-password-reset.md"
    );
}
