use super::*;
use serde_json::json;
use unicode_segmentation::UnicodeSegmentation;

/// One case per row of the table in the M6.5.4 brief. Every input value (file path,
/// pattern, command, ...) is distinct across cases so that no transposition between two
/// cases could pass by accident.
#[test]
fn every_named_tool_has_its_own_summary() {
    assert_eq!(
        for_tool(
            "Edit",
            Some(&json!({"file_path": "/repo/crates/parse/src/lib.rs"})),
        ),
        "lib.rs — 1 hunk"
    );

    assert_eq!(
        for_tool(
            "MultiEdit",
            Some(&json!({
                "file_path": "/repo/crates/tui/src/app/mod.rs",
                "edits": [{}, {}, {}],
            })),
        ),
        "mod.rs — 3 hunks"
    );

    assert_eq!(
        for_tool(
            "Write",
            Some(&json!({
                "file_path": "/repo/README.md",
                "content": "one\ntwo\nthree\nfour",
            })),
        ),
        "README.md — 4 lines"
    );

    // Not in the brief's illustrative list, but the table gives NotebookEdit its own row,
    // and the bullet says "one case per row of the table" -- so it gets a case too.
    assert_eq!(
        for_tool(
            "NotebookEdit",
            Some(&json!({"notebook_path": "/repo/notebooks/eda.ipynb"})),
        ),
        "eda.ipynb"
    );

    assert_eq!(
        for_tool(
            "Read",
            Some(&json!({"file_path": "/repo/Cargo.toml", "offset": 120})),
        ),
        "Cargo.toml:120"
    );

    assert_eq!(
        for_tool(
            "Bash",
            Some(&json!({"command": "cargo test --workspace\n# second line"})),
        ),
        "cargo test --workspace"
    );

    assert_eq!(
        for_tool(
            "Grep",
            Some(&json!({"pattern": "parse_", "path": "/repo/crates/parse"})),
        ),
        "\"parse_\" in parse"
    );

    assert_eq!(
        for_tool("Glob", Some(&json!({"pattern": "**/*.rs"}))),
        "**/*.rs"
    );

    assert_eq!(
        for_tool(
            "Task",
            Some(&json!({
                "subagent_type": "Explore",
                "description": "find every call site",
            })),
        ),
        "Explore · find every call site"
    );

    assert_eq!(
        for_tool(
            "WebFetch",
            Some(&json!({"url": "https://docs.rs/vt100/latest/vt100/"})),
        ),
        "docs.rs"
    );

    assert_eq!(
        for_tool("TodoWrite", Some(&json!({"todos": [{}, {}, {}, {}, {}]})),),
        "5 items"
    );
}

/// `Task` and `Agent` are one row in the table, sharing one rule. This pins the "Agent"
/// spelling to the same code path as "Task" so a match arm that only lists one of the two
/// names would be caught here, not just by chance elsewhere.
#[test]
fn agent_shares_the_task_rule() {
    assert_eq!(
        for_tool(
            "Agent",
            Some(&json!({"subagent_type": "Reviewer", "description": "check the diff"})),
        ),
        "Reviewer · check the diff"
    );
}

/// The optional side of `Task`/`Agent` drops cleanly, on either side, without leaving a
/// stray separator.
#[test]
fn task_drops_the_missing_side_and_its_separator() {
    assert_eq!(
        for_tool("Task", Some(&json!({"subagent_type": "Explore"}))),
        "Explore"
    );
    assert_eq!(
        for_tool(
            "Task",
            Some(&json!({"description": "audit the config crate"}))
        ),
        "audit the config crate"
    );
    assert_eq!(for_tool("Task", Some(&json!({}))), "");
}

/// `Grep` without a `path` omits the " in ..." suffix entirely rather than appending " in
/// " with nothing after it.
#[test]
fn grep_without_path_has_no_suffix() {
    assert_eq!(
        for_tool("Grep", Some(&json!({"pattern": "TODO"}))),
        "\"TODO\""
    );
}

/// `Read` without an integer `offset` omits the `:offset` suffix; a non-integer `offset`
/// (a string, here) is not confused for one.
#[test]
fn read_without_integer_offset_has_no_suffix() {
    assert_eq!(
        for_tool("Read", Some(&json!({"file_path": "/repo/x/y.rs"}))),
        "y.rs"
    );
    assert_eq!(
        for_tool(
            "Read",
            Some(&json!({"file_path": "/repo/x/z.rs", "offset": "12"})),
        ),
        "z.rs"
    );
}

/// `WebFetch` on a value with no `"://"` falls back to the whole value, per the table.
#[test]
fn web_fetch_without_scheme_uses_the_whole_value() {
    assert_eq!(
        for_tool("WebFetch", Some(&json!({"url": "example.com/no-scheme"}))),
        "example.com/no-scheme"
    );
}

/// An unrecognized tool name falls back to the first *string* value found among a fixed
/// key order; a non-string never satisfies a key even when it appears earlier in that
/// order.
#[test]
fn an_unknown_tool_falls_back_in_key_order() {
    assert_eq!(
        for_tool(
            "Frobnicate",
            Some(&json!({"query": "second", "command": "first"})),
        ),
        "first"
    );
    assert_eq!(
        for_tool("Frobnicate", Some(&json!({"query": "second"}))),
        "second"
    );
    assert_eq!(
        for_tool("Frobnicate", Some(&json!({"file_path": 7, "path": "/x"})),),
        "/x"
    );
    // None of the fallback keys present at all.
    assert_eq!(
        for_tool("Frobnicate", Some(&json!({"unrelated": "value"}))),
        ""
    );
}

/// No JSON shape a hook could plausibly deliver -- missing input, a non-object input, an
/// object missing the fields a rule expects, or a field of the wrong type -- may panic.
#[test]
fn missing_and_malformed_input_never_panics() {
    let cases: Vec<(&str, Option<Value>)> = vec![
        ("Edit", None),
        ("Edit", Some(json!([1, 2]))),
        ("Edit", Some(json!({}))),
        (
            "MultiEdit",
            Some(json!({"file_path": "/a/b", "edits": "not an array"})),
        ),
        ("", None),
    ];
    for (name, input) in &cases {
        let result = for_tool(name, input.as_ref());
        // Every result is a truncated String: at most SUMMARY_MAX_GRAPHEMES graphemes,
        // plus one more for the "…" when truncation happened.
        assert!(
            result.graphemes(true).count() <= SUMMARY_MAX_GRAPHEMES + 1,
            "for_tool({name:?}, {input:?}) produced an over-long summary: {result:?}"
        );
    }
}

/// Grapheme-cluster truncation, not byte or `char` truncation: a `chars()`-based cut would
/// split a multi-codepoint family-emoji cluster and miscount.
#[test]
fn long_summaries_truncate_by_grapheme() {
    let ascii = "a".repeat(200);
    let expected_ascii = format!("{}…", "a".repeat(80));
    assert_eq!(
        for_tool("Bash", Some(&json!({"command": ascii}))),
        expected_ascii
    );

    // U+1F468 U+200D U+1F469 U+200D U+1F467 U+200D U+1F466 -- a ZWJ family sequence: one
    // grapheme cluster made of seven `char`s. 100 of them is 700 `char`s but 100
    // graphemes, so a `chars()`-based truncation would cut mid-cluster.
    let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";
    let command = family.repeat(100);
    let result = for_tool("Bash", Some(&json!({"command": command})));
    let expected = format!("{}…", family.repeat(80));
    assert_eq!(result, expected);
    assert_eq!(result.graphemes(true).count(), 81);
}

/// Exactly at the limit: no truncation marker, and the text is not cut short by one.
#[test]
fn a_summary_at_exactly_the_limit_is_not_marked_truncated() {
    // 80 varying characters (not a single repeated unit) so the exact-boundary check
    // can't accidentally pass because a repeated unit's size divides the 80-grapheme
    // bound -- the failure mode task 2 found in `crates/cli/src/hook.rs`.
    let command: String = (0..80u32)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    assert_eq!(command.graphemes(true).count(), 80);

    let result = for_tool("Bash", Some(&json!({"command": command.clone()})));
    assert_eq!(result, command);
    assert!(!result.ends_with('…'));
}

/// `n == 1` is "1 hunk", not "1 hunks".
#[test]
fn multi_edit_counts_hunks_not_edits_of_one() {
    assert_eq!(
        for_tool(
            "MultiEdit",
            Some(&json!({
                "file_path": "/repo/crates/single/src/only.rs",
                "edits": [{}],
            })),
        ),
        "only.rs — 1 hunk"
    );
}
