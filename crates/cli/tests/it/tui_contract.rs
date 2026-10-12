//! A contract between the client and the daemon: the Profile screen's delivery edits
//! send the value the daemon's `apply_edit` reads. Moved from
//! `tui/src/profile_view_tests.rs` (milestone 9.12 decision 11), with a copy of that
//! file's `full_profile` fixture.

use daemon::profile::proposal::apply_edit;
use proto::{ModuleNames, RepoProfile};
use std::collections::BTreeMap;
use tui::profile_view::{Kind, edit_text, kind_of, value_literal};

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

/// Milestone 9.2 decision 3 (ruling, M9.2.15): a delivery edit sends the bare text
/// `proposal_delivery::edit` reads (`pr`, as `profile edit delivery.mode=pr` sends it),
/// never the quoted TOML string the other text keys send; the daemon's own
/// `apply_edit` takes what the screen sends.
#[test]
fn delivery_edits_send_the_bare_value_the_daemon_reads() {
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
