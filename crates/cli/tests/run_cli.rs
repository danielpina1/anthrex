//! Milestone 8a, task 23: the `anthrex run` commands, driven as the real binary against
//! the run harness's isolated daemon (`fake-agent` as both runtimes).

mod support;

use proto::{RunRequest, RunState, RunsSnapshot, TaskState};
use serde_json::json;
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// One `anthrex run …` outcome.
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

/// `anthrex run <args> --dir <repo>` with `input` on stdin.
fn run(h: &RunHarness, args: &[&str], input: &str) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--dir", &repo]);
    let output = h.anthrex_input(&all, input);
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// `run start --plan <plan>` plus `flags`; asserts it started and returns the id.
fn start(h: &RunHarness, plan_toml: &str, flags: &[&str]) -> (String, Out) {
    let plan = h.plan(plan_toml).display().to_string();
    let mut args = vec!["start", "--plan", plan.as_str()];
    args.extend_from_slice(flags);
    let out = run(h, &args, "");
    assert_eq!(out.code, 0, "start failed: {}{}", out.stderr, h.log_tail());
    let id = out.stdout.trim().to_string();
    assert!(!id.is_empty() && !id.contains('\n'), "{:?}", out.stdout);
    (id, out)
}

fn ok(out: &Out) {
    assert_eq!(
        out.code, 0,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
}

/// Exit 1 with `message` as the whole of stderr's last line.
fn refused_with(out: &Out, message: &str) {
    assert_eq!(
        out.code, 1,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert_eq!(out.stderr.lines().last(), Some(message), "{}", out.stderr);
}

/// A worker that commits and then waits for a message that never comes: the run stays
/// `running`.
fn waiting_worker(h: &RunHarness) {
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            json!({"read_message": {"expect": "never"}}),
        ],
    );
}

#[test]
fn help_lists_the_run_commands_and_hides_mcp() {
    let dir = support::tempdir();
    let help = support::isolated_command(dir.path(), &["run", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&help.stdout);
    let commands = [
        "start", "status", "approve", "reject", "edit", "retry", "override", "cancel", "resume",
        "accept", "discard",
    ];
    for command in commands {
        assert!(
            text.lines()
                .any(|l| l.trim_start().starts_with(&format!("{command} "))),
            "{command} missing from:\n{text}"
        );
    }
    let help = support::isolated_command(dir.path(), &["--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(
        text.lines().any(|l| l.trim_start().starts_with("run ")),
        "{text}"
    );
    assert!(
        !text.lines().any(|l| l.trim_start().starts_with("mcp")),
        "{text}"
    );
}

#[test]
fn start_approve_status_accept() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let (id, out) = start(&h, &plan("", &[task("t1", &["a.txt"], "")]), &[]);
    assert!(
        out.stderr
            .contains(&format!("approve with: anthrex run approve {id}\n")),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains(&format!("{id}  awaiting_approval  0/1 merged  base main@"))
    );
    assert!(
        out.stderr.contains("  ID   SIZE MODE   STATE"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("  t1   S    check  queued "),
        "{}",
        out.stderr
    );

    // By a suffix of the id.
    let suffix = &id[id.len() - 4..];
    ok(&run(&h, &["approve", suffix], ""));

    let out = run(&h, &["status", &id, "--json"], "");
    ok(&out);
    let snapshot: RunsSnapshot = serde_json::from_str(&out.stdout).expect("a RunsSnapshot");
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.runs[0].run_id, id);

    let done = h.wait_run(&id, complete, RUN_WAIT);
    let out = run(&h, &["status"], "");
    ok(&out);
    assert!(
        out.stdout
            .starts_with(&format!("{id}  complete  1/1 merged  ")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\n  t1   S    check  merged "),
        "{}",
        out.stdout
    );

    // A "no" leaves everything as it was.
    let out = run(&h, &["accept", &id], "n\n");
    let question = format!(
        "merge anthrex/{id}/integration into main in {}? [y/N] ",
        done.root.display()
    );
    assert!(out.stderr.starts_with(&question), "{}", out.stderr);
    refused_with(&out, &format!("{question}not merged"));
    assert_eq!(h.run(&id).unwrap().state, RunState::Complete);

    ok(&run(&h, &["accept", &id, "--yes"], ""));
    assert_eq!(h.run(&id).unwrap().state, RunState::Accepted);
    assert_eq!(h.git(&["show", "main:a.txt"]), "a");
    assert!(no_run_branches(&h.repo));
}

#[test]
fn start_prints_the_protected_warning() {
    let h = RunHarness::with_repo("", &[], true, &[("AGENTS.md", "rules\n")]);
    let (id, out) = start(&h, &plan("", &[task("t1", &["**"], "")]), &["--yes"]);
    let warning = "warning: t1: owns ** covers protected AGENTS.md; name it exactly in owns if this task must change it (rule 6.protected)\n";
    assert!(out.stderr.starts_with(warning), "{}", out.stderr);
    assert!(
        out.stderr
            .ends_with(&format!("watch with: anthrex run status {id}\n")),
        "{}",
        out.stderr
    );
    assert!(h.run(&id).is_some(), "the run started");
}

#[test]
fn reject_needs_the_id() {
    let h = RunHarness::new("");
    let (id, _) = start(&h, &plan("", &[task("t1", &["a.txt"], "")]), &[]);

    let out = run(&h, &["reject", &id], "not-the-id\n");
    assert!(
        out.stderr
            .starts_with(&format!("reject run {id}: remove its worktrees and delete its branches?\ntype the run id to confirm: ")),
        "{}",
        out.stderr
    );
    refused_with(
        &out,
        "type the run id to confirm: confirmation does not match the run id",
    );
    refused_with(
        &run(&h, &["reject", &id, "--confirm", "other"], ""),
        "confirmation does not match the run id",
    );
    assert_eq!(h.run(&id).unwrap().state, RunState::AwaitingApproval);

    ok(&run(&h, &["reject", &id], &format!("{id}\n")));
    h.wait_run(&id, |r| r.state == RunState::Discarded, RUN_WAIT);
    assert!(no_run_branches(&h.repo));
}

#[test]
fn discard_keeps_salvage_refs() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let (id, _) = start(&h, &plan("", &[task("t1", &["a.txt"], "")]), &["--yes"]);
    let done = h.wait_run(&id, complete, RUN_WAIT);
    // Work left in the integration worktree is salvaged, not lost.
    let integration = integration(t(&done, "t1"));
    std::fs::write(integration.join("left.txt"), "left behind\n").unwrap();

    let out = run(&h, &["discard", &id], "nope\n");
    assert!(
        out.stderr.starts_with(&format!(
            "discard run {id}: remove its worktrees and delete its branches?\ntype the run id to confirm: "
        )),
        "{}",
        out.stderr
    );
    assert_eq!(out.code, 1);
    refused_with(
        &run(&h, &["discard", &id, "--confirm", "nope"], ""),
        "confirmation does not match the run id",
    );
    assert_eq!(h.run(&id).unwrap().state, RunState::Complete);

    ok(&run(&h, &["discard", &id], &format!("{id}\n")));
    let run_info = h.run(&id).unwrap();
    assert_eq!(run_info.state, RunState::Discarded);
    assert!(no_run_branches(&h.repo));
    assert!(!integration.exists());
    let salvage = h.git(&[
        "for-each-ref",
        "--format=%(refname)",
        &format!("refs/anthrex/salvage/{id}/"),
    ]);
    assert!(!salvage.is_empty(), "no salvage ref kept");
    let salvaged = salvage.lines().next().unwrap();
    assert_eq!(
        h.git(&["show", &format!("{salvaged}:left.txt")]),
        "left behind"
    );
    assert!(run_info.report_path.exists(), "the run's data is kept");
}

#[test]
fn edit_from_a_file() {
    let h = RunHarness::new("");
    let (id, _) = start(
        &h,
        &plan(
            "",
            &[task("t1", &["a.txt"], ""), task("t2", &["b.txt"], "")],
        ),
        &[],
    );
    let bad = h.dir.path().join("bad.toml");
    std::fs::write(&bad, "[[edit]]\nop = \"no_such_edit\"\n").unwrap();
    let out = run(&h, &["edit", &id, "--file", &bad.display().to_string()], "");
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.starts_with(&format!("{}: ", bad.display())),
        "{}",
        out.stderr
    );

    let edits = h.dir.path().join("edits.toml");
    std::fs::write(&edits, "[[edit]]\nop = \"cancel_task\"\ntask_id = \"t2\"\n").unwrap();
    ok(&run(
        &h,
        &["edit", &id, "--file", &edits.display().to_string()],
        "",
    ));
    let info = h.run(&id).unwrap();
    assert_eq!(t(&info, "t2").state, TaskState::Cancelled);
    assert_eq!(t(&info, "t1").state, TaskState::Queued);
}

/// Each refusal's text is the daemon's own: the same request sent raw is refused with
/// the same words.
#[test]
fn retry_and_override_reach_the_daemon() {
    let h = RunHarness::new("");
    let (id, _) = start(&h, &plan("", &[task("t1", &["a.txt"], "")]), &[]);
    let raw = |request| match h.request(request) {
        proto::RunReply::Refused { message, .. } => message,
        other => panic!("expected a refusal, got {other:?}"),
    };
    let retry = raw(RunRequest::Retry {
        run_id: id.clone(),
        task_id: "t1".into(),
    });
    refused_with(&run(&h, &["retry", &id, "t1"], ""), &retry);
    let over = raw(RunRequest::Override {
        run_id: id.clone(),
        task_id: "t1".into(),
        reason: "because".into(),
    });
    refused_with(
        &run(&h, &["override", &id, "t1", "--reason", "because"], ""),
        &over,
    );
    // An unknown run never reaches a request.
    refused_with(
        &run(&h, &["retry", "zzzz", "t1"], ""),
        "no run matches 'zzzz'",
    );
}

#[test]
fn trust_project_flag_reaches_the_daemon() {
    let hooked = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"true"}]}]}}"#;
    let h = RunHarness::with_repo(
        "",
        &[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")],
        true,
        &[(".claude/settings.json", hooked)],
    );
    let plan_toml = plan("", &[task("t1", &["a.txt"], "")]);
    let plan_path = h.plan(&plan_toml).display().to_string();
    refused_with(
        &run(&h, &["start", "--plan", &plan_path], ""),
        "this repository has project settings that headless Claude sessions would run without asking: .claude/settings.json; review them, then start again with --trust-project",
    );
    let (id, _) = start(&h, &plan_toml, &["--trust-project"]);
    assert_eq!(
        h.run(&id).unwrap().trusted_project,
        vec![".claude/settings.json".to_string()]
    );
}

#[test]
fn accept_of_a_running_run_is_refused() {
    let h = RunHarness::new("");
    waiting_worker(&h);
    let (id, _) = start(&h, &plan("", &[task("t1", &["a.txt"], "")]), &["--yes"]);
    h.wait_run(&id, |r| r.state == RunState::Running, RUN_WAIT);
    refused_with(
        &run(&h, &["accept", &id, "--yes"], ""),
        &format!("run {id} is running; accept applies only to a complete run"),
    );
    assert_eq!(h.run(&id).unwrap().state, RunState::Running);
}

/// Decision 20 through the CLI: a moved base is listed; `--yes` does not answer it,
/// `--base` naming another head is refused, and a typed yes to both questions merges.
#[test]
fn accept_onto_a_moved_base_lists_it_and_needs_its_own_yes() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let (id, _) = start(&h, &plan("", &[task("t1", &["a.txt"], "")]), &["--yes"]);
    h.wait_run(&id, complete, RUN_WAIT);
    let from = h.git(&["rev-parse", "HEAD"]);
    std::fs::write(h.repo.join("other.txt"), "other\n").unwrap();
    h.git(&["add", "other.txt"]);
    h.git(&["commit", "-qm", "base moves on"]);
    let to = h.git(&["rev-parse", "HEAD"]);
    let listing = format!(
        "main moved since the run started ({}..{}, 1 commits):\n  {} Test User: base moves on\n",
        &from[..7],
        &to[..7],
        &to[..7]
    );
    let question = format!(
        "merge onto main at {} including these commits? [y/N] ",
        &to[..7]
    );

    let out = run(&h, &["accept", &id, "--yes"], "");
    assert!(
        out.stderr.starts_with(&format!("{listing}{question}")),
        "{}",
        out.stderr
    );
    refused_with(&out, &format!("{question}not merged"));
    let out = run(&h, &["accept", &id, "--yes", "--base", &from], "");
    refused_with(
        &out,
        &format!("--base {from} is not the listed head {to}; not merged"),
    );
    assert_eq!(h.git(&["rev-parse", "main"]), to);
    assert_eq!(h.run(&id).unwrap().state, RunState::Complete);

    let out = run(&h, &["accept", &id], "y\ny\n");
    ok(&out);
    assert!(out.stderr.contains(&listing), "{}", out.stderr);
    assert_eq!(h.run(&id).unwrap().state, RunState::Accepted);
    assert!(
        h.git(&["log", "-1", "--format=%P", "main"])
            .starts_with(&to)
    );
}

/// `--base <listed head>` answers the moved-base question for a script.
#[test]
fn accept_with_the_listed_base_merges() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    let (id, _) = start(&h, &plan("", &[task("t1", &["a.txt"], "")]), &["--yes"]);
    h.wait_run(&id, complete, RUN_WAIT);
    std::fs::write(h.repo.join("other.txt"), "other\n").unwrap();
    h.git(&["add", "other.txt"]);
    h.git(&["commit", "-qm", "base moves on"]);
    let to = h.git(&["rev-parse", "HEAD"]);
    ok(&run(&h, &["accept", &id, "--yes", "--base", &to], ""));
    assert_eq!(h.run(&id).unwrap().state, RunState::Accepted);
    assert_eq!(h.git(&["show", "main:a.txt"]), "a");
}
