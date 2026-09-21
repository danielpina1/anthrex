use super::*;
use std::collections::HashSet;

fn keys(problems: &[Problem]) -> HashSet<String> {
    problems.iter().map(|p| p.key.clone()).collect()
}

#[test]
fn empty_text_gives_defaults() {
    let (config, problems) = parse("");
    assert!(problems.is_empty());
    assert_eq!(config, Config::default());

    assert_eq!(config.prefix, Prefix('b'));
    assert_eq!(config.prefix.label(), "C-b");
    assert_eq!(config.accent, Rgb(0x89, 0xb4, 0xfa));
    assert!(config.bell.attention);
    assert!(!config.bell.done);
    assert_eq!(config.default_runtime, proto::Runtime::Shell);
    assert_eq!(config.scrollback_lines, 5000);
    assert_eq!(config.ui.sidebar_width, None);
    assert_eq!(config.ui.tree_keep_finished_secs, 300);
    assert_eq!(config.panes.max, 6);
    assert_eq!(config.runtimes.claude.command, "claude");
    assert_eq!(config.runtimes.codex.command, "codex");
    assert!(!config.runtimes.codex_bypass_hook_trust);
}

#[test]
fn every_key_is_read() {
    let home = std::env::var("HOME").expect("HOME must be set to run this test");
    let text = r##"
prefix = "C-a"
accent = "#f5c2e7"
default_runtime = "claude"
scrollback_lines = 10000

[bell]
attention = true
done = true

[ui]
sidebar_width = 40
tree_keep_finished_secs = 120

[panes]
max = 4

[runtimes.claude]
command = "~/bin/claude"

[runtimes.codex]
command = "/opt/homebrew/bin/codex"
bypass_hook_trust = true

[orchestrator]
max_parallel = 3
"##;
    let (config, problems) = parse(text);
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");

    assert_eq!(config.prefix, Prefix('a'));
    assert_eq!(config.accent, Rgb(0xf5, 0xc2, 0xe7));
    assert_eq!(config.default_runtime, proto::Runtime::Claude);
    assert_eq!(config.scrollback_lines, 10000);
    assert!(config.bell.attention);
    assert!(config.bell.done);
    assert_eq!(config.ui.sidebar_width, Some(40));
    assert_eq!(config.ui.tree_keep_finished_secs, 120);
    assert_eq!(config.panes.max, 4);
    assert_eq!(config.runtimes.claude.command, format!("{home}/bin/claude"));
    assert_eq!(config.runtimes.codex.command, "/opt/homebrew/bin/codex");
    assert!(config.runtimes.codex_bypass_hook_trust);
}

#[test]
fn bad_values_keep_their_defaults() {
    let text = r#"
prefix = "C-m"
accent = "blue"
scrollback_lines = -1
default_runtime = "perl"

[ui]
sidebar_width = 10

[panes]
max = 17

[bell]
done = "yes"

[runtimes.codex]
bypass_hook_trust = "yes"
command = ""
"#;
    let (config, problems) = parse(text);
    assert_eq!(config, Config::default());

    let expected: HashSet<String> = [
        "prefix",
        "accent",
        "scrollback_lines",
        "default_runtime",
        "ui.sidebar_width",
        "panes.max",
        "bell.done",
        "runtimes.codex.bypass_hook_trust",
        "runtimes.codex.command",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let actual = keys(&problems);
    assert_eq!(actual, expected);
    assert_eq!(
        problems.len(),
        expected.len(),
        "exactly one problem per key"
    );
}

#[test]
fn syntax_error_is_one_problem() {
    let (config, problems) = parse("prefix = ");
    assert_eq!(config, Config::default());
    assert_eq!(problems.len(), 1);
}

#[test]
fn unknown_keys_are_reported_and_ignored() {
    let text = r#"
colour = "red"

[bell]
loud = true
"#;
    let (config, problems) = parse(text);
    assert_eq!(config, Config::default());
    assert_eq!(problems.len(), 2);
    for problem in &problems {
        assert_eq!(problem.message, "unknown key, ignored");
    }
    assert_eq!(
        keys(&problems),
        ["colour", "bell.loud"]
            .into_iter()
            .map(String::from)
            .collect()
    );
}

#[test]
fn old_sidebar_width_is_an_alias() {
    let (config, problems) = parse("sidebar_width = 40");
    assert_eq!(config.ui.sidebar_width, Some(40));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].key, "sidebar_width");
    assert_eq!(problems[0].message, "renamed to ui.sidebar_width");

    // When both are present, `ui.sidebar_width` wins.
    let text = "sidebar_width = 20\n\n[ui]\nsidebar_width = 50\n";
    let (config, _problems) = parse(text);
    assert_eq!(config.ui.sidebar_width, Some(50));
}

#[test]
fn problem_display() {
    let p = Problem {
        key: "prefix".to_string(),
        message: "expected C- followed by a letter".to_string(),
        default: "C-b".to_string(),
    };
    assert_eq!(
        p.to_string(),
        "prefix: expected C- followed by a letter (using C-b)"
    );
}

#[test]
fn missing_file_is_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("does-not-exist").join("config.toml");
    let (config, problems) = load(&path);
    assert_eq!(config, Config::default());
    assert!(problems.is_empty());
}

/// Decision 4 excludes `h`, `i`, `j` and `m` specifically, because terminals
/// send those as Backspace, Tab and Enter -- not just any letter, and not
/// only `m`. `bad_values_keep_their_defaults` only exercises `C-m`; this
/// covers the other three so a narrower exclusion list doesn't slip through.
#[test]
fn prefix_excludes_reserved_letters() {
    for letter in ['h', 'i', 'j', 'm'] {
        let text = format!("prefix = \"C-{letter}\"");
        let (config, problems) = parse(&text);
        assert_eq!(
            config.prefix,
            Prefix('b'),
            "C-{letter} should have been rejected"
        );
        assert_eq!(
            problems.len(),
            1,
            "C-{letter} should report exactly one problem"
        );
        assert_eq!(problems[0].key, "prefix");
    }
    // A letter outside the exclusion list is accepted.
    let (config, problems) = parse("prefix = \"C-x\"");
    assert_eq!(config.prefix, Prefix('x'));
    assert!(problems.is_empty());
}

/// Exercises both ends of every numeric range from decision 4: the boundary
/// values themselves are valid, and one step past each boundary is not. A
/// mutation that shifts a range by one, or flips `<`/`<=`, fails here even
/// though `bad_values_keep_their_defaults` only tries values well outside
/// the range.
#[test]
fn range_boundaries_are_inclusive() {
    let valid = r#"
scrollback_lines = 0

[ui]
sidebar_width = 24
tree_keep_finished_secs = 0

[panes]
max = 1
"#;
    let (config, problems) = parse(valid);
    assert!(
        problems.is_empty(),
        "boundary-low values should be valid: {problems:?}"
    );
    assert_eq!(config.scrollback_lines, 0);
    assert_eq!(config.ui.sidebar_width, Some(24));
    assert_eq!(config.ui.tree_keep_finished_secs, 0);
    assert_eq!(config.panes.max, 1);

    let valid_high = r#"
scrollback_lines = 100000

[ui]
sidebar_width = 60
tree_keep_finished_secs = 300

[panes]
max = 16
"#;
    let (config, problems) = parse(valid_high);
    assert!(
        problems.is_empty(),
        "boundary-high values should be valid: {problems:?}"
    );
    assert_eq!(config.scrollback_lines, 100000);
    assert_eq!(config.ui.sidebar_width, Some(60));
    assert_eq!(config.ui.tree_keep_finished_secs, 300);
    assert_eq!(config.panes.max, 16);

    let invalid = r#"
scrollback_lines = 100001

[ui]
sidebar_width = 61
tree_keep_finished_secs = 301

[panes]
max = 0
"#;
    let (config, problems) = parse(invalid);
    assert_eq!(config, Config::default());
    assert_eq!(
        keys(&problems),
        [
            "scrollback_lines",
            "ui.sidebar_width",
            "ui.tree_keep_finished_secs",
            "panes.max"
        ]
        .into_iter()
        .map(String::from)
        .collect()
    );
}
