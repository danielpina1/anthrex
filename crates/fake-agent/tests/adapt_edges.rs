//! M8b.6 review minors: the edges of decisions 36 and 37 that `adapt_modes.rs` leaves
//! open. Real pipes, real git repositories, the real `fake-agent` binary.

mod headless_support;

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use daemon::headless::claude_stream;
use headless_support::*;
use serde_json::json;

/// A hook script at `<dir>/<name>.sh` that reads its payload and prints an
/// `updatedInput` with `command`.
fn rewrite_hook(dir: &Path, name: &str, command: &str) -> String {
    let path = dir.join(format!("{name}.sh"));
    let output = json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": {"command": command},
    }});
    fs::write(
        &path,
        format!("cat > /dev/null\nprintf '%s\\n' '{output}'\n"),
    )
    .unwrap();
    format!("sh '{}'", path.display())
}

#[test]
fn the_last_updated_input_wins() {
    let dir = tempdir();
    let settings = json!({"hooks": {"PreToolUse": [
        group("Bash", &[rewrite_hook(dir.path(), "first", "echo rewritten")]),
        group("", &[rewrite_hook(dir.path(), "second", "echo later")]),
    ]}});
    let (_, log) = run_bash(
        dir.path(),
        Some(&settings),
        &[json!({"bash": {"cmd": "echo original"}})],
    );

    assert_eq!(log[0]["ran"], "echo later", "{log:?}");
    assert_eq!(log[0]["output_lines"], json!(["later"]));
}

/// The real CLI treats a `PreToolUse` hook's non-zero exit (other than 2) as a
/// non-blocking error and runs the tool as it was.
#[test]
fn a_failing_pre_tool_use_hook_runs_the_original_command() {
    let dir = tempdir();
    let settings = json!({"hooks": {"PreToolUse": [
        group("Bash", &["cat > /dev/null; echo broken >&2; exit 1".to_string()]),
    ]}});
    let (agent, log) = run_bash(
        dir.path(),
        Some(&settings),
        &[json!({"bash": {"cmd": "echo original"}})],
    );

    assert_eq!(
        log,
        vec![
            json!({"original": "echo original", "ran": "echo original", "exit": 0,
                    "output_lines": ["original"]})
        ]
    );
    let result = agent.of_type("result").pop().unwrap();
    assert_eq!(result["is_error"], json!(false), "{result}");
}

#[test]
fn a_missing_decider_directory_exits_2() {
    let (work, dir) = (tempdir(), tempdir());
    let missing = dir.path().join("never-made");
    let args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
    ]
    .map(String::from)
    .to_vec();
    let mut agent = Agent::spawn(&args, work.path(), &[("FAKE_AGENT_DECIDER_DIR", &missing)]);
    agent.send(&claude_stream::user_message(
        "[anthrex decider] triage v1\nDecide.",
        None,
    ));
    agent.close_stdin();

    assert_eq!(agent.wait(RUN).code(), Some(2), "stderr {}", agent.stderr());
    assert!(
        agent
            .stderr()
            .contains("fake-agent: no scripted decider answer for triage"),
        "{}",
        agent.stderr()
    );
    assert!(!missing.exists());
}

/// Runs a scout session with id `scout` in `repo`; returns its assistant texts.
fn scout_texts(repo: &Path, scout: &str) -> Vec<String> {
    let mut agent = Agent::spawn(&scout_argv(scout), repo, &[]);
    agent.send(&user_message("scout", "s"));
    agent.until(RUN, is_result);
    agent.close_stdin();
    assert!(agent.wait(RUN).success(), "stderr {}", agent.stderr());
    texts(&agent)
}

#[test]
fn an_exact_scout_id_beats_the_base_name() {
    let dir = tempdir();
    let repo = repo(dir.path());
    role_script(&repo, "scout-onboarding-1", &[json!({"print": "base"})]);
    role_script(
        &repo,
        "scout-onboarding-1695000000-1",
        &[json!({"print": "exact"})],
    );

    assert_eq!(scout_texts(&repo, "onboarding-1695000000"), ["exact"]);
    assert_eq!(scout_texts(&repo, "onboarding-1695000001"), ["base"]);
}

/// Only a timestamp suffix (9 or more digits) is stripped, so the area scout `api-2`
/// never takes `api`'s script.
#[test]
fn only_a_timestamp_suffix_falls_back_to_the_base_name() {
    let dir = tempdir();
    let repo = repo(dir.path());
    role_script(&repo, "scout-api-1", &[json!({"print": "api"})]);

    assert!(scout_texts(&repo, "api-2").is_empty());
    assert!(scout_texts(&repo, "api-12345678").is_empty());
    assert!(
        !repo
            .join(".git/fake-agent/scout-api-1.jsonl.claimed")
            .exists()
    );
    assert_eq!(scout_texts(&repo, "api-123456789"), ["api"]);
}

/// Waits, within `RUN`, for `path` to exist; stdin stays open and unwritten meanwhile.
fn exists_before_stdin(path: &Path) -> bool {
    let deadline = Instant::now() + RUN;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

/// The first-message read is only for a session with no MCP server that is not
/// resumed: a role session and a resumed one claim and record before any stdin.
#[test]
fn role_and_resumed_sessions_do_not_read_stdin_before_starting() {
    let dir = tempdir();
    let repo = repo(dir.path());
    role_script(&repo, "worker-t1-1", &[json!({"print": "working"})]);
    let records = dir.path().join("args");
    fs::create_dir(&records).unwrap();
    let env: &[(&str, &Path)] = &[("FAKE_AGENT_ARGS_FILE", &records)];

    let mcp = Mcp::unused("worker", "t1");
    let role = Agent::spawn(&claude_argv(Session::New("s-role"), Some(&mcp)), &repo, env);
    let claimed = repo.join(".git/fake-agent/worker-t1-1.jsonl.claimed");
    assert!(
        exists_before_stdin(&claimed),
        "the role session did not claim"
    );
    // The argv is recorded just after the claim (the claim names the record), so it
    // is waited for too, with nothing written to stdin yet.
    assert!(
        exists_before_stdin(&records.join("worker-t1-1.args")),
        "the role session did not record its argv"
    );
    drop(role);

    let resumed = Agent::spawn(&claude_argv(Session::Resume("s-old"), None), &repo, env);
    assert!(
        exists_before_stdin(&records.join("fake-agent.args")),
        "the resumed session did not record its argv"
    );
    drop(resumed);
}

/// A hook command that appends its payload, one line, to `file`.
fn record(file: &Path) -> String {
    format!("{{ cat; echo; }} >> '{}'", file.display())
}

fn payloads(file: &Path) -> Vec<serde_json::Value> {
    (lines(file).iter())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Milestone 9.14 (Interfaces, `fake-agent`): with `FAKE_AGENT_TOOL_IDS=1` each `bash`
/// call's `PreToolUse` payload carries `fake-tu-<n>`, and a call whose command exits 0 is
/// followed by one `PostToolUse` with the same id, its command, stdout and exit code.
/// Milestone 9.18 decision 14: a call that exits non-zero fires `PostToolUseFailure`
/// instead (`a_failed_bash_step_fires_post_tool_use_failure`). Without the variable,
/// none of them.
#[test]
fn tool_ids_and_post_tool_use_only_with_the_variable() {
    let steps = [
        json!({"bash": {"cmd": "echo one"}}),
        json!({"bash": {"cmd": "echo two; exit 3"}}),
    ];
    for with_ids in [true, false] {
        let dir = tempdir();
        let (pre, post) = (dir.path().join("pre"), dir.path().join("post"));
        let failed = dir.path().join("failed");
        let order = dir.path().join("order");
        let settings = json!({"hooks": {
            "PreToolUse": [group("Bash", &[record(&pre)])],
            "PostToolUse": [
                group("Bash", &[record(&post)]),
                group("Edit", &[format!("cat > /dev/null; echo edit >> '{}'", order.display())]),
            ],
            "PostToolUseFailure": [group("Bash", &[record(&failed)])],
        }});
        let one = Path::new("1");
        let extra: &[(&str, &Path)] = if with_ids {
            &[("FAKE_AGENT_TOOL_IDS", one)]
        } else {
            &[]
        };
        let (_, log) = run_bash_with(dir.path(), Some(&settings), &steps, extra);
        assert_eq!(log.len(), 2, "{log:?}");
        let pre = payloads(&pre);
        assert_eq!(pre.len(), 2, "{pre:?}");
        assert!(!order.exists(), "a PostToolUse group for another tool ran");
        if !with_ids {
            assert!(
                pre.iter().all(|p| p.get("tool_use_id").is_none()),
                "{pre:?}"
            );
            assert!(!post.exists(), "PostToolUse without the variable");
            assert!(!failed.exists(), "PostToolUseFailure without the variable");
            continue;
        }
        assert_eq!(pre[0]["tool_use_id"], "fake-tu-1");
        assert_eq!(pre[1]["tool_use_id"], "fake-tu-2");
        let post = payloads(&post);
        assert_eq!(post.len(), 1, "only the call that exited 0: {post:?}");
        let p = &post[0];
        assert_eq!(p["hook_event_name"], "PostToolUse");
        assert_eq!(p["tool_name"], "Bash");
        assert_eq!(p["tool_use_id"], "fake-tu-1");
        assert_eq!(p["tool_input"], json!({"command": "echo one"}));
        assert_eq!(
            p["tool_response"],
            json!({"stdout": "one\n", "exit_code": 0})
        );
        assert!(p["session_id"].is_string() && p["cwd"].is_string(), "{p}");
        let failed = payloads(&failed);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert_eq!(failed[0]["tool_use_id"], "fake-tu-2");
    }
}

/// Milestone 9.18 decision 14 (rulings C14, C15): as the real CLI does (9.14 fact F4),
/// a `bash` call that exits non-zero fires `PostToolUseFailure` with `error: "Exit code
/// <n>"`, `is_interrupt: false` and `duration_ms`, and no `PostToolUse`; a call that
/// exits 0 still fires `PostToolUse`. Both only with `FAKE_AGENT_TOOL_IDS=1`.
#[test]
fn a_failed_bash_step_fires_post_tool_use_failure() {
    let dir = tempdir();
    let (post, failed) = (dir.path().join("post"), dir.path().join("failed"));
    let settings = json!({"hooks": {
        "PostToolUse": [group("Bash", &[record(&post)])],
        "PostToolUseFailure": [group("Bash", &[record(&failed)])],
    }});
    let steps = [
        json!({"bash": {"cmd": "echo bad >&2; exit 3"}}),
        json!({"bash": {"cmd": "true"}}),
    ];
    let one = Path::new("1");
    let (_, log) = run_bash_with(
        dir.path(),
        Some(&settings),
        &steps,
        &[("FAKE_AGENT_TOOL_IDS", one)],
    );
    assert_eq!(log.len(), 2, "{log:?}");

    let failed = payloads(&failed);
    assert_eq!(failed.len(), 1, "{failed:?}");
    let f = &failed[0];
    assert_eq!(f["hook_event_name"], "PostToolUseFailure");
    assert_eq!(f["tool_name"], "Bash");
    assert_eq!(f["tool_use_id"], "fake-tu-1");
    assert_eq!(f["tool_input"], json!({"command": "echo bad >&2; exit 3"}));
    assert_eq!(f["error"], "Exit code 3");
    assert_eq!(f["is_interrupt"], false);
    assert!(f["duration_ms"].is_u64(), "{f}");
    assert!(f.get("tool_response").is_none(), "{f}");
    assert!(f["session_id"].is_string() && f["cwd"].is_string(), "{f}");

    let post = payloads(&post);
    assert_eq!(
        post.len(),
        1,
        "no PostToolUse for the failed call: {post:?}"
    );
    assert_eq!(post[0]["tool_use_id"], "fake-tu-2");
    assert_eq!(post[0]["hook_event_name"], "PostToolUse");
}

/// Milestone 9.14 (Interfaces, `fake-agent`): a hook entry's `timeout` (seconds)
/// replaces `STEP_TIMEOUT` (5 s, `fake-agent/src/main.rs`) for that hook. A hook that
/// sleeps 6 s and then rewrites the command: with `"timeout": 8` its rewrite wins; with
/// none, `STEP_TIMEOUT` cuts it and the original command runs. Timing: 6 s is past 5 s
/// by a whole second, so the cut cannot be missed; the 8 s case needs the hook's
/// `/bin/sh` spawn and `sleep 6` to end within 8 s (2 s for a spawn of `sh` and
/// `sleep`, milliseconds, `docs/timing-budgets.md`); `RUN` (20 s) bounds the session.
#[test]
fn a_hook_timeout_replaces_the_step_timeout() {
    for timeout in [Some(8), None] {
        let dir = tempdir();
        let output = json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "updatedInput": {"command": "echo late"},
        }});
        let command = format!("cat > /dev/null; sleep 6; printf '%s\\n' '{output}'");
        let mut entry = json!({"type": "command", "command": command});
        if let Some(secs) = timeout {
            entry["timeout"] = json!(secs);
        }
        let settings = json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [entry]}]}});
        let (_, log) = run_bash(
            dir.path(),
            Some(&settings),
            &[json!({"bash": {"cmd": "echo original"}})],
        );
        let ran = if timeout.is_some() {
            "echo late"
        } else {
            "echo original"
        };
        assert_eq!(log[0]["ran"], ran, "timeout {timeout:?}: {log:?}");
    }
}
