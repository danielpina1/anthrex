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
