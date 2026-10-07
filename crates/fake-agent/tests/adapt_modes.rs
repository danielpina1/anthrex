//! M8b.6, decisions 36 and 37: `fake-agent`'s decider mode, scout scripts, the `bash`
//! step and every hook group, over real pipes and real git repositories. The decider
//! output is read back with M8a's own stream parsers and M8b.5's `answer_from_events`.

mod headless_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use daemon::decider::argv::{claude_decider_args, codex_decider_args};
use daemon::decider::parse::{answer_from_events, parse};
use daemon::decider::schema::schema;
use daemon::decider::{
    BlockedReasonInput, DECIDER_CAPS, DeciderAnswer, DeciderContext, DeciderKind, DeciderRequest,
    prompt,
};
use daemon::headless::argv::CLI_CAPS;
use daemon::headless::{SessionEvent, claude_stream, codex_stream};
use headless_support::*;
use proto::{DeciderMode, Effort, Route, Runtime, Strength, TokenUsage};
use serde_json::{Value, json};

/// How long a hanging decider is watched for output before it is killed: an absence
/// window (a `hang` never writes), so load can only hide a defect, never fail a build.
const HANG_WINDOW: Duration = Duration::from_secs(1);

fn ctx(runtime: Runtime) -> DeciderContext {
    DeciderContext {
        mode: match runtime {
            Runtime::Claude => DeciderMode::Claude,
            _ => DeciderMode::Codex,
        },
        program: "fake-agent".into(),
        route: Route {
            runtime,
            model: "fast-model".into(),
            strength: Strength::Fast,
            effort: Effort::LOW,
        },
        timeout: Duration::from_secs(90),
        cwd: PathBuf::from("/tmp/unused"),
        schema_dir: PathBuf::from("/tmp/unused"),
        caps: CLI_CAPS,
        live: daemon::live_config::LiveSettings::defaults_of(config::Orchestrator::default()),
        data_dir: PathBuf::from("/tmp/unused"),
        bins: ("fake-agent".into(), "fake-agent".into()),
        decider_bin: None,
        launch_gate: daemon::launch::LaunchGate::open_already(),
    }
}

fn blocked_prompt() -> String {
    prompt::render(&DeciderRequest::BlockedReason(BlockedReasonInput {
        task_id: "t1".into(),
        title: "Add a retry to the fetch helper".into(),
        reason: "the linker `cc` is not installed".into(),
    }))
}

/// A decider script `<dir>/<name>.json`.
fn decider_script(dir: &Path, name: &str, script: Value) {
    fs::write(dir.join(format!("{name}.json")), script.to_string()).unwrap();
}

fn claude_args(kind: DeciderKind) -> Vec<String> {
    claude_decider_args(&ctx(Runtime::Claude), &DECIDER_CAPS, &schema(kind))
}

/// One Claude decider call as M8b.7 makes it: the prompt as the one stdin message, then
/// stdin closed.
fn claude_call(args: &[String], prompt: &str, cwd: &Path, dir: &Path) -> Agent {
    let mut agent = Agent::spawn(args, cwd, &[("FAKE_AGENT_DECIDER_DIR", dir)]);
    agent.send(&claude_stream::user_message(prompt, None));
    agent.close_stdin();
    agent
}

fn codex_args(prompt: &str) -> Vec<String> {
    let schema_file = Path::new("/tmp/unused/blocked_reason.json");
    codex_decider_args(&ctx(Runtime::Codex), &DECIDER_CAPS, schema_file, prompt)
}

fn claude_events(agent: &Agent) -> Vec<SessionEvent> {
    let mut stream = claude_stream::ClaudeStream::default();
    agent
        .seen
        .iter()
        .flat_map(|v| stream.parse_line(&v.to_string()))
        .collect()
}

fn codex_events(agent: &Agent) -> Vec<SessionEvent> {
    agent
        .seen
        .iter()
        .flat_map(|v| codex_stream::parse_line(&v.to_string()))
        .collect()
}

fn turn_usage(events: &[SessionEvent]) -> Option<TokenUsage> {
    events.iter().find_map(|e| match e {
        SessionEvent::TurnEnded { usage, .. } => *usage,
        _ => None,
    })
}

fn usage_json() -> Value {
    json!({"input": 11, "output": 22, "cache_read": 33, "cache_write": 44})
}

fn answer() -> Value {
    json!({"kind": "environment", "reason": "cc is not installed"})
}

#[test]
fn decider_mode_answers_in_the_claude_shape() {
    let (work, dir) = (tempdir(), tempdir());
    decider_script(
        dir.path(),
        "blocked_reason-1",
        json!({"answer": answer(), "usage": usage_json()}),
    );
    let args = claude_args(DeciderKind::BlockedReason);
    let mut agent = claude_call(&args, &blocked_prompt(), work.path(), dir.path());
    assert!(agent.wait(RUN).success(), "stderr {}", agent.stderr());

    let events = claude_events(&agent);
    let value = answer_from_events(&events).unwrap();
    assert_eq!(value, answer());
    assert!(matches!(
        parse(DeciderKind::BlockedReason, &value),
        Ok(DeciderAnswer::BlockedReason { .. })
    ));
    let usage = turn_usage(&events).expect("a turn end with usage");
    assert_eq!((usage.input, usage.output), (11, 22));
    assert_eq!((usage.cache_read, usage.cache_write), (33, 44));
    let result = agent.of_type("result").pop().unwrap();
    assert_eq!(result["structured_output"], answer(), "{result}");
}

#[test]
fn decider_mode_answers_in_the_codex_shape() {
    let (work, dir) = (tempdir(), tempdir());
    decider_script(
        dir.path(),
        "blocked_reason-1",
        json!({"answer": answer(), "usage": usage_json()}),
    );
    let args = codex_args(&blocked_prompt());
    let mut agent = Agent::codex(
        &args,
        work.path(),
        &[("FAKE_AGENT_DECIDER_DIR", dir.path())],
    );
    assert!(agent.wait(RUN).success(), "stderr {}", agent.stderr());

    let events = codex_events(&agent);
    assert_eq!(answer_from_events(&events).unwrap(), answer());
    let usage = turn_usage(&events).expect("a turn end with usage");
    assert_eq!((usage.input, usage.output), (11, 22));
    assert_eq!(usage.cache_read, 33);
}

#[test]
fn decider_output_conforms_to_the_fixtures() {
    let (work, dir) = (tempdir(), tempdir());
    decider_script(dir.path(), "blocked_reason-1", json!({"answer": answer()}));
    decider_script(
        dir.path(),
        "blocked_reason-2",
        json!({"text": format!("```json\n{}\n```", answer())}),
    );
    decider_script(
        dir.path(),
        "blocked_reason-3",
        json!({"fail_turn": "rate_limit"}),
    );
    let args = claude_args(DeciderKind::BlockedReason);
    let mut runs = Vec::new();
    for _ in 0..3 {
        let mut agent = claude_call(&args, &blocked_prompt(), work.path(), dir.path());
        agent.wait(RUN);
        runs.push(agent);
    }

    // The structured answer against M8b.1's decider recordings alone.
    let types = assert_conforms_in(&["deciders"], "claude", &runs[0].seen);
    for kind in ["system/init", "assistant", "user", "result/success"] {
        assert!(types.contains(kind), "{kind} not emitted: {types:?}");
    }
    // M8b.1 recorded no decider that answered only in text or failed its turn, so those
    // are checked against the decider and M8a.1's headless recordings merged.
    for run in &runs[1..] {
        assert_conforms_in(&["deciders", "headless"], "claude", &run.seen);
    }
    assert_eq!(
        answer_from_events(&claude_events(&runs[1])).unwrap(),
        answer()
    );
    assert!(runs[2].of_type("result")[0]["is_error"] == json!(true));

    if fixture_files("deciders", "codex").is_empty() {
        println!(
            "skipped the Codex decider shape: no codex-<version>-decider.jsonl fixture \
             (M8b.1 item 2 was not run, controller ruling)"
        );
    } else {
        decider_script(dir.path(), "blocked_reason-4", json!({"answer": answer()}));
        let mut agent = Agent::codex(
            &codex_args(&blocked_prompt()),
            work.path(),
            &[("FAKE_AGENT_DECIDER_DIR", dir.path())],
        );
        agent.wait(RUN);
        assert_conforms_in(&["deciders"], "codex", &agent.seen);
    }
}

#[test]
fn decider_mode_is_chosen_by_the_prompt_not_the_flags() {
    let (work, dir) = (tempdir(), tempdir());
    decider_script(dir.path(), "triage-1", json!({"answer": {"marker": 1}}));
    let args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
    ]
    .map(String::from)
    .to_vec();
    assert!(!args.iter().any(|a| a == "--json-schema"));

    // Flags alone do not make a decider: a plain prompt gets the plain empty turn.
    let mut plain = claude_call(&args, "hello", work.path(), dir.path());
    assert!(plain.wait(RUN).success(), "stderr {}", plain.stderr());
    assert!(!dir.path().join("triage-1.json.claimed").exists());
    assert!(answer_from_events(&claude_events(&plain)).is_err());

    let prompt = "[anthrex decider] triage v1\nDecide.";
    let mut agent = claude_call(&args, prompt, work.path(), dir.path());
    assert!(agent.wait(RUN).success(), "stderr {}", agent.stderr());
    assert_eq!(
        answer_from_events(&claude_events(&agent)).unwrap(),
        json!({"marker": 1})
    );
}

#[test]
fn decider_calls_are_recorded() {
    let (work, dir) = (tempdir(), tempdir());
    decider_script(dir.path(), "blocked_reason-1", json!({"answer": answer()}));
    decider_script(dir.path(), "blocked_reason-2", json!({"answer": answer()}));
    let prompt = blocked_prompt();
    let claude = claude_args(DeciderKind::BlockedReason);
    claude_call(&claude, &prompt, work.path(), dir.path()).wait(RUN);
    let codex = codex_args(&prompt);
    Agent::codex(
        &codex,
        work.path(),
        &[("FAKE_AGENT_DECIDER_DIR", dir.path())],
    )
    .wait(RUN);

    let calls: Vec<Value> = lines(&dir.path().join("calls.jsonl"))
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        calls,
        vec![
            json!({"kind": "blocked_reason", "argv": claude, "prompt": prompt}),
            json!({"kind": "blocked_reason", "argv": codex, "prompt": prompt}),
        ]
    );
}

#[test]
fn decider_without_a_script_exits_2() {
    let (work, dir) = (tempdir(), tempdir());
    let args = claude_args(DeciderKind::BlockedReason);
    let mut agent = claude_call(&args, &blocked_prompt(), work.path(), dir.path());
    assert_eq!(agent.wait(RUN).code(), Some(2));
    assert!(
        agent
            .stderr()
            .contains("fake-agent: no scripted decider answer for blocked_reason"),
        "{}",
        agent.stderr()
    );
    assert!(agent.of_type("result").is_empty(), "{:?}", agent.seen);

    // No FAKE_AGENT_DECIDER_DIR at all.
    let mut bare = Agent::codex(&codex_args(&blocked_prompt()), work.path(), &[]);
    assert_eq!(bare.wait(RUN).code(), Some(2));
    assert!(
        bare.stderr()
            .contains("no scripted decider answer for blocked_reason")
    );
}

#[test]
fn decider_hang_blocks_until_killed() {
    let (work, dir) = (tempdir(), tempdir());
    decider_script(dir.path(), "blocked_reason-1", json!({"hang": true}));
    let args = claude_args(DeciderKind::BlockedReason);
    // Stdin is closed at once, as the daemon's call does; a hang still does not end.
    let mut agent = claude_call(&args, &blocked_prompt(), work.path(), dir.path());

    assert_eq!(agent.next_within(HANG_WINDOW), None, "{:?}", agent.seen);
    assert!(agent.is_running(), "stderr {}", agent.stderr());
    // Only the pid this test spawned.
    agent.signal(libc::SIGKILL);
    let status = agent.wait(RUN);
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    assert!(agent.seen.is_empty(), "{:?}", agent.seen);
}

#[test]
fn decider_scripts_are_claimed_in_order() {
    let (work, dir) = (tempdir(), tempdir());
    for n in [10, 2, 1] {
        decider_script(
            dir.path(),
            &format!("blocked_reason-{n}"),
            json!({"answer": {"n": n}}),
        );
    }
    let args = claude_args(DeciderKind::BlockedReason);
    let mut answers = Vec::new();
    for _ in 0..3 {
        let mut agent = claude_call(&args, &blocked_prompt(), work.path(), dir.path());
        assert!(agent.wait(RUN).success(), "stderr {}", agent.stderr());
        answers.push(answer_from_events(&claude_events(&agent)).unwrap()["n"].clone());
    }
    assert_eq!(answers, [json!(1), json!(2), json!(10)]);
    for n in [1, 2, 10] {
        let claim = dir.path().join(format!("blocked_reason-{n}.json.claimed"));
        assert!(claim.exists(), "{}", claim.display());
    }
    let mut fourth = claude_call(&args, &blocked_prompt(), work.path(), dir.path());
    assert_eq!(fourth.wait(RUN).code(), Some(2));
}

#[test]
fn scout_scripts_are_claimed_by_scout_id() {
    let dir = tempdir();
    let repo = repo(dir.path());
    role_script(
        &repo,
        "scout-onboarding-1",
        &[json!({"print": "scouted"}), json!({"end_turn": {}})],
    );
    role_script(&repo, "scout-area-1", &[json!({"print": "wrong scout"})]);
    let checkout = dir.path().join("checkout");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            "--shared",
            "repo",
            checkout.to_str().unwrap(),
        ],
    );
    assert!(!checkout.join(".git/fake-agent").exists());

    let mut agent = Agent::spawn(&scout_argv("onboarding-1695000000"), &checkout, &[]);
    agent.send(&user_message("scout the repository", "s"));
    agent.until(RUN, is_result);
    agent.close_stdin();
    assert!(agent.wait(RUN).success(), "stderr {}", agent.stderr());

    assert_eq!(texts(&agent), ["scouted"]);
    let claimed = repo.join(".git/fake-agent/scout-onboarding-1.jsonl.claimed");
    assert!(claimed.exists(), "{}", claimed.display());
    assert!(
        !repo
            .join(".git/fake-agent/scout-area-1.jsonl.claimed")
            .exists()
    );
}

#[test]
fn bash_step_applies_updated_input() {
    let dir = tempdir();
    let payloads = dir.path().join("payloads.jsonl");
    let rewrite = dir.path().join("rewrite.sh");
    fs::write(
        &rewrite,
        "cat > /dev/null\nprintf '%s\\n' '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"updatedInput\":{\"command\":\"echo rewritten\"}}}'\n",
    )
    .unwrap();
    let edit_marker = dir.path().join("edit-ran");
    let settings = json!({"hooks": {"PreToolUse": [
        group("", &[format!("{{ cat; echo; }} >> '{}'", payloads.display())]),
        group("Bash", &[format!("sh '{}'", rewrite.display())]),
        group("Edit|Write", &[format!("touch '{}'", edit_marker.display())]),
    ]}});
    let (agent, log) = run_bash(
        dir.path(),
        Some(&settings),
        &[
            json!({"bash": {"cmd": "echo original"}}),
            json!({"sh": {"cmd": "printf %s \"$FAKE_AGENT_RESULT\""}}),
        ],
    );

    assert_eq!(
        log,
        vec![
            json!({"original": "echo original", "ran": "echo rewritten", "exit": 0,
                    "output_lines": ["rewritten"]})
        ]
    );
    // Ruling R-T1-3: the tool_use keeps the original command, as the real CLI's does;
    // only the tool_result shows that the rewritten command ran.
    let pairs = bash_pairs(&agent);
    assert_eq!(
        pairs[0],
        (json!({"command": "echo original"}), json!("rewritten\n"))
    );
    assert_eq!(pairs[1].1, json!("rewritten\n"), "FAKE_AGENT_RESULT");
    let payload: Value = serde_json::from_str(&lines(&payloads)[0]).unwrap();
    assert_eq!(payload["hook_event_name"], "PreToolUse");
    assert_eq!(payload["tool_name"], "Bash");
    assert_eq!(payload["tool_input"], json!({"command": "echo original"}));
    assert!(payload["session_id"].is_string() && payload["cwd"].is_string());
    assert!(!edit_marker.exists(), "a group for another tool ran");
    assert!(
        agent
            .seen
            .iter()
            .all(|v| v.get("hookSpecificOutput").is_none())
    );
    assert_conforms("claude", &agent.seen);
}

#[test]
fn bash_step_without_hooks_runs_the_command() {
    let dir = tempdir();
    let (agent, log) = run_bash(
        dir.path(),
        None,
        &[json!({"bash": {"cmd": "echo hi; echo there; exit 3"}})],
    );

    assert_eq!(
        log,
        vec![json!({"original": "echo hi; echo there; exit 3",
                    "ran": "echo hi; echo there; exit 3", "exit": 3,
                    "output_lines": ["hi", "there"]})]
    );
    let pairs = bash_pairs(&agent);
    assert_eq!(
        pairs,
        [(
            json!({"command": "echo hi; echo there; exit 3"}),
            json!("Exit code 3\nhi\nthere\n")
        )]
    );
}

#[test]
fn every_hook_group_is_discovered_and_hook_keeps_the_first() {
    let dir = tempdir();
    let order = dir.path().join("order");
    let append = |letter: &str| format!("cat > /dev/null; echo {letter} >> '{}'", order.display());
    let settings = json!({"hooks": {"PreToolUse": [
        group("", &[append("a"), append("b")]),
        group("Bash", &[append("c")]),
    ]}});

    // Milestone 3's terminal `hook` step: the first group's first command only.
    let script = write_steps(
        &dir.path().join("terminal.jsonl"),
        &[json!({"hook": "PreToolUse", "payload": {"tool_name": "Bash"}})],
    );
    let args = vec!["--settings".to_string(), settings.to_string()];
    let mut terminal = Agent::spawn(&args, dir.path(), &[("FAKE_AGENT_SCRIPT", &script)]);
    terminal.close_stdin();
    assert!(terminal.wait(RUN).success(), "stderr {}", terminal.stderr());
    assert_eq!(lines(&order), ["a"]);

    // The `bash` step: every matching group, every command, in order.
    fs::remove_file(&order).unwrap();
    run_bash(
        dir.path(),
        Some(&settings),
        &[json!({"bash": {"cmd": "true"}})],
    );
    assert_eq!(lines(&order), ["a", "b", "c"]);
}
