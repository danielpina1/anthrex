//! Milestone 8a, task 22: the run engine end to end, through raw socket requests (the
//! `anthrex run` commands arrive in M8a.23), with `fake-agent` as both runtimes. Decision
//! 53's project-settings cases are in `run_e2e_settings.rs`.

mod support;

use std::time::{Duration, Instant};

use proto::{
    AgentRole, Block, ClientMsg, DaemonMsg, DoneSignal, Head, Role, RunRef, RunReply, RunRequest,
    RunState, Runtime, Status, TaskState, TokenUsage, WindowKind,
};
use serde_json::{Value, json};
use support::run_harness::{RUN_WAIT, RunHarness, git_in};
use support::run_plans::*;

#[test]
fn e2e_green_s_task_runs_to_merged() {
    let h = RunHarness::new("");
    let usage = json!({"usage": {"input": 11, "output": 23, "cache_read": 37, "cache_write": 41}});
    h.script(
        "worker-t1-1",
        &[commit("a.txt", "a\n"), usage, done("added a")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let base = h.git(&["rev-parse", "HEAD"]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);

    let run = h.wait_run(
        &id,
        |r| window_of(r, "t1", AgentRole::Worker).is_some(),
        RUN_WAIT,
    );
    let worker = window_of(&run, "t1", AgentRole::Worker).unwrap();
    let info = h
        .windows()
        .into_iter()
        .find(|w| w.id == worker)
        .expect("the worker is listed");
    assert_eq!(info.kind, WindowKind::Headless);
    assert_eq!(
        info.run,
        Some(RunRef {
            run_id: id.clone(),
            task_id: Some("t1".into()),
            role: AgentRole::Worker,
            session: 1,
        })
    );

    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.done_signal, Some(DoneSignal::TaskDone));
    let head = t1.head.clone().expect("the claimed head");
    let merges = h.git(&[
        "log",
        "--merges",
        "--format=%P",
        &format!("anthrex/{id}/integration"),
    ]);
    assert_eq!(merges, format!("{base} {head}"), "one merge of (base, t1)");
    assert!(!t1.worktree.exists(), "the task worktree is removed");
    assert!(!t1.worktree.with_file_name("t1.review").exists());
    assert!(!t1.worktree.with_file_name("t1.proof").exists());
    assert!(integration(t1).exists(), "the integration worktree remains");
    // The report is written at most every 500 ms: waited for, with a deadline (F4).
    report_with(&run, "## t1:");

    let worker_round = t1
        .rounds
        .iter()
        .find(|r| r.role == AgentRole::Worker)
        .unwrap();
    assert_eq!(
        worker_round.usage,
        TokenUsage {
            input: 11,
            output: 23,
            cache_read: 37,
            cache_write: 41
        }
    );
    let stdin = h.io_lines("worker-t1-1", "stdin");
    let first: Value = serde_json::from_str(&stdin[0]).expect("one stream-json envelope");
    assert_eq!(first["type"], "user", "{first}");
    let text = first["message"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("[anthrex] Task t1: Task t1"), "{text}");

    let reviewer_round = t1
        .rounds
        .iter()
        .find(|r| r.role == AgentRole::Reviewer)
        .expect("a reviewer round");
    assert_eq!(reviewer_round.route.runtime, Runtime::Codex);
    let reviewer = reviewer_round.window_id.unwrap();
    let info = until("the reviewer to exit", RUN_WAIT, || {
        h.windows()
            .into_iter()
            .find(|w| w.id == reviewer && w.status == Status::Exited)
    });
    assert_eq!(info.runtime, Runtime::Codex);
    assert_eq!(info.run.unwrap().role, AgentRole::Reviewer);
    until(
        "the reviewer window to go",
        RETIRE_AFTER + Duration::from_secs(5),
        || (!h.windows().iter().any(|w| w.id == reviewer)).then_some(()),
    );
}

#[test]
fn e2e_plan_gate_waits_and_approve_runs() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), false);
    let run = h.run(&id).unwrap();
    assert_eq!(run.state, RunState::AwaitingApproval);
    let path = integration(t(&run, "t1"));
    until("the integration worktree", RUN_WAIT, || {
        path.exists().then_some(())
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        assert_eq!(h.run(&id).unwrap().state, RunState::AwaitingApproval);
        assert!(h.windows().is_empty(), "no session starts at the gate");
        std::thread::sleep(Duration::from_millis(100));
    }
    match h.request(RunRequest::Approve { run_id: id.clone() }) {
        RunReply::Done { request, .. } => assert_eq!(request, "run approve"),
        other => panic!("approve: {other:?}"),
    }
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert_eq!(run.approved_by.as_deref(), Some("user"));
}

#[test]
fn e2e_plan_gate_survives_a_restart() {
    let mut h = RunHarness::new("");
    green_scripts(&h.repo);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), false);
    let path = integration(t(&h.run(&id).unwrap(), "t1"));
    until("the integration worktree", RUN_WAIT, || {
        path.exists().then_some(())
    });
    h.restart_daemon(&[]);
    assert_eq!(h.run(&id).unwrap().state, RunState::AwaitingApproval);
    assert!(matches!(
        h.request(RunRequest::Approve { run_id: id.clone() }),
        RunReply::Done { .. }
    ));
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
}

#[test]
fn e2e_subscribe_pushes_snapshots_with_rising_revisions() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let watcher = h.subscribe();
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    h.wait_run(&id, complete, RUN_WAIT);
    watcher.wait_for(Duration::from_secs(10), |m| {
        matches!(m, DaemonMsg::Run(RunReply::Snapshot(s))
            if s.runs.iter().any(|r| r.run_id == id && r.state == RunState::Complete))
    });
    let snapshots = watcher.snapshots();
    for pair in snapshots.windows(2) {
        assert!(
            pair[1].revision > pair[0].revision,
            "revisions {} then {}",
            pair[0].revision,
            pair[1].revision
        );
    }
    let mut states: Vec<TaskState> = Vec::new();
    for snapshot in &snapshots {
        if let Some(run) = snapshot.runs.iter().find(|r| r.run_id == id) {
            let state = t(run, "t1").state;
            if states.last() != Some(&state) {
                states.push(state);
            }
        }
    }
    let wanted = [
        &[TaskState::Queued, TaskState::Preparing][..],
        &[TaskState::Working],
        &[TaskState::Check],
        &[TaskState::Review],
        &[TaskState::MergeQueue],
        &[TaskState::Merged],
    ];
    let mut rest = states.iter();
    for want in wanted {
        assert!(
            rest.any(|s| want.contains(s)),
            "{want:?} missing, in order, from {states:?}"
        );
    }
}

#[test]
fn e2e_task_worktree_is_watched() {
    let h = RunHarness::with_env("", &[], false);
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            json!({"wait_ms": 2000}),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let watcher = h.watch(None);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let run = h.run(&id).unwrap();
    let path = t(&run, "t1").worktree.clone();
    // Final fix batch F1b: the task worktree is detached, and its probe shows the
    // worker's own commit (read from its private object directory) before the engine
    // has imported it.
    let base = run.base_sha.clone();
    watcher.wait_for(RUN_WAIT, |m| {
        matches!(m, DaemonMsg::Git { root, state: Some(s) }
            if *root == path
                && matches!(&s.head, Head::Detached(short) if !base.starts_with(short.as_str())))
    });
    h.wait_run(&id, complete, RUN_WAIT);
}

#[test]
fn e2e_a_worker_is_watchable_through_its_conversation() {
    // A Codex worker: its conversation is built from its stream (decision 27). A fake
    // Claude session fires no turn hooks of its own (M8a.20's m7), so it would show none.
    let h = RunHarness::new("");
    h.script("worker-t1-1", &[commit("a.txt", "a\n"), done("added a")]);
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], CODEX)]), true);
    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Merged, RUN_WAIT);
    let worker = window_of(&run, "t1", AgentRole::Worker).unwrap();
    let watcher = h.watch(Some(ClientMsg::SubscribeConversation {
        window_id: worker,
        agent_id: None,
        from_rev: None,
    }));
    let DaemonMsg::ConversationSnapshot { conversation, .. } = watcher.wait_for(
        Duration::from_secs(10),
        |m| matches!(m, DaemonMsg::ConversationSnapshot { window_id, .. } if *window_id == worker),
    ) else {
        unreachable!()
    };
    let texts: Vec<&str> = conversation
        .turns
        .iter()
        .filter(|turn| turn.role == Role::User)
        .flat_map(|turn| turn.blocks.iter())
        .filter_map(|b| match b {
            Block::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("[anthrex] Task t1: Task t1")),
        "{conversation:#?}"
    );
    let tools: Vec<&str> = conversation
        .turns
        .iter()
        .flat_map(|turn| turn.blocks.iter())
        .filter_map(|b| match b {
            Block::ToolCall { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        tools.contains(&"mcp__anthrex__task_done"),
        "{conversation:#?}"
    );
}

#[test]
fn e2e_dirty_tree_refuses_to_start() {
    let h = RunHarness::new("");
    std::fs::write(h.repo.join("README"), "changed\n").unwrap();
    let message = refused(h.start_reply(
        &h.repo,
        &plan("", &[task("t1", &["a.txt"], "")]),
        true,
        false,
    ));
    assert!(
        message.ends_with("has uncommitted changes; commit or stash them first"),
        "{message}"
    );
    assert!(no_run_branches(&h.repo));
    assert!(h.snapshot().runs.is_empty());
}

#[test]
fn e2e_validation_errors_come_back_together() {
    let h = RunHarness::new("");
    let bad_size = task("t1", &["a.txt"], "").replace("size = \"S\"", "size = \"L\"");
    let no_reason = task("t2", &["b.txt"], "").replace("test_mode_reason = \"smoke\"\n", "");
    let message = refused(h.start_reply(&h.repo, &plan("", &[bad_size, no_reason]), true, false));
    let lines: Vec<&str> = message.lines().collect();
    assert!(
        lines.contains(&"task t1: size: L tasks are never executed; split the task (rule 7.2.4)"),
        "{message}"
    );
    assert!(
        lines.contains(&"task t2: test_mode_reason: required when test_mode is check or none"),
        "{message}"
    );
    assert!(no_run_branches(&h.repo));
}

#[test]
fn e2e_protected_file_bounces_then_merges() {
    let h = RunHarness::with_repo("", &[], true, &[("AGENTS.md", "base rules\n")]);
    h.script(
        "worker-t1-1",
        &[
            commit("src/a.rs", "fn a() {}\n"),
            commit("AGENTS.md", "changed rules\n"),
            done_expecting_error(),
            sh(
                "printf '%s' \"$FAKE_AGENT_RESULT\" | grep -q 'configures or instructs future agents' && git checkout HEAD~1 -- AGENTS.md && git commit -qm revert",
            ),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["src/**"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.bounces.done, 1);
    let agents = h.git(&["show", &format!("anthrex/{id}/integration:AGENTS.md")]);
    assert_eq!(agents, "base rules");

    // A task that owns AGENTS.md exactly may change it.
    h.script(
        "worker-t2-1",
        &[commit("AGENTS.md", "new rules\n"), done("rules")],
    );
    h.script("reviewer-t2-1", &[approve()]);
    let id = h.start(&plan("", &[task("t2", &["AGENTS.md"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t2 = t(&run, "t2");
    assert_eq!((t2.state, t2.bounces.done), (TaskState::Merged, 0));
    let agents = h.git(&["show", &format!("anthrex/{id}/integration:AGENTS.md")]);
    assert_eq!(agents, "new rules");
}

#[test]
fn e2e_protected_warning_is_printed_at_start() {
    // The daemon's half: the rule-6.protected note on the task. Printing it on stderr is
    // `anthrex run start`'s (M8a.23).
    let h = RunHarness::with_repo("", &[], true, &[("AGENTS.md", "rules\n")]);
    let id = h.start(&plan("", &[task("t1", &["**"], "")]), false);
    let run = h.run(&id).unwrap();
    assert_eq!(run.state, RunState::AwaitingApproval);
    let note = "owns ** covers protected AGENTS.md; name it exactly in owns if this task must change it (rule 6.protected)";
    assert!(
        t(&run, "t1").notes.iter().any(|n| n == note),
        "{:?}",
        t(&run, "t1").notes
    );
}

#[test]
fn e2e_generated_file_bounces_then_merges() {
    let h = RunHarness::with_repo("", &[], true, &[("Cargo.lock", "version = 3\n")]);
    h.script(
        "worker-t1-1",
        &[
            sh("printf 'a\\n' > a.txt && printf 'changed\\n' >> Cargo.lock && git add -A && git commit -qm work"),
            done_expecting_error(),
            sh("git checkout HEAD~1 -- Cargo.lock && git commit -qm \"revert Cargo.lock\""),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(
        &plan(
            "generated = [\"Cargo.lock\"]",
            &[task("t1", &["a.txt"], "")],
        ),
        true,
    );
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!((t1.bounces.done, t1.rung), (1, 1));
}

/// Ruling T22-minors, m3: a merge candidate whose second ref guard errors (here the
/// check, run in the integration worktree, leaves the base ref pointing at a missing
/// object) blocks the task, and the integration worktree is back on the run branch,
/// not left detached at the candidate.
#[test]
fn e2e_a_failed_merge_candidate_reattaches_the_integration_worktree() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let base = h.git(&["symbolic-ref", "--short", "HEAD"]);
    // Final fix batch F1c (I2): the check is confined and can no longer move the base
    // itself, so it waits at the candidate while this test does, from outside.
    let go = h.dir.path().join("go");
    let check = format!(
        "case \"$PWD\" in */integration) for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1;; esac",
        go.display()
    );
    let plan = plan("", &[task("t1", &["a.txt"], "")])
        .replace("check = \"true\"", &format!("check = {check:?}"));
    let id = h.start(&plan, true);
    let integration = until("the candidate's check", RUN_WAIT, || {
        let run = h.run(&id)?;
        let at = integration(t(&run, "t1"));
        let detached = at.is_dir()
            && git_read(&at, &["symbolic-ref", "-q", "HEAD"]).is_none()
            && git_read(&at, &["rev-parse", "HEAD^2"]).is_some();
        detached.then_some(at)
    });
    let common = h.git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    std::fs::write(
        std::path::Path::new(&common).join(format!("refs/heads/{base}")),
        "1111111111111111111111111111111111111111\n",
    )
    .unwrap();
    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Blocked, RUN_WAIT);
    let block = t(&run, "t1").block.clone().expect("a block");
    assert!(block.text.contains("could not merge"), "{block:?}");
    let head = git_in(&integration, &["symbolic-ref", "-q", "HEAD"]);
    assert_eq!(head, format!("refs/heads/anthrex/{id}/integration"));
}

/// M8a.24 fix round 1: a daemon that binds its socket after the harness stopped waiting
/// (a slow start, simulated by a zero wait) is still the harness's, and dropping the
/// harness stops it. Before this, `Drop` saw no socket and left the daemon running.
#[test]
fn a_slow_starting_daemon_does_not_outlive_its_harness() {
    let (h, started) = RunHarness::try_started("", Duration::ZERO);
    assert!(started.is_err(), "{started:?}");
    let pid = h.daemon_pid().expect("the harness owns its daemon");
    let data = h.data();
    // Linux CI read an empty /proc/<pid>/environ right after the spawn in 2 of 4 runs:
    // wait (bounded) for the process to show its environment.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !daemon_of(pid, &data) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        daemon_of(pid, &data),
        "the daemon is starting: {}",
        daemon_diagnosis(pid, &data)
    );
    drop(h);
    assert!(
        !daemon_of(pid, &data),
        "the daemon of {} survived",
        data.display()
    );
}

/// Whether process `pid` (one pid, never a scan) is a daemon of `data`: its environment
/// names that data directory.
fn daemon_of(pid: u32, data: &std::path::Path) -> bool {
    let wanted = format!("ANTHREX_DATA_DIR={}", data.display());
    // Linux: `ps -E` is BSD-only; read this one pid's environment from /proc.
    if cfg!(target_os = "linux") {
        return std::fs::read(format!("/proc/{pid}/environ"))
            .map(|env| env.split(|b| *b == 0).any(|v| v == wanted.as_bytes()))
            .unwrap_or(false);
    }
    let output = std::process::Command::new("ps")
        .args(["-E", "-o", "command=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    output.status.success() && String::from_utf8_lossy(&output.stdout).contains(&wanted)
}

/// What one pid's process looks like, for a failed `daemon_of`: its anthrex variables
/// (never other values) and the data directory's files.
fn daemon_diagnosis(pid: u32, data: &std::path::Path) -> String {
    let env = if cfg!(target_os = "linux") {
        match std::fs::read(format!("/proc/{pid}/environ")) {
            Ok(env) => env
                .split(|b| *b == 0)
                .map(|v| String::from_utf8_lossy(v).into_owned())
                .filter(|v| v.starts_with("ANTHREX_"))
                .collect::<Vec<_>>()
                .join(" "),
            Err(e) => format!("no /proc/{pid}/environ: {e}"),
        }
    } else {
        String::from("(not Linux)")
    };
    let files: Vec<String> = std::fs::read_dir(data)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let log = std::fs::read_to_string(data.join("daemon.stderr.log")).unwrap_or_default();
    let out = data
        .parent()
        .map(|dir| std::fs::read_to_string(dir.join("daemon.out")).unwrap_or_default())
        .unwrap_or_default();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    format!(
        "env [{env}] data {} files {files:?} stderr {log:?} out {out:?} stat {stat:?}",
        data.display()
    )
}

/// M8a.24 fix round 2 (N2): an owned daemon process dropped without a completed stop
/// (here a panic unwinds past it) is killed through its own handle. The child is a
/// distinctive `sleep`, so the check reads that one pid's argv.
#[test]
fn a_dropped_daemon_process_is_killed() {
    let dir = support::tempdir();
    let mut command = std::process::Command::new("sleep");
    command.arg("3737");
    let mut pid = 0;
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let daemon =
            support::run_daemon::DaemonProcess::spawn(&mut command, &dir.path().join("out"));
        pid = daemon.pid();
        panic!("the stop timed out");
    }));
    assert!(unwound.is_err());
    let output = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("sleep 3737"),
        "pid {pid} still runs: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
