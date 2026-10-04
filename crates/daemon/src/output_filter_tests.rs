use super::*;
use proto::OutputFilter;
use serde_json::json;
use std::path::{Path, PathBuf};

const EXE: &str = "/opt/anthrex/bin/anthrex";

fn numbered(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("line {i}")).collect()
}

fn hook(prefixes: &[&str]) -> FilterHook {
    FilterHook {
        mode: OutputFilter::FailuresOnly,
        prefixes: prefixes.iter().map(|p| p.to_string()).collect(),
        log_dir: PathBuf::from("/tmp/ax/t1/anthrex-logs"),
    }
}

fn omitted(n: usize) -> String {
    format!("[anthrex] … {n} lines omitted …")
}

#[test]
fn failures_only_keeps_matches_with_context_and_the_tail() {
    let mut lines = numbered(5000);
    lines[2499] = "test foo ... FAILED".into();
    lines[4999] = "test result: FAILED. 1 passed; 1 failed".into();
    let out = apply(OutputFilter::FailuresOnly, &lines, 101);
    let mut expected = vec![omitted(2499)];
    expected.extend(lines[2499..2505].iter().cloned());
    expected.push(omitted(4980 - 2505));
    expected.extend(lines[4980..].iter().cloned());
    assert_eq!(out, expected);
    assert_eq!(out.len(), 28);
}

#[test]
fn failures_only_on_success_is_the_last_10_lines() {
    let mut lines = numbered(500);
    lines[100] = "error: ignored on success".into();
    let out = apply(OutputFilter::FailuresOnly, &lines, 0);
    assert_eq!(out, lines[490..].to_vec());
}

#[test]
fn failures_only_without_matches_is_the_last_60() {
    let lines = numbered(500);
    assert_eq!(
        apply(OutputFilter::FailuresOnly, &lines, 2),
        lines[440..].to_vec()
    );
    // Fewer lines than the view: every line.
    let short = numbered(7);
    assert_eq!(apply(OutputFilter::FailuresOnly, &short, 2), short);
}

#[test]
fn at_most_100_matched_lines() {
    let lines: Vec<String> = (1..=1000).map(|i| format!("error {i}")).collect();
    let out = apply(OutputFilter::FailuresOnly, &lines, 1);
    let mut expected = lines[..100].to_vec();
    expected.push(omitted(1000 - 100 - 20));
    expected.extend(lines[980..].iter().cloned());
    assert_eq!(out, expected);
    // Overlapping ranges merge: two matches three lines apart keep one range.
    let mut lines = numbered(200);
    lines[10] = "assertion failed".into();
    lines[13] = "panicked at x".into();
    let out = apply(OutputFilter::FailuresOnly, &lines, 1);
    let mut expected = vec![omitted(10)];
    expected.extend(lines[10..19].iter().cloned());
    expected.push(omitted(180 - 19));
    expected.extend(lines[180..].iter().cloned());
    assert_eq!(out, expected);
}

#[test]
fn tail_is_60_lines() {
    let lines = numbered(500);
    assert_eq!(apply(OutputFilter::Tail, &lines, 1), lines[440..].to_vec());
    assert_eq!(apply(OutputFilter::Tail, &lines, 0), lines[440..].to_vec());
    assert_eq!(apply(OutputFilter::None, &lines, 1), lines);
}

#[test]
fn long_lines_are_cut_to_500_chars() {
    let long = "世".repeat(700);
    let lines = vec![long.clone(), "ok".to_string()];
    for mode in [
        OutputFilter::None,
        OutputFilter::Tail,
        OutputFilter::FailuresOnly,
    ] {
        let out = apply(mode, &lines, 0);
        assert_eq!(out[0], "世".repeat(LINE_MAX_CHARS), "{mode:?}");
        assert_eq!(out[0].chars().count(), 500);
        assert_eq!(out[1], "ok");
    }
    let failing = vec![format!("error: {long}")];
    let out = apply(OutputFilter::FailuresOnly, &failing, 1);
    assert_eq!(out[0].chars().count(), 500);
}

#[test]
fn matches_strips_cd_and_env_prefixes_and_respects_word_boundaries() {
    let prefixes = vec!["cargo test".to_string(), "npm run".to_string()];
    for yes in [
        "cargo test",
        "cargo test -q -p daemon",
        "cargo test\t--all",
        "cd crates/daemon && cargo test",
        "cd a && cd b &&  RUST_LOG=debug cargo test",
        "RUST_BACKTRACE=1 FOO_2=x cargo test --lib",
        "  npm run lint",
    ] {
        assert!(matches(yes, &prefixes), "{yes:?}");
    }
    for no in [
        "cargo testing",
        "cargo build",
        "echo cargo test",
        "cd && cargo test",
        "cd 'a b' && cargo test",
        "1FOO=x cargo test",
        "'/x/anthrex' filter-run --mode tail --log-dir '/l' -c 'cargo test'",
        "cargo test | anthrex filter-run x",
        "",
    ] {
        assert!(!matches(no, &prefixes), "{no:?}");
    }
    assert!(!matches("cargo test", &[]));
}

/// M8b.18: a prefix that ends in a path separator (the brief's `filter_prefixes = ["sh
/// tests/"]`, and what `derived_prefixes` makes of `single_test = "sh
/// tests/{test}.sh"`) already ends at a word boundary, so the path that follows it
/// matches. A prefix that ends in a word character still needs the end or whitespace.
#[test]
fn a_prefix_ending_in_a_separator_matches_the_path_after_it() {
    let prefixes = vec!["sh tests/".to_string(), "cargo test".to_string()];
    for yes in [
        "sh tests/noisy.sh",
        "sh tests/t_ok.sh",
        "cd sub && sh tests/x.sh -v",
        "sh tests/",
    ] {
        assert!(matches(yes, &prefixes), "{yes:?}");
    }
    for no in ["sh testsuite", "sh tests", "cargo testing", "sh test/x.sh"] {
        assert!(!matches(no, &prefixes), "{no:?}");
    }
}

#[test]
fn wrap_quotes_every_part() {
    let hook = FilterHook {
        mode: OutputFilter::Tail,
        ..hook(&["cargo test"])
    };
    assert_eq!(
        wrap(Path::new(EXE), &hook, "echo \"it's\""),
        "'/opt/anthrex/bin/anthrex' filter-run --mode tail --log-dir '/tmp/ax/t1/anthrex-logs' -c 'echo \"it'\\''s\"'"
    );
    assert_eq!(
        hook_command(Path::new(EXE), &hook),
        "'/opt/anthrex/bin/anthrex' filter-hook --mode tail --log-dir '/tmp/ax/t1/anthrex-logs' --prefix 'cargo test'"
    );
}

#[test]
fn rewrite_keeps_every_tool_input_key() {
    let hook = hook(&["cargo test"]);
    let payload = json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_use_id": "toolu_1",
        "tool_input": {
            "command": "cargo test -q",
            "description": "Run the tests",
            "timeout": 600000,
            "run_in_background": false
        }
    });
    let out = rewrite(&payload, Path::new(EXE), &hook).expect("rewritten");
    let mut expected = json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": {
            "command": wrap(Path::new(EXE), &hook, "cargo test -q"),
            "description": "Run the tests",
            "timeout": 600000,
            "run_in_background": false
        }
    }});
    if HOOK_SETS_ALLOW {
        expected["hookSpecificOutput"]["permissionDecision"] = json!("allow");
    }
    assert_eq!(out, expected);
    // Other tools, other commands, and a payload without a string command: nothing.
    let mut read = payload.clone();
    read["tool_name"] = json!("Read");
    assert_eq!(rewrite(&read, Path::new(EXE), &hook), None);
    let mut other = payload.clone();
    other["tool_input"]["command"] = json!("ls");
    assert_eq!(rewrite(&other, Path::new(EXE), &hook), None);
    let mut broken = payload.clone();
    broken["tool_input"]["command"] = json!(["cargo", "test"]);
    assert_eq!(rewrite(&broken, Path::new(EXE), &hook), None);
    assert_eq!(rewrite(&json!("x"), Path::new(EXE), &hook), None);
}

#[test]
fn rewrite_never_double_wraps() {
    // Even a prefix that the wrapped command itself starts with does not wrap twice.
    let exe = Path::new("cargo");
    let hook = hook(&["cargo test", "'cargo'"]);
    let payload = json!({"tool_name": "Bash", "tool_input": {"command": "cargo test"}});
    let once = rewrite(&payload, exe, &hook).expect("wrapped once");
    let wrapped = once["hookSpecificOutput"]["updatedInput"].clone();
    assert!(
        wrapped["command"]
            .as_str()
            .unwrap()
            .contains(" filter-run ")
    );
    let again = json!({"tool_name": "Bash", "tool_input": wrapped});
    assert_eq!(rewrite(&again, exe, &hook), None);
}

#[test]
fn add_hook_appends_a_second_group() {
    let caps = crate::headless::argv::CLI_CAPS;
    let sandbox = crate::headless::ClaudeSandbox {
        writable_roots: vec![PathBuf::from("/tmp/ax/t1")],
        deny_write: vec![PathBuf::from("/tmp/wt/.claude")],
    };
    let exe = Path::new(EXE);
    let before = crate::headless::argv::claude_settings(exe, 4, Some(&sandbox), &caps);
    let mut settings = before.clone();
    add_hook(&mut settings, exe, None);
    assert_eq!(settings, before);
    let hook = hook(&["cargo test", "cargo build"]);
    add_hook(&mut settings, exe, Some(&hook));
    let m3 = json!({
        "hooks": [{"command": "'/opt/anthrex/bin/anthrex' hook --window 4 --source claude", "type": "command"}],
        "matcher": ""
    });
    let filter = json!({
        "matcher": "Bash",
        "hooks": [{"type": "command", "command":
            "'/opt/anthrex/bin/anthrex' filter-hook --mode failures-only --log-dir '/tmp/ax/t1/anthrex-logs' --prefix 'cargo test' --prefix 'cargo build'"}]
    });
    let mut expected = before.clone();
    expected["hooks"]["PreToolUse"] = json!([m3, filter]);
    assert_eq!(settings, expected);
    // The sandbox block, its pins, every other event and key are untouched.
    assert_eq!(settings["sandbox"], before["sandbox"]);
    for (key, value) in before["hooks"].as_object().unwrap() {
        if key != "PreToolUse" {
            assert_eq!(&settings["hooks"][key], value, "{key}");
        }
    }
    assert_eq!(
        settings.as_object().unwrap().keys().collect::<Vec<_>>(),
        before.as_object().unwrap().keys().collect::<Vec<_>>()
    );
}

/// Review I2: leading `cd <dir> && ` segments stay outside the wrapper, so the Bash
/// tool's shell still moves; assignments stay inside.
#[test]
fn wrap_keeps_leading_cd_segments_outside() {
    let hook = hook(&["cargo test"]);
    assert_eq!(
        wrap(Path::new(EXE), &hook, "cd a && cd 'b' &&  FOO=1 cargo test"),
        "cd a && cd 'b' &&  '/opt/anthrex/bin/anthrex' filter-run --mode failures-only --log-dir '/tmp/ax/t1/anthrex-logs' -c 'FOO=1 cargo test'"
    );
    assert_eq!(
        wrap(Path::new(EXE), &hook, "FOO=1 cd a && cargo test"),
        "'/opt/anthrex/bin/anthrex' filter-run --mode failures-only --log-dir '/tmp/ax/t1/anthrex-logs' -c 'FOO=1 cd a && cargo test'"
    );
    let payload = json!({"tool_name": "Bash", "tool_input": {"command": "cd x && cargo test"}});
    let out = rewrite(&payload, Path::new(EXE), &hook).unwrap();
    let wrapped = out["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    assert!(wrapped.starts_with("cd x && '/opt/anthrex/bin/anthrex' filter-run "));
    // Never wrapped twice.
    let again = json!({"tool_name": "Bash", "tool_input": {"command": wrapped}});
    assert_eq!(rewrite(&again, Path::new(EXE), &hook), None);
}

/// Review M2: `shell_quote` turns each `'` into four bytes, so a command whose wrapped
/// form would pass `WRAPPED_MAX` is left alone rather than risk `E2BIG`.
#[test]
fn rewrite_leaves_an_oversized_command_alone() {
    let hook = hook(&["echo"]);
    let payload =
        |command: String| json!({"tool_name": "Bash", "tool_input": {"command": command}});
    let quotes = format!("echo {}", "''".repeat(20_000));
    assert!(quotes.len() < WRAPPED_MAX && wrap(Path::new(EXE), &hook, &quotes).len() > WRAPPED_MAX);
    assert_eq!(rewrite(&payload(quotes), Path::new(EXE), &hook), None);
    let plain = format!("echo {}", "x".repeat(100_000));
    assert!(wrap(Path::new(EXE), &hook, &plain).len() <= WRAPPED_MAX);
    assert!(rewrite(&payload(plain), Path::new(EXE), &hook).is_some());
}

/// Milestone 9.5 decision 28 (`CodexFilter::Instruction`): the note a Codex worker's
/// `developer_instructions` gets, word for word, the executable and the log directory
/// shell-quoted as [`wrap`] quotes them.
#[test]
fn codex_filter_note_is_the_exact_text() {
    let mut tail = hook(&["cargo test", "sh tests/"]);
    tail.mode = OutputFilter::Tail;
    tail.log_dir = PathBuf::from("/data/runs/r1/tmp/t 1/anthrex-logs");
    assert_eq!(
        codex_filter_note(Path::new(EXE), &tail),
        "Test output: run every command that starts with cargo test, sh tests/ through \
         the output filter, as '/opt/anthrex/bin/anthrex' filter-run --mode tail \
         --log-dir '/data/runs/r1/tmp/t 1/anthrex-logs' -c '<command>'. It prints the \
         failures and the path of the full log."
    );
    let one = hook(&["cargo test"]);
    assert!(codex_filter_note(Path::new(EXE), &one).starts_with(
        "Test output: run every command that starts with cargo test through the \
             output filter, as '/opt/anthrex/bin/anthrex' filter-run --mode failures-only \
             --log-dir '/tmp/ax/t1/anthrex-logs' -c '<command>'."
    ));
}
