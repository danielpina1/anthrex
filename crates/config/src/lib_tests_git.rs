//! The `[git]` table's tests, moved out of `lib_tests.rs` (600-line rule).

use super::*;

#[test]
fn the_git_table_is_read() {
    let (config, problems) = parse(
        r#"
[git]
enabled = false
poll_secs = 5
debounce_ms = 50
ignore = ["vendor", "Pods"]
"#,
    );
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    assert!(!config.git.enabled);
    assert_eq!(config.git.poll_secs, 5);
    assert_eq!(config.git.debounce_ms, 50);
    assert_eq!(config.git.ignore, vec!["vendor", "Pods"]);
}

#[test]
fn out_of_range_git_values_keep_their_defaults() {
    // Both ends of both ranges, plus the wrong type, in one file: every one of them
    // keeps the default rather than failing the parse (decision 3).
    let (config, problems) = parse(
        r#"
[git]
poll_secs = 4
debounce_ms = 5001
enabled = "yes"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from([
            "git.poll_secs".to_string(),
            "git.debounce_ms".to_string(),
            "git.enabled".to_string(),
        ])
    );
    assert_eq!(config.git, Git::default());

    // The in-range boundaries are accepted, so the ranges are inclusive as documented.
    let (config, problems) = parse("[git]\npoll_secs = 3600\ndebounce_ms = 5000\n");
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    assert_eq!(config.git.poll_secs, 3600);
    assert_eq!(config.git.debounce_ms, 5000);
}

#[test]
fn bad_ignore_entries_are_dropped_and_the_rest_survive() {
    let long = "x".repeat(65);
    let text = format!(
        "[git]\nignore = [\"good\", \"\", \"with/slash\", \"..\", \".\", \"{long}\", 7, \"also-good\"]\n"
    );
    let (config, problems) = parse(&text);
    assert_eq!(
        config.git.ignore,
        vec!["good", "also-good"],
        "one bad entry must not cost the good ones"
    );
    assert_eq!(problems.len(), 6, "one problem per rejected entry");
    assert!(problems.iter().all(|p| p.key == "git.ignore"));
    // Task 7 re-review: a dropped entry prints `(entry skipped)`, as decision 9a's do,
    // never `(using entry dropped)`.
    let shown: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert!(
        shown.contains(&"git.ignore: expected a string (entry skipped)".to_string()),
        "{shown:?}"
    );
    assert!(
        shown.iter().all(|p| p.ends_with(" (entry skipped)")),
        "{shown:?}"
    );
}

#[test]
fn an_over_long_ignore_list_is_truncated_with_one_problem() {
    let entries: Vec<String> = (0..33).map(|n| format!("\"d{n}\"")).collect();
    let (config, problems) = parse(&format!("[git]\nignore = [{}]\n", entries.join(", ")));
    assert_eq!(config.git.ignore.len(), 32);
    assert_eq!(config.git.ignore[31], "d31");
    assert_eq!(keys(&problems), HashSet::from(["git.ignore".to_string()]));
}

#[test]
fn a_git_table_of_the_wrong_type_keeps_every_default() {
    let (config, problems) = parse("git = 3\n");
    assert_eq!(keys(&problems), HashSet::from(["git".to_string()]));
    assert_eq!(config.git, Git::default());
}

#[test]
fn an_unknown_git_key_is_reported() {
    let (config, problems) = parse("[git]\nenabled = true\nrecursive = false\n");
    assert_eq!(
        keys(&problems),
        HashSet::from(["git.recursive".to_string()])
    );
    assert!(config.git.enabled);
}
