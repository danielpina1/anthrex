//! Milestone 9.14 task M9.14.7: the heavy matcher (decision 18, C13; pure). Split from
//! `admit_tests.rs` (task 8 review M2).

use super::*;

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

/// Task 7 review I1: a backslash-newline continuation is no word of its own.
#[test]
fn a_line_continuation_does_not_hide_the_program() {
    assert_heavy(&[
        "cd x && \\\n  cargo test",
        "RUST_LOG=1 \\\n cargo test",
        "cargo \\\n test",
        "cargo test \\\n --workspace",
    ]);
}

/// Task 7 review M1: a wrapper's options (and the values `nice -n`, `env -u` take)
/// are skipped.
#[test]
fn wrapper_options_are_skipped() {
    assert_heavy(&[
        "nice -n 19 cargo build",
        "nice -n19 cargo build",
        "time -p cargo test",
        "env -i cargo test",
        "env -u X cargo test",
        "env -i A=1 cargo test",
    ]);
    assert_light(&["nice -n 19 ls", "env -u X cargo fmt"]);
}

/// Task 7 review M2: a subshell's or a group's opening word, and a reserved word,
/// never hide the program.
#[test]
fn grouping_and_reserved_words_do_not_hide_the_program() {
    assert_heavy(&[
        "(cd x && cargo test)",
        "( cargo test )",
        "{ cargo test; }",
        "if cargo check; then echo ok; fi",
        "for c in a b; do cargo test; done",
        "while true; do cargo build; done",
        "! cargo test",
        "true && echo a || { cargo build; }",
        "if false; then :; else cargo test; fi",
    ]);
    assert_light(&["(cd x && cargo fmt)", "{ ls; }", "if true; then echo; fi"]);
}

/// Task 7 review M4: a redirection written without a space ends the subcommand.
#[test]
fn a_glued_redirection_does_not_hide_the_subcommand() {
    assert_heavy(&[
        "cargo test>log",
        "cargo build 2>err",
        "cargo build&>log",
        "cargo test<in",
        "cargo test>>log",
    ]);
    assert_light(&["cargo fmt>log", "cargo tree&>log"]);
}

/// Task 7 review M5 (the controller's ruling): `fix`, `rustc` and `rustdoc` compile
/// as `check` and `build` do.
#[test]
fn fix_rustc_and_rustdoc_count() {
    assert_heavy(&[
        "cargo fix --allow-dirty",
        "cargo rustc -- -C x",
        "cargo rustdoc",
    ]);
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
