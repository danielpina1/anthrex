//! Milestone 9.0.6 task 13: the Profile screen's pure view of a profile.

use super::*;
use proto::{CommandCheck, ModuleNames, ProfileVerification, RepoProfile};
use std::collections::BTreeMap;

/// A literal copy of `EDIT_KEYS` (`daemon/src/profile/proposal.rs:41`), written out so
/// a key the daemon gains goes red here until the screen groups it.
const EDIT_KEYS: &[&str] = &[
    "languages",
    "modules",
    "hub",
    "source",
    "generated",
    "protected",
    "setup",
    "check",
    "check_timeout_secs",
    "single_test",
    "test_passed",
    "sample_test",
    "output_filter",
    "filter_prefixes",
    "conventions",
    "manifests",
    "build_check",
    "module_test",
    "module_tests",
    "module_graph",
    "module_names",
    "full_triggers",
    "slow_tests",
    "timing_tests",
    "skip_markers",
    "test_paths",
    "full_shards",
    "toolchain_id",
    "env",
    "env.<NAME>",
];

/// Every field set, so its TOML table names every key a profile can carry.
fn full_profile() -> RepoProfile {
    let list = |s: &str| vec![s.to_string()];
    let text = |s: &str| Some(s.to_string());
    RepoProfile {
        languages: list("rust"),
        modules: list("crates/*"),
        hub: list("crates/proto"),
        source: list("src/**"),
        generated: list("gen/**"),
        protected: list("secrets/**"),
        setup: text("make setup"),
        check: text("cargo test"),
        check_timeout_secs: Some(600),
        single_test: text("cargo test {test}"),
        test_passed: text("test result: ok"),
        sample_test: text("a::b"),
        output_filter: proto::OutputFilter::Tail,
        filter_prefixes: list("test "),
        conventions: list("snake case"),
        manifests: list("Cargo.toml"),
        build_check: text("cargo build"),
        module_test: text("cargo test -p {module}"),
        module_tests: text("cargo test {modules}"),
        module_graph: text("cargo metadata"),
        module_names: Some(ModuleNames::Cargo),
        full_triggers: list("Cargo.lock"),
        slow_tests: text("--ignored"),
        timing_tests: text("timing"),
        skip_markers: list("#[ignore]"),
        test_paths: list("tests/**"),
        full_shards: Some(4),
        toolchain_id: text("rustc -V"),
        env: BTreeMap::from([("RUST_LOG".to_string(), "debug".to_string())]),
    }
}

/// Interfaces "Profile groups": every key once, in the four groups, in their order;
/// preflight F28: `EDIT_KEYS`' `env` is the environment group's (one row per
/// `env.<NAME>`).
#[test]
fn profile_groups_cover_every_key() {
    let names: Vec<&str> = groups().iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["commands", "tiers", "paths", "environment"]);
    let grouped: Vec<(&str, &str)> = groups()
        .iter()
        .flat_map(|(name, keys)| keys.iter().map(move |key| (*name, *key)))
        .collect();
    let wanted: Vec<&str> = EDIT_KEYS
        .iter()
        .copied()
        .filter(|key| *key != "env.<NAME>")
        .collect();
    for key in &wanted {
        let count = grouped.iter().filter(|(_, k)| k == key).count();
        assert_eq!(count, 1, "{key} is grouped {count} times");
    }
    assert_eq!(grouped.len(), wanted.len(), "{grouped:?}");
    assert!(grouped.contains(&("environment", "env")));
    // The commands group keeps the Interfaces order.
    assert_eq!(
        groups()[0].1,
        [
            "setup",
            "check",
            "check_timeout_secs",
            "single_test",
            "test_passed",
            "sample_test",
            "output_filter",
            "filter_prefixes",
        ]
    );
    // And every key a real profile carries is a row.
    let Ok(toml::Value::Table(table)) = toml::Value::try_from(full_profile()) else {
        panic!("a profile is a table");
    };
    for key in table.keys() {
        assert!(grouped.iter().any(|(_, k)| k == key), "{key} has no group");
    }
    let keys: Vec<String> = rows(&full_profile(), None, None)
        .into_iter()
        .map(|row| row.key)
        .collect();
    assert!(keys.contains(&"env.RUST_LOG".to_string()), "{keys:?}");
}

fn check(code: Option<i32>, timed_out: bool, secs: u64) -> CommandCheck {
    CommandCheck {
        command: "cargo test".into(),
        ok: code == Some(0) && !timed_out,
        code,
        timed_out,
        secs,
        tail: String::new(),
    }
}

/// Interfaces "Profile check cell", and its ASCII twin.
#[test]
fn check_cells_read_pass_fail_and_timeout() {
    let pass = check(Some(0), false, 4);
    let fail = check(Some(101), false, 12);
    let late = check(None, true, 600);
    assert_eq!(check_cell(&pass, false), "✓ 0 · 4s");
    assert_eq!(check_cell(&fail, false), "✗ 101 · 12s");
    assert_eq!(check_cell(&late, false), "✗ timed out · 600s");
    assert_eq!(check_cell(&pass, true), "+ 0 - 4s");
    assert_eq!(check_cell(&fail, true), "x 101 - 12s");
    assert_eq!(check_cell(&late, true), "x timed out - 600s");
}

/// A verified command's row carries its check, only while the command is the one run.
#[test]
fn a_rows_check_is_its_own_commands() {
    let profile = full_profile();
    let verification = ProfileVerification {
        at: 0,
        confined: true,
        setup: Some(CommandCheck {
            command: "something else".into(),
            ..check(Some(0), false, 1)
        }),
        check: Some(check(Some(0), false, 4)),
        single_test: None,
        build_check: None,
        module_graph: None,
        module_test: None,
        module_tests: None,
        toolchain_id: None,
    };
    let rows = rows(&profile, Some(&verification), None);
    let row = |key: &str| rows.iter().find(|r| r.key == key).unwrap().clone();
    assert_eq!(row("check").check.map(|c| c.secs), Some(4));
    assert_eq!(row("setup").check, None);
    assert_eq!(row("hub").check, None);
}

/// Decision 35: on the proposal view, `+` added, `−` removed, `~` changed; nothing
/// for an equal key, or one absent from both.
#[test]
fn a_proposal_is_marked_against_the_stored_profile() {
    let stored = RepoProfile {
        check: Some("cargo test".into()),
        setup: Some("make".into()),
        modules: vec!["crates/*".into()],
        env: BTreeMap::from([("A".into(), "1".into()), ("B".into(), "2".into())]),
        ..RepoProfile::default()
    };
    let proposal = RepoProfile {
        check: Some("cargo test --workspace".into()),
        modules: vec!["crates/*".into()],
        build_check: Some("cargo build".into()),
        env: BTreeMap::from([("A".into(), "1".into()), ("B".into(), "3".into())]),
        ..RepoProfile::default()
    };
    let rows = rows(&proposal, None, Some(&stored));
    let mark = |key: &str| {
        rows.iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("no row {key}"))
            .mark
    };
    assert_eq!(mark("check"), Some(Mark::Changed));
    assert_eq!(mark("setup"), Some(Mark::Removed));
    assert_eq!(mark("build_check"), Some(Mark::Added));
    assert_eq!(mark("modules"), None);
    assert_eq!(mark("hub"), None, "absent from both");
    assert_eq!(mark("output_filter"), None, "both the default");
    assert_eq!(mark("env.A"), None);
    assert_eq!(mark("env.B"), Some(Mark::Changed));
    // A removed key still has its row, with no value.
    let setup = rows.iter().find(|r| r.key == "setup").unwrap();
    assert_eq!(setup.value, None);
    // The stored view marks nothing.
    assert!(rows_of_stored(&stored).iter().all(|r| r.mark.is_none()));
    assert_eq!(Mark::Removed.glyph(false), "−");
    assert_eq!(Mark::Removed.glyph(true), "-");
    assert_eq!(Mark::Added.glyph(false), "+");
    assert_eq!(Mark::Changed.glyph(true), "~");
}

fn rows_of_stored(profile: &RepoProfile) -> Vec<Row> {
    rows(profile, None, None)
}

/// The rule `daemon/src/profile/proposal.rs:193-200` applies: the edit's value is read
/// as `v = <value>`.
fn read_back(literal: &str) -> toml::Value {
    let mut table = toml::from_str::<toml::Table>(&format!("v = {literal}"))
        .unwrap_or_else(|e| panic!("{literal}: {e}"));
    assert_eq!(table.len(), 1, "{literal}");
    table.remove("v").unwrap()
}

/// Decision 35: the value an edit sends is a TOML literal `parse_value` reads back as
/// exactly the typed value.
#[test]
fn value_literals_read_back_through_parse_value() {
    let command = r#"cargo test -- --exact "a b" 'c' \d"#;
    let cases: Vec<(&str, &str, toml::Value)> = vec![
        ("check", command, toml::Value::String(command.into())),
        (
            "check",
            "  cargo test  ",
            toml::Value::String("cargo test".into()),
        ),
        (
            "source",
            "src/**\n  crates/*/src/**  \n\n\"q\"\n",
            toml::Value::Array(vec![
                toml::Value::String("src/**".into()),
                toml::Value::String("crates/*/src/**".into()),
                toml::Value::String("\"q\"".into()),
            ]),
        ),
        ("full_shards", " 4 ", toml::Value::Integer(4)),
        ("check_timeout_secs", "600", toml::Value::Integer(600)),
        ("module_names", "cargo", toml::Value::String("cargo".into())),
        (
            "output_filter",
            "failures-only",
            toml::Value::String("failures-only".into()),
        ),
    ];
    for (key, typed, want) in cases {
        let literal = value_literal(key, typed).unwrap_or_else(|e| panic!("{key}: {e}"));
        assert_eq!(read_back(&literal), want, "{key}: {literal}");
    }
    assert!(value_literal("full_shards", "4x").is_err());
    assert!(value_literal("full_shards", "").is_err());
    assert!(value_literal("check", "   ").is_err());
    // An environment value is stored as written (`apply_edit` does not parse it).
    assert_eq!(value_literal("env.A", "x = 1").as_deref(), Ok("x = 1"));
}

/// Decision 35: each key's editor.
#[test]
fn every_key_has_an_editor_kind() {
    assert_eq!(kind_of("check"), Kind::Text);
    assert_eq!(kind_of("sample_test"), Kind::Text);
    assert_eq!(kind_of("source"), Kind::List);
    assert_eq!(kind_of("full_shards"), Kind::Number);
    assert_eq!(kind_of("check_timeout_secs"), Kind::Number);
    assert_eq!(kind_of("module_names"), Kind::Choice(&["cargo", "dir"]));
    assert_eq!(
        kind_of("output_filter"),
        Kind::Choice(&["failures-only", "tail", "none"])
    );
    assert_eq!(kind_of("env.RUST_LOG"), Kind::Env);
    assert_eq!(kind_of("env"), Kind::Env);
    // The editor starts from the shown value: a list one item per line.
    let profile = full_profile();
    assert_eq!(edit_text(&profile, "source"), "src/**");
    assert_eq!(edit_text(&profile, "full_shards"), "4");
    assert_eq!(edit_text(&profile, "module_names"), "cargo");
    assert_eq!(edit_text(&profile, "env.RUST_LOG"), "debug");
    assert_eq!(edit_text(&RepoProfile::default(), "check"), "");
}
