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
    "delivery.mode",
    "delivery.remote",
];

/// The `[delivery]` table's name in a profile's TOML: its keys are grouped as
/// `delivery.mode` and `delivery.remote` (milestone 9.2 decision 3; task M9.2.15).
const TABLES: &[&str] = &["delivery"];

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
        delivery: Some(proto::DeliveryProfile {
            mode: proto::DeliveryMode::Pr,
            remote: "origin".into(),
        }),
    }
}

/// Decision 24: every key once across the eleven sections, `env` as the environment
/// section's add row; the Advanced sections are marked.
#[test]
fn sections_cover_every_key_once() {
    let titles: Vec<&str> = sections().iter().map(|(t, _, _)| *t).collect();
    assert_eq!(
        titles,
        [
            "How anthrex checks your work",
            "Your repo",
            "Delivery",
            "testing tiers",
            "output filter",
            "environment",
            "timeouts",
            "test result pattern",
            "shared code area",
            "repo details",
            "delivery remote",
        ]
    );
    let listed: Vec<&str> = sections()
        .iter()
        .flat_map(|(_, _, keys)| keys.iter().copied())
        .collect();
    let wanted: Vec<&str> = EDIT_KEYS
        .iter()
        .copied()
        .filter(|key| *key != "env.<NAME>")
        .collect();
    for key in &wanted {
        let count = listed.iter().filter(|k| *k == key).count();
        assert_eq!(count, 1, "{key} is in {count} sections");
    }
    assert_eq!(listed.len(), wanted.len(), "{listed:?}");
    assert!(sections()[5].2.contains(&"env"));
    // And every key a real profile carries is a row, with its section.
    let all = rows(&full_profile(), None, None);
    let Ok(toml::Value::Table(table)) = toml::Value::try_from(full_profile()) else {
        panic!("a profile is a table");
    };
    for key in table.keys() {
        let has = |r: &Row| {
            r.key == *key
                || (TABLES.contains(&key.as_str()) && r.key.starts_with(&format!("{key}.")))
        };
        assert!(all.iter().any(has), "{key} has no row");
    }
    let find = |key: &str| all.iter().find(|r| r.key == key).unwrap().clone();
    assert!(all.iter().all(|r| !r.section.is_empty()));
    assert_eq!(find("check").section, "How anthrex checks your work");
    assert_eq!(find("check").label, "check");
    assert_eq!(find("env.RUST_LOG").section, "environment");
    assert_eq!(find("env.RUST_LOG").label, "RUST_LOG");
    assert_eq!(find("env").label, "add a variable");
    assert_eq!(find("delivery.mode").label, "Delivery");
    assert_eq!(find("delivery.remote").section, "delivery remote");
    assert_eq!(find("delivery.mode").value.as_deref(), Some("pr"));
    assert_eq!(find("delivery.remote").value.as_deref(), Some("origin"));
}

/// Decision 24: a row is Advanced when its section is.
#[test]
fn advanced_rows_are_marked() {
    let all = rows(&full_profile(), None, None);
    let find = |key: &str| all.iter().find(|r| r.key == key).unwrap().advanced;
    for key in ["setup", "check", "single_test", "source", "delivery.mode"] {
        assert!(!find(key), "{key} is a main row");
    }
    for key in [
        "build_check",
        "module_graph",
        "output_filter",
        "env.RUST_LOG",
        "env",
        "check_timeout_secs",
        "test_passed",
        "hub",
        "languages",
        "delivery.remote",
    ] {
        assert!(find(key), "{key} is Advanced");
    }
    let main: Vec<&str> = sections()
        .iter()
        .filter(|(_, advanced, _)| !advanced)
        .map(|(t, _, _)| *t)
        .collect();
    assert_eq!(
        main,
        ["How anthrex checks your work", "Your repo", "Delivery"]
    );
}

/// A row against the stored profile carries the stored value as `old`.
#[test]
fn a_proposal_row_carries_its_old_value() {
    let stored = RepoProfile {
        check: Some("cargo test".into()),
        setup: Some("make".into()),
        modules: vec!["a".into(), "b".into()],
        ..RepoProfile::default()
    };
    let proposal = RepoProfile {
        check: Some("cargo test --workspace".into()),
        build_check: Some("cargo build".into()),
        modules: vec!["a".into(), "b".into()],
        ..RepoProfile::default()
    };
    let all = rows(&proposal, None, Some(&stored));
    let find = |key: &str| all.iter().find(|r| r.key == key).unwrap().clone();
    assert_eq!(find("check").old.as_deref(), Some("cargo test"));
    assert_eq!(
        find("check").value.as_deref(),
        Some("cargo test --workspace")
    );
    assert_eq!(find("setup").old.as_deref(), Some("make"), "removed");
    assert_eq!(find("build_check").old, None, "added");
    assert_eq!(find("modules").old.as_deref(), Some("a, b"), "unchanged");
    // Without `against` nothing is old.
    assert!(rows(&proposal, None, None).iter().all(|r| r.old.is_none()));
}

/// The changes card lists only the marked rows, in order.
#[test]
fn changed_keeps_only_marked_rows() {
    let stored = RepoProfile {
        check: Some("cargo test".into()),
        setup: Some("make".into()),
        modules: vec!["a".into()],
        ..RepoProfile::default()
    };
    let proposal = RepoProfile {
        check: Some("cargo test --workspace".into()),
        build_check: Some("cargo build".into()),
        modules: vec!["a".into()],
        ..RepoProfile::default()
    };
    let kept: Vec<String> = changed(rows(&proposal, None, Some(&stored)))
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(kept, ["setup", "check", "build_check"]);
    assert!(changed(rows(&proposal, None, None)).is_empty());
}

/// The card's check cell: the glyph and how long the command took.
#[test]
fn card_cells() {
    let pass = check(Some(0), false, 12);
    let fail = check(Some(1), false, 190);
    assert_eq!(card_cell(&pass, false), "✓ 12s");
    assert_eq!(card_cell(&fail, false), "✗ 3m10s");
    assert_eq!(card_cell(&pass, true), "+ 12s");
    assert_eq!(card_cell(&fail, true), "x 3m10s");
}

/// Milestone 9.2 decision 3 (ruling, M9.2.15): a delivery edit sends the bare text
/// `proposal_delivery::edit` reads (`pr`, as `profile edit delivery.mode=pr` sends it),
/// never the quoted TOML string the other text keys send; the daemon's own
/// `apply_edit` takes what the screen sends.
#[test]
fn delivery_edits_send_the_bare_value_the_daemon_reads() {
    use daemon::profile::proposal::apply_edit;
    assert_eq!(kind_of("delivery.mode"), Kind::Choice(&["local", "pr"]));
    assert_eq!(kind_of("delivery.remote"), Kind::Text);
    assert_eq!(value_literal("delivery.mode", " pr ").as_deref(), Ok("pr"));
    assert_eq!(
        value_literal("delivery.remote", "  upstream ").as_deref(),
        Ok("upstream")
    );
    assert!(value_literal("delivery.remote", "   ").is_err());
    let stored = RepoProfile::default();
    let sent = value_literal("delivery.mode", "pr").unwrap();
    let (edited, verify) = apply_edit(&stored, "delivery.mode", Some(&sent)).unwrap();
    assert_eq!(
        edited.delivery,
        Some(proto::DeliveryProfile {
            mode: proto::DeliveryMode::Pr,
            remote: "origin".into(),
        })
    );
    assert!(!verify, "a delivery edit runs no command");
    let sent = value_literal("delivery.remote", "upstream").unwrap();
    let (edited, _) = apply_edit(&edited, "delivery.remote", Some(&sent)).unwrap();
    assert_eq!(
        edited.delivery.map(|d| d.remote).as_deref(),
        Some("upstream")
    );
    // What a text key's literal would have sent is refused.
    assert!(apply_edit(&stored, "delivery.mode", Some("\"pr\"")).is_err());
    // The editors start from the stored values.
    assert_eq!(edit_text(&full_profile(), "delivery.mode"), "pr");
    assert_eq!(edit_text(&full_profile(), "delivery.remote"), "origin");
    assert_eq!(edit_text(&RepoProfile::default(), "delivery.mode"), "");
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

/// Decision 24 (M9.10.9): `rows` walks the sections in order, each key at its place,
/// every `env.<NAME>` before the environment section's add row.
#[test]
fn rows_walk_the_sections_in_order() {
    let mut profile = full_profile();
    profile.env.insert("A_FIRST".into(), "1".into());
    let keys: Vec<String> = rows(&profile, None, None)
        .into_iter()
        .map(|r| r.key)
        .collect();
    let mut want: Vec<String> = Vec::new();
    for (_, _, section) in sections() {
        for key in section.iter() {
            if *key == ENV_ADD {
                want.extend(["env.A_FIRST".to_string(), "env.RUST_LOG".to_string()]);
            }
            want.push(key.to_string());
        }
    }
    assert_eq!(keys, want);
    let titles: Vec<&str> = rows(&profile, None, None)
        .iter()
        .map(|r| r.section)
        .collect();
    let mut deduped = titles.clone();
    deduped.dedup();
    let order: Vec<&str> = sections().iter().map(|(t, _, _)| *t).collect();
    assert_eq!(deduped, order, "each section's rows are together");
}
