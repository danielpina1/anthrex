//! Milestone 9.14 task M9.14.7: the heavy matcher, the hook group, release and the
//! hold book (decisions 18, 20, 22 and 23; pure).

use super::*;
use proto::HookSource;
use serde_json::json;

fn heavy(command: &str) -> bool {
    is_heavy(command, &[])
}

fn assert_heavy(commands: &[&str]) {
    for command in commands {
        assert!(heavy(command), "{command:?} should be heavy");
    }
}

fn assert_light(commands: &[&str]) {
    for command in commands {
        assert!(!heavy(command), "{command:?} should not be heavy");
    }
}

#[test]
fn heavy_commands_match_after_leading_cd_and_assignments() {
    assert_heavy(&["cd crates/x && RUST_LOG=1 cargo test -q", "cargo test"]);
    assert_light(&[
        "cargo testing",
        "echo cargo test",
        "echo 'a && cargo test'",
        "cargo fmt",
        "cargo tree",
        "cargo --version",
        "",
    ]);
}

#[test]
fn a_segment_after_and_or_semicolon_or_pipe_counts() {
    assert_heavy(&[
        "git status && cargo build",
        "true || cargo check",
        "ls; cargo clippy",
        "cargo-nextest run | tee x",
        "sleep 1 & cargo test",
        "echo a\ncargo test",
    ]);
    // A redirection's `&` is not a separator.
    assert_light(&["cargo fmt 2>&1", "echo \"x; cargo test\""]);
}

#[test]
fn cargo_aliases_toolchains_global_flags_and_wrappers_count() {
    assert_heavy(&[
        "cargo t",
        "cargo b",
        "cargo c",
        "cargo r",
        "cargo d",
        "cargo +nightly test",
        "cargo -q test",
        "cargo --locked build",
        "cargo --offline test",
        "cargo --config k=v test",
        "cargo --config=k=v test",
        "cargo -Z unstable-options check",
        "cargo --color never clippy",
        "cargo --manifest-path a/Cargo.toml test",
        "/home/u/.cargo/bin/cargo test",
        "time cargo test",
        "nice cargo build",
        "env RUST_LOG=1 cargo test",
        "command cargo check",
        "nohup cargo build",
        "exec cargo test",
    ]);
    assert_light(&["cargo +nightly fmt", "cargo -q tree", "time ls"]);
}

#[test]
fn profile_prefixes_count_too() {
    let prefixes = vec!["./slow-test".to_string(), "make test".to_string()];
    assert!(is_heavy("./slow-test --all", &prefixes));
    assert!(is_heavy("ls && ./slow-test", &prefixes));
    assert!(is_heavy("make test -j2", &prefixes));
    assert!(!is_heavy("./slow-testing --all", &prefixes));
    assert!(!is_heavy("make tests", &prefixes));
    assert!(!is_heavy("./slow-test --all", &[]));
}

#[test]
fn the_hook_command_and_group_are_exact() {
    let exe = Path::new("/opt/anthrex/bin/anthrex");
    let hook = AdmitHook {
        wait_secs: 300,
        slots: Some(2),
        prefixes: vec!["make test".into()],
    };
    let command = hook_command(exe, 7, &hook);
    assert_eq!(
        command,
        "'/opt/anthrex/bin/anthrex' admit --window 7 --wait-secs 300 --slots 2 --prefix 'make test'"
    );
    let half = AdmitHook {
        wait_secs: 120,
        slots: None,
        prefixes: Vec::new(),
    };
    assert_eq!(
        hook_command(exe, 3, &half),
        "'/opt/anthrex/bin/anthrex' admit --window 3 --wait-secs 120"
    );

    let filter = json!({"matcher": "Bash", "hooks": [{"type": "command", "command": "f"}]});
    let before = json!({
        "permissions": {"allow": ["Bash"]},
        "hooks": {
            "PreToolUse": [{"matcher": "*", "hooks": []}, filter],
            "Stop": [{"hooks": []}],
        },
    });
    let mut settings = before.clone();
    add_hook(&mut settings, exe, 7, Some(&hook));
    let mut expected = before.clone();
    expected["hooks"]["PreToolUse"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "matcher": "Bash",
            "hooks": [{"type": "command", "command": command, "timeout": 330}],
        }));
    assert_eq!(settings, expected);

    let mut untouched = before.clone();
    add_hook(&mut untouched, exe, 7, None);
    assert_eq!(untouched, before);
}

fn parsed(payload: serde_json::Value) -> ParsedHook {
    crate::hooks::parse(HookSource::Claude, &payload).expect("a hook payload")
}

fn event(name: &str, id: Option<&str>) -> ParsedHook {
    let mut payload = json!({"hook_event_name": name, "session_id": "s"});
    if let Some(id) = id {
        payload["tool_use_id"] = json!(id);
    }
    parsed(payload)
}

#[test]
fn release_for_each_event() {
    assert_eq!(
        release_for(&event("PostToolUse", Some("x"))),
        Release::One("x".into())
    );
    assert_eq!(release_for(&event("PostToolUse", None)), Release::Nothing);
    assert_eq!(
        release_for(&event("PreToolUse", Some("y"))),
        Release::AllBut("y".into())
    );
    assert_eq!(release_for(&event("PreToolUse", None)), Release::Nothing);
    for name in ["Stop", "StopFailure", "UserPromptSubmit", "SessionEnd"] {
        assert_eq!(release_for(&event(name, None)), Release::Window, "{name}");
    }
    assert_eq!(release_for(&event("Notification", None)), Release::Nothing);
}

/// A book with `holds`, each granted `at`; a hold's grant is its tool use id.
fn book_of(holds: &[(u32, &str)], at: Instant) -> HoldBook<String> {
    let mut book = HoldBook::default();
    for (window, id) in holds {
        book.insert(*window, id.to_string(), id.to_string(), at);
    }
    book
}

#[test]
fn a_newer_pre_tool_use_releases_the_older_holds() {
    let at = Instant::now();
    let mut book = book_of(&[(1, "a"), (1, "b"), (2, "c")], at);
    assert_eq!(book.release(1, &Release::AllBut("b".into())), ["a"]);
    assert!(book.get(1, "b").is_some());
    assert!(book.get(2, "c").is_some());
    assert_eq!(book.release(1, &Release::AllBut("z".into())), ["b"]);
    assert!(book.get(2, "c").is_some());
}

#[test]
fn a_post_tool_use_releases_the_hold() {
    let at = Instant::now();
    let mut book: HoldBook<u32> = HoldBook::default();
    book.insert(1, "a".into(), 10, at);
    book.insert(1, "b".into(), 20, at);
    assert_eq!(book.release(1, &Release::One("a".into())), [10]);
    assert_eq!(book.get(1, "a"), None);
    assert_eq!(book.get(1, "b"), Some(&20));
    assert_eq!(
        book.release(2, &Release::One("b".into())),
        Vec::<u32>::new()
    );
    assert_eq!(book.release(1, &Release::Nothing), Vec::<u32>::new());
}

#[test]
fn stop_releases_every_hold_of_the_window() {
    let at = Instant::now();
    let mut book = book_of(&[(1, "a"), (1, "b"), (2, "c")], at);
    let mut released = book.release(1, &Release::Window);
    released.sort();
    assert_eq!(released, ["a", "b"]);
    assert!(book.get(2, "c").is_some());
}

#[test]
fn the_sweep_releases_old_holds_and_holds_of_gone_windows() {
    let start = Instant::now();
    let max_age = Duration::from_secs(630);
    let mut book = book_of(&[(1, "old")], start);
    let fresh = start + Duration::from_secs(100);
    book.insert(1, "fresh".into(), "fresh".into(), fresh);
    book.insert(2, "gone".into(), "gone".into(), fresh);
    let now = start + max_age + Duration::from_secs(1);
    let live = |window: u32| window == 1;
    let mut released = book.sweep(now, max_age, &live);
    released.sort();
    assert_eq!(released, ["gone", "old"]);
    assert!(book.get(1, "fresh").is_some());
    // Exactly the max age is not older than it.
    let mut edge = book_of(&[(1, "edge")], start);
    assert!(edge.sweep(start + max_age, max_age, &|_| true).is_empty());
}

#[test]
fn a_repeated_ask_is_the_same_hold() {
    let at = Instant::now();
    let mut book: HoldBook<u32> = HoldBook::default();
    assert_eq!(book.get(1, "a"), None);
    book.insert(1, "a".into(), 3, at);
    // The caller checks `get` before a second insert (decision 23): the held grant.
    assert_eq!(book.get(1, "a"), Some(&3));
    assert_eq!(book.get(2, "a"), None);
}
