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

    // Git-surface spec 3.7: the `[git]` table's defaults are that spec's own numbers —
    // the 30-second safety poll and the 300 ms debounce of its section 3.3 — and the
    // subsystem is on unless something turns it off.
    assert!(config.git.enabled);
    assert_eq!(config.git.poll_secs, 30);
    assert_eq!(config.git.debounce_ms, 300);
    assert!(config.git.ignore.is_empty());

    // Conversation-view spec decision 7 / A5: `[conversation]` and
    // `[conversation.badges]` default to the spec's own numbers and glyphs.
    assert_eq!(config.conversation, Conversation::default());
    assert_eq!(config.conversation.max_turns, 500);
    assert_eq!(config.conversation.max_bytes, 2_097_152);
    assert_eq!(config.conversation.max_result_bytes, 16384);
    assert_eq!(config.conversation.linger_secs, 30);
    assert!(!config.conversation.badges.force_ascii);
    assert_eq!(config.conversation.badges.claude.glyph, "\u{25c6}");
    assert_eq!(config.conversation.badges.claude.ascii, "[C]");
    assert_eq!(
        config.conversation.badges.claude.color,
        Rgb(0xd7, 0x9b, 0x61)
    );
    assert_eq!(config.conversation.badges.codex.glyph, "\u{25c7}");
    assert_eq!(config.conversation.badges.codex.ascii, "[X]");
    assert_eq!(
        config.conversation.badges.codex.color,
        Rgb(0x7f, 0xc8, 0xb4)
    );
    assert_eq!(config.conversation.badges.shell.glyph, "$");
    assert_eq!(config.conversation.badges.shell.ascii, "[$]");
    assert_eq!(
        config.conversation.badges.shell.color,
        Rgb(0x9a, 0xa0, 0xb5)
    );
}

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

    // When both are present, `ui.sidebar_width` wins, and the alias's own
    // "renamed" problem is still reported exactly once -- not duplicated,
    // and not dropped because it was overridden. The alias value (30) is
    // itself valid so it actually reaches the "renamed" problem, rather
    // than being rejected for being out of range and masking the
    // precedence behaviour this is meant to pin down.
    let text = "sidebar_width = 30\n\n[ui]\nsidebar_width = 50\n";
    let (config, problems) = parse(text);
    assert_eq!(config.ui.sidebar_width, Some(50));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].key, "sidebar_width");
    assert_eq!(problems[0].message, "renamed to ui.sidebar_width");
}

/// `every_key_is_read` sets `attention = true, done = true`, the brief's own
/// pinned fixture, which cannot tell the two `[bell]` fields apart: swapping
/// which field each key writes into is invisible when both end up `true`.
/// This uses different values per field so a swap fails here instead.
#[test]
fn bell_fields_are_independent() {
    let (config, problems) = parse("[bell]\nattention = false\ndone = true\n");
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    assert!(!config.bell.attention);
    assert!(config.bell.done);
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

/// Unlike a missing file, a path that exists but can't be read as config
/// text must not fold into the same silent "defaults, no problems" result --
/// the caller needs a way to tell "no config" from "config I could not read".
#[test]
fn directory_path_reports_a_problem() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::create_dir(&path).unwrap();

    let (config, problems) = load(&path);
    assert_eq!(config, Config::default());
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].key, "<config>");
    assert!(
        problems[0].message.contains(&path.display().to_string()),
        "message should name the path: {:?}",
        problems[0].message
    );
}

#[test]
fn unreadable_file_reports_a_problem() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "prefix = \"C-a\"").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

    // Root ignores the mode, so confirm the denial is real before asserting on it.
    let denied = matches!(
        std::fs::read_to_string(&path),
        Err(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied
    );
    let result = load(&path);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    if !denied {
        eprintln!("skipped: the file mode did not deny access (running as root?)");
        return;
    }
    let (config, problems) = result;
    assert_eq!(config, Config::default());
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].key, "<config>");
    assert!(
        problems[0].message.contains(&path.display().to_string()),
        "message should name the path: {:?}",
        problems[0].message
    );
}

#[test]
fn non_utf8_file_reports_a_problem() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();

    let (config, problems) = load(&path);
    assert_eq!(config, Config::default());
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].key, "<config>");
    assert!(
        problems[0].message.contains(&path.display().to_string()),
        "message should name the path: {:?}",
        problems[0].message
    );
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

#[test]
fn the_conversation_table_is_read() {
    let (config, problems) = parse(
        r##"
[conversation]
max_turns = 40
max_bytes = 131072
max_result_bytes = 8192
linger_secs = 7

[conversation.badges]
force_ascii = true

[conversation.badges.claude]
glyph = "▲"
ascii = "(C)"
color = "#112233"

[conversation.badges.codex]
glyph = "▼"
ascii = "(X)"
color = "#445566"

[conversation.badges.shell]
glyph = "#"
ascii = "(S)"
color = "#778899"
"##,
    );
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    assert_eq!(config.conversation.max_turns, 40);
    assert_eq!(config.conversation.max_bytes, 131072);
    assert_eq!(config.conversation.max_result_bytes, 8192);
    assert_eq!(config.conversation.linger_secs, 7);
    assert!(config.conversation.badges.force_ascii);
    assert_eq!(config.conversation.badges.claude.glyph, "\u{25b2}");
    assert_eq!(config.conversation.badges.claude.ascii, "(C)");
    assert_eq!(
        config.conversation.badges.claude.color,
        Rgb(0x11, 0x22, 0x33)
    );
    assert_eq!(config.conversation.badges.codex.glyph, "\u{25bc}");
    assert_eq!(config.conversation.badges.codex.ascii, "(X)");
    assert_eq!(
        config.conversation.badges.codex.color,
        Rgb(0x44, 0x55, 0x66)
    );
    assert_eq!(config.conversation.badges.shell.glyph, "#");
    assert_eq!(config.conversation.badges.shell.ascii, "(S)");
    assert_eq!(
        config.conversation.badges.shell.color,
        Rgb(0x77, 0x88, 0x99)
    );
}

#[test]
fn out_of_range_conversation_values_keep_their_defaults() {
    let (config, problems) = parse(
        r#"
[conversation]
max_turns = 0
max_bytes = 1024
max_result_bytes = 2000000
linger_secs = 301
"#,
    );
    assert_eq!(config.conversation, Conversation::default());
    assert_eq!(
        keys(&problems),
        HashSet::from([
            "conversation.max_turns".to_string(),
            "conversation.max_bytes".to_string(),
            "conversation.max_result_bytes".to_string(),
            "conversation.linger_secs".to_string(),
        ])
    );
}

/// Each bad key lives on a *different* badge (force_ascii, claude.glyph, codex.ascii,
/// shell.color), so a `read_badge` that wrote into the wrong runtime's struct -- or a
/// `report_unknown_conversation` that conflated the three -- fails this test.
#[test]
fn bad_badge_values_keep_their_defaults() {
    let (config, problems) = parse(
        r#"
[conversation.badges]
force_ascii = "yes"

[conversation.badges.claude]
glyph = "abc"

[conversation.badges.codex]
ascii = "◇◇"

[conversation.badges.shell]
color = "blue"
"#,
    );
    assert_eq!(config.conversation.badges, Badges::default());
    assert_eq!(
        keys(&problems),
        HashSet::from([
            "conversation.badges.force_ascii".to_string(),
            "conversation.badges.claude.glyph".to_string(),
            "conversation.badges.codex.ascii".to_string(),
            "conversation.badges.shell.color".to_string(),
        ])
    );
    assert_eq!(problems.len(), 4, "exactly one problem per bad key");
}

#[test]
fn a_two_column_glyph_is_accepted() {
    let (config, problems) = parse("[conversation.badges.claude]\nglyph = \"\u{1f9e0}\"\n");
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    assert_eq!(config.conversation.badges.claude.glyph, "\u{1f9e0}");

    let (config, problems) = parse("[conversation.badges.claude]\nglyph = \"\"\n");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].key, "conversation.badges.claude.glyph");
    assert_eq!(config.conversation.badges.claude.glyph, "\u{25c6}");
}

#[test]
fn a_conversation_table_of_the_wrong_type_keeps_every_default() {
    let (config, problems) = parse("conversation = 3\n");
    assert_eq!(keys(&problems), HashSet::from(["conversation".to_string()]));
    assert_eq!(problems[0].message, "expected a table");
    assert_eq!(config.conversation, Config::default().conversation);
}

#[test]
fn unknown_conversation_keys_are_reported() {
    let (config, problems) = parse(
        r##"
[conversation]
max_lines = 1

[conversation.badges]
zsh = {}

[conversation.badges.claude]
colour = "#fff"
"##,
    );
    assert_eq!(config.conversation, Conversation::default());
    assert_eq!(
        keys(&problems),
        HashSet::from([
            "conversation.max_lines".to_string(),
            "conversation.badges.zsh".to_string(),
            "conversation.badges.claude.colour".to_string(),
        ])
    );
    for problem in &problems {
        assert_eq!(problem.message, "unknown key, ignored");
    }
}

#[test]
fn conversation_range_boundaries_are_inclusive() {
    let low = r#"
[conversation]
max_turns = 1
linger_secs = 0
max_result_bytes = 4096
"#;
    let (config, problems) = parse(low);
    assert!(
        problems.is_empty(),
        "boundary-low values should be valid: {problems:?}"
    );
    assert_eq!(config.conversation.max_turns, 1);
    assert_eq!(config.conversation.linger_secs, 0);
    assert_eq!(config.conversation.max_result_bytes, 4096);

    let high = r#"
[conversation]
max_turns = 10000
linger_secs = 300
max_result_bytes = 1048576
"#;
    let (config, problems) = parse(high);
    assert!(
        problems.is_empty(),
        "boundary-high values should be valid: {problems:?}"
    );
    assert_eq!(config.conversation.max_turns, 10000);
    assert_eq!(config.conversation.linger_secs, 300);
    assert_eq!(config.conversation.max_result_bytes, 1048576);
}

#[test]
fn badges_for_runtime_picks_the_right_one() {
    let (config, problems) = parse(
        r##"
[conversation.badges.claude]
glyph = "▲"
ascii = "(C)"
color = "#112233"

[conversation.badges.codex]
glyph = "▼"
ascii = "(X)"
color = "#445566"

[conversation.badges.shell]
glyph = "#"
ascii = "(S)"
color = "#778899"
"##,
    );
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    let badges = &config.conversation.badges;
    assert_eq!(badges.for_runtime(proto::Runtime::Claude).glyph, "\u{25b2}");
    assert_eq!(badges.for_runtime(proto::Runtime::Codex).glyph, "\u{25bc}");
    assert_eq!(badges.for_runtime(proto::Runtime::Shell).glyph, "#");
}

/// Task M6.5.10 fix round 1 (F1): a whole conversation is sent in one frame, so
/// `max_bytes` stops at `proto::MAX_FRAME` less the headroom. The last legal value is
/// accepted and the next one refused.
#[test]
fn max_bytes_cannot_exceed_what_one_frame_carries() {
    let ceiling = proto::MAX_FRAME as u64 - crate::CONVERSATION_MAX_BYTES_HEADROOM;
    assert_eq!(ceiling, 15_728_640);
    let (config, problems) = parse(&format!("[conversation]\nmax_bytes = {ceiling}\n"));
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.conversation.max_bytes, ceiling);

    let (config, problems) = parse(&format!("[conversation]\nmax_bytes = {}\n", ceiling + 1));
    assert_eq!(
        keys(&problems),
        HashSet::from(["conversation.max_bytes".to_string()])
    );
    assert_eq!(
        config.conversation.max_bytes,
        Conversation::default().max_bytes
    );
}
