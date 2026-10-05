//! Task M9.6.4: the gate's change summary (DF §6.1) and decision 36's line diff (carry
//! M-4), both pure.

use super::{DIFF_CAP, line_diff, summary};

const V1: &str = "\
# Password reset

## Goal and success criteria
Users reset their password.

## Requirements
R1 Tokens expire after an hour.
R2 Links are single use.
R3 Errors are logged.

## Testing
Unit tests.
Integration tests.

## Risks
Mail delays.

## Open questions
";

const V2: &str = "\
# Password reset

## Goal and success criteria
Users reset their password.

## Requirements
R1 Tokens expire after an hour.
R2 Links are single use, even across devices.
R4 Resets are rate limited.
R5 Admins can revoke links.

## Testing
Unit tests.
Load tests.

## Interfaces
`reset(token)`.

## Open questions
";

#[test]
fn changes_summarise_added_removed_and_changed_sections_and_requirements() {
    assert_eq!(
        summary(V1, V2),
        vec![
            "+ R4, R5".to_string(),
            "~ R2".to_string(),
            "- R3".to_string(),
            "~ Requirements: 5 lines".to_string(),
            "~ Testing: 2 lines".to_string(),
            "+ Interfaces".to_string(),
            "- Risks".to_string(),
        ]
    );
    assert_eq!(summary(V1, V1), Vec::<String>::new());
    // The title before the first section is compared too; one line is singular.
    let retitled = V1.replace("# Password reset", "# Password resets");
    assert_eq!(summary(V1, &retitled), vec!["~ Title: 2 lines".to_string()]);
    let one = V1.replace("## Open questions\n", "## Open questions\nNone?\n");
    assert_eq!(
        summary(V1, &one),
        vec!["~ Open questions: 1 line".to_string()]
    );
}

#[test]
fn line_diff_is_unified_with_hunk_headers() {
    let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\n";
    let new = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\n";
    assert_eq!(
        line_diff(old, new),
        "\
@@ -1,5 +1,5 @@
 a
-b
+B
 c
 d
 e
@@ -11,3 +11,4 @@
 k
 l
 m
+n
"
    );
    assert_eq!(line_diff(old, old), "");
    // Changes within six lines of each other share a hunk.
    let near = "a\nB\nc\nd\ne\nf\ng\nH\ni\nj\nk\nl\nm\n";
    assert_eq!(
        line_diff(old, near),
        "\
@@ -1,11 +1,11 @@
 a
-b
+B
 c
 d
 e
 f
 g
-h
+H
 i
 j
 k
"
    );
    // From and to nothing: a count of 0 starts at the line before.
    assert_eq!(line_diff("", "x\ny\n"), "@@ -0,0 +1,2 @@\n+x\n+y\n");
    assert_eq!(line_diff("x\ny\n", ""), "@@ -1,2 +0,0 @@\n-x\n-y\n");
}

#[test]
fn line_diff_keeps_the_longest_common_lines() {
    // A moved block: the diff keeps the longer common run.
    let old = "1\n2\n3\n4\n5\nX\n";
    let new = "X\n1\n2\n3\n4\n5\n";
    assert_eq!(
        line_diff(old, new),
        "@@ -1,6 +1,6 @@\n+X\n 1\n 2\n 3\n 4\n 5\n-X\n"
    );
}

#[test]
fn line_diff_refuses_an_input_over_its_cap() {
    assert_eq!(DIFF_CAP, 64 * 1024);
    let at = "x\n".repeat(DIFF_CAP / 2);
    assert_eq!(at.len(), DIFF_CAP);
    assert!(line_diff(&at, "y\n").starts_with("@@ "));
    let over = format!("{at}z");
    let lines = over.lines().count() + 1;
    assert_eq!(
        line_diff(&over, "y\n"),
        format!("diff too large ({lines} lines)")
    );
    assert_eq!(
        line_diff("y\n", &over),
        format!("diff too large ({lines} lines)")
    );
}

#[test]
fn a_large_diff_stays_correct_when_it_falls_back() {
    // Two 64 KiB inputs with no line in common (past the LCS table's bound): every old
    // line is removed and every new one added, and the hunk counts add up.
    let old: String = (0..7000).map(|i| format!("old {i}\n")).collect();
    let new: String = (0..7000).map(|i| format!("new {i}\n")).collect();
    assert!(old.len() <= DIFF_CAP && new.len() <= DIFF_CAP);
    let diff = line_diff(&old, &new);
    assert!(
        diff.starts_with("@@ -1,7000 +1,7000 @@\n-old 0\n"),
        "{}",
        &diff[..40]
    );
    assert_eq!(diff.lines().filter(|l| l.starts_with('-')).count(), 7000);
    assert_eq!(diff.lines().filter(|l| l.starts_with('+')).count(), 7000);
}

/// Review m6: a missing final newline is no change.
#[test]
fn a_final_newline_alone_is_no_change() {
    assert_eq!(line_diff("a", "a\n"), "");
    assert_eq!(line_diff("a\n", "a"), "");
}
