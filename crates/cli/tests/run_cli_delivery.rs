//! Milestone 9.2, task M9.2.14: `anthrex run start --delivery`, `run prs`, `run deliver`,
//! `run watch`, `run status`'s delivery lines and the hidden `run fake-github`, through
//! the real binary against an isolated daemon. A `pr` run's daemon runs on `FakeHost`
//! (`PrRig`): nothing here can reach GitHub or run a real `gh`.

mod support;

use daemon::host::fake::{CiRule, FakeGithubCtl};
use daemon::host::{Conclusion, RepoPermission};
use proto::{DeliveryMode, RunInfo, RunState, StagePrInfo};
use serde_json::{Value, json};
use support::run_harness::RunHarness;
use support::run_plans::*;
use support::run_pr::{PR_OPEN_WAIT, PrRig, pr_harness, pr_start};

/// One `anthrex …` outcome.
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn out(output: std::process::Output) -> Out {
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// `anthrex run <args> --dir <repo>`.
fn run(h: &RunHarness, args: &[&str]) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--dir", &repo]);
    out(h.anthrex(&all))
}

fn ok(out: &Out) -> &str {
    assert_eq!(
        out.code, 0,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    &out.stdout
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

/// `run reject <id> --confirm <id>`: a run at the plan gate, gone.
fn reject(h: &RunHarness, id: &str) {
    ok(&run(h, &["reject", id, "--confirm", id]));
}

fn one_task() -> String {
    plan("", &[task("t1", &["a.txt"], "")])
}

/// `run start --plan <one task>` plus `flags`; asserts it started and returns the id.
/// Waited for as a `pr` start (`PR_START_WAIT`, its host preflight included) whatever
/// the flags: the bound of the largest start.
fn start(h: &RunHarness, flags: &[&str]) -> String {
    let (plan, repo) = (h.plan(&one_task()), h.repo.display().to_string());
    let plan = plan.display().to_string();
    let mut args = vec!["run", "start", "--plan", plan.as_str(), "--dir", &repo];
    args.extend_from_slice(flags);
    let out = out(pr_start(h, &args));
    assert_eq!(out.code, 0, "start failed: {}{}", out.stderr, h.log_tail());
    let id = out.stdout.trim().to_string();
    assert!(!id.is_empty() && !id.contains('\n'), "{:?}", out.stdout);
    id
}

/// A `pr` run of one green task whose stage PR is open, with CI green on its head.
fn open_pr(h: &RunHarness, rig: &PrRig) -> (String, RunInfo) {
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    green_scripts(&h.repo);
    let id = start(h, &["--delivery", "pr", "--yes"]);
    rig.wait_stage(h, &id, 1, "/state", &json!("open"));
    rig.wait_stage(h, &id, 1, "/ci", &json!("green"));
    let run = h.wait_run(
        &id,
        |r| r.delivery.as_ref().is_some_and(|d| d.delivering),
        PR_OPEN_WAIT,
    );
    (id, run)
}

fn stage_pr(run: &RunInfo) -> &StagePrInfo {
    run.stages[0].pr.as_ref().expect("stage 1 has a PR")
}

const HEADER: &str = "STAGE  PR     STATE    CI       THREADS                      FIX TASKS";

#[test]
fn run_start_delivery_flag_parses_pr_and_local_and_refuses_others() {
    let (h, _rig) = pr_harness("");
    let plan = h.plan(&one_task()).display().to_string();
    for source in [["--plan", plan.as_str()], ["--goal", "add a"]] {
        let mut args = vec!["start"];
        args.extend_from_slice(&source);
        args.extend_from_slice(&["--delivery", "github"]);
        let out = run(&h, &args);
        assert_eq!(out.code, 1, "{}", out.stderr);
        assert!(
            out.stderr
                .contains("invalid value 'github' for '--delivery <pr|local>'"),
            "{}",
            out.stderr
        );
        assert!(
            out.stderr.contains("[possible values: pr, local]"),
            "{}",
            out.stderr
        );
    }
    assert!(h.snapshot().runs.is_empty(), "nothing started");

    let local = start(&h, &["--delivery", "local"]);
    assert_eq!(h.run(&local).unwrap().delivery, None);
    let pr = start(&h, &["--delivery", "pr"]);
    let delivery = h.run(&pr).unwrap().delivery.expect("a pr run");
    assert_eq!(delivery.mode, DeliveryMode::Pr);
    assert_eq!(delivery.repo, "fake/app");
    assert_eq!(delivery.remote, "origin");
    // No flag: the profile's mode, local without a `[delivery]` table.
    let default = start(&h, &[]);
    assert_eq!(h.run(&default).unwrap().delivery, None);
}

#[test]
fn run_prs_on_a_local_run_prints_the_local_line() {
    let h = RunHarness::new("");
    let id = start(&h, &[]);
    let out = run(&h, &["prs", &id]);
    assert_eq!(
        ok(&out),
        format!("run {id} is delivered locally; it has no pull requests\n")
    );
    let out = run(&h, &["prs", &id, "--json"]);
    assert_eq!(ok(&out), "[]\n");
    refused_with(&run(&h, &["prs", "nope"]), "no run matches 'nope'");
}

#[test]
fn run_prs_prints_the_exact_table() {
    let (h, rig) = pr_harness("");
    // At the plan gate no PR is open yet.
    let waiting = start(&h, &["--delivery", "pr"]);
    let out = run(&h, &["prs", &waiting]);
    assert_eq!(
        ok(&out),
        format!("{HEADER}\n1/1    –      waiting  –        –                            –\n")
    );
    reject(&h, &waiting);

    let (id, run_info) = open_pr(&h, &rig);
    let pr = stage_pr(&run_info);
    let number = format!("#{}", pr.number);
    let out = run(&h, &["prs", &id]);
    assert_eq!(
        ok(&out),
        format!("{HEADER}\n1/1    {number:<7}open     green    0 new, 0 tasked, 0 replied   –\n")
    );
}

#[test]
fn run_prs_json_is_the_stage_pr_info_array() {
    let (h, rig) = pr_harness("");
    let waiting = start(&h, &["--delivery", "pr"]);
    let out = run(&h, &["prs", &waiting, "--json"]);
    let value: Value = serde_json::from_str(ok(&out)).unwrap();
    assert_eq!(value, json!([null]));
    reject(&h, &waiting);

    let (id, _) = open_pr(&h, &rig);
    let out = run(&h, &["prs", &id, "--json"]);
    let printed: Vec<Option<StagePrInfo>> = serde_json::from_str(ok(&out)).unwrap();
    let snapshot = h.run(&id).unwrap();
    let expected: Vec<Option<StagePrInfo>> = snapshot.stages.iter().map(|s| s.pr.clone()).collect();
    // Polls may move the PR's fields between the two reads; its identity may not.
    assert_eq!(printed.len(), expected.len());
    let (printed, expected) = (printed[0].as_ref().unwrap(), expected[0].as_ref().unwrap());
    assert_eq!(
        (printed.number, &printed.url, printed.state, &printed.head),
        (
            expected.number,
            &expected.url,
            expected.state,
            &expected.head
        )
    );
    let fake = rig.wait_pr(printed.number, |_| true);
    assert_eq!(fake.head, format!("anthrex/{id}/stage-1"));
}

#[test]
fn run_deliver_and_watch_print_the_reply_or_the_refusal_with_exit_codes() {
    let (h, rig) = pr_harness("");
    let local = start(&h, &["--delivery", "local"]);
    let pr_only =
        format!("run {local} delivers locally; run deliver and run watch apply to pr mode");
    refused_with(&run(&h, &["deliver", &local, "--stage", "1"]), &pr_only);
    refused_with(&run(&h, &["watch", &local, "--off"]), &pr_only);
    reject(&h, &local);

    let waiting = start(&h, &["--delivery", "pr"]);
    let out = run(&h, &["watch", &waiting, "--off"]);
    assert_eq!(
        ok(&out),
        format!(
            "stopped watching run {waiting}'s pull requests; anthrex run watch {waiting} --on resumes\n"
        )
    );
    assert!(!h.run(&waiting).unwrap().delivery.unwrap().watching);
    let out = run(&h, &["watch", &waiting, "--on"]);
    assert_eq!(
        ok(&out),
        format!("watching run {waiting}'s pull requests\n")
    );
    assert!(h.run(&waiting).unwrap().delivery.unwrap().watching);
    refused_with(
        &run(&h, &["deliver", &waiting, "--stage", "1"]),
        &format!("run {waiting} is awaiting_approval"),
    );
    // Decisions 38-39: the daemon's refusals, printed as they are.
    refused_with(
        &run(&h, &["accept", &waiting, "--yes"]),
        &format!(
            "run {waiting} is delivered by pull request; merge its pull requests on GitHub (anthrex never merges)"
        ),
    );
    let discard = format!(
        "run {waiting} is delivered by pull request; its branches back its pull requests, so discard is refused (anthrex run cancel stops its agents)"
    );
    refused_with(&run(&h, &["discard", &waiting]), &discard);
    refused_with(
        &run(&h, &["discard", &waiting, "--confirm", &waiting]),
        &discard,
    );
    reject(&h, &waiting);

    let (id, run_info) = open_pr(&h, &rig);
    let url = stage_pr(&run_info).url.clone();
    let out = run(&h, &["deliver", &id, "--stage", "1"]);
    assert_eq!(ok(&out), format!("stage 1's PR is already open: {url}\n"));
    let out = run(&h, &["deliver", &id, "--stage", "2"]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert!(
        out.stderr.starts_with("stage 2 is not ready: "),
        "{}",
        out.stderr
    );
    refused_with(
        &run(&h, &["deliver", "nope", "--stage", "1"]),
        "no run matches 'nope'",
    );
}

#[test]
fn run_watch_needs_exactly_one_of_off_and_on() {
    let h = RunHarness::new("");
    let id = start(&h, &[]);
    for args in [
        vec!["watch", id.as_str()],
        vec!["watch", id.as_str(), "--off", "--on"],
    ] {
        let out = run(&h, &args);
        assert_eq!(out.code, 2, "{args:?}: {}", out.stderr);
        assert!(
            out.stderr.contains("--off") && out.stderr.contains("--on"),
            "{args:?}: {}",
            out.stderr
        );
        assert!(out.stdout.is_empty(), "{}", out.stdout);
    }
    // Exit code 2 is clap's usage error, which ends the process before it connects to
    // the daemon (a refusal from the daemon exits 1); the run is untouched.
    assert_eq!(h.run(&id).unwrap().state, RunState::AwaitingApproval);
    let out = run(&h, &["deliver", &id]);
    assert_eq!(out.code, 2, "--stage is required: {}", out.stderr);
}

#[test]
fn run_status_shows_the_delivery_and_prs_lines() {
    let (h, rig) = pr_harness("");
    let local = start(&h, &[]);
    let out = run(&h, &["status", &local]);
    assert!(!ok(&out).contains("delivery:"), "{}", out.stdout);
    reject(&h, &local);

    let waiting = start(&h, &["--delivery", "pr"]);
    let out = run(&h, &["status", &waiting]);
    let text = ok(&out);
    assert!(
        text.contains("\n  delivery: pr to origin (fake/app), watching every 1 s\n"),
        "{text}"
    );
    assert!(!text.contains("  prs:"), "no PR yet: {text}");
    run(&h, &["watch", &waiting, "--off"]);
    let out = run(&h, &["status", &waiting]);
    assert!(
        ok(&out).contains("\n  delivery: pr to origin (fake/app), not watching\n"),
        "{}",
        out.stdout
    );
    reject(&h, &waiting);

    let (id, run_info) = open_pr(&h, &rig);
    let number = stage_pr(&run_info).number;
    let out = run(&h, &["status", &id]);
    let text = ok(&out);
    let header = text.lines().next().unwrap();
    assert!(
        header.starts_with(&format!("{id}  running (delivering)  1/1 merged")),
        "{text}"
    );
    assert!(
        text.contains(&format!("\n  prs: #{number} open (ci green)\n")),
        "{text}"
    );
}

#[test]
fn run_fake_github_is_hidden_from_help() {
    let h = RunHarness::new("");
    let help = h.anthrex(&["run", "--help"]);
    let text = String::from_utf8_lossy(&help.stdout);
    for listed in ["prs ", "deliver ", "watch "] {
        assert!(
            text.lines().any(|l| l.trim_start().starts_with(listed)),
            "{listed} missing from:\n{text}"
        );
    }
    assert!(!text.contains("fake-github"), "{text}");

    // It works on a fake GitHub's directory, and only there.
    let dir = h.dir.path().join("github");
    std::fs::create_dir_all(&dir).unwrap();
    let bare_in = |at: &std::path::Path| {
        let path = at.display().to_string();
        support::run_harness::git_in(&h.repo, &["init", "-q", "--bare", &path]);
        path
    };
    let bare = bare_in(&dir.join("remote.git"));
    let fake = |args: &[&str]| {
        let d = dir.display().to_string();
        let mut all = vec!["run", "fake-github", "--dir", d.as_str()];
        all.extend_from_slice(args);
        out(h.anthrex(&all))
    };
    // Ruling m2: only a bare repository under --dir backs the fake.
    let outside = bare_in(&h.dir.path().join("outside.git"));
    let plain = dir.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let plain = plain.display().to_string();
    let repo = h.repo.display().to_string();
    let objects = format!("{bare}/objects");
    for path in [&outside, &plain, &repo, &objects] {
        let refused = fake(&["create-repo", "fake", "app", path]);
        assert_eq!(refused.code, 1, "{path}: {}", refused.stderr);
        assert!(
            refused.stderr.contains("is not a bare repository under"),
            "{path}: {}",
            refused.stderr
        );
    }
    assert!(!dir.join("github.json").exists(), "nothing was written");
    ok(&fake(&["create-repo", "fake", "app", &bare]));
    ok(&fake(&["log-in", "github.com"]));
    ok(&fake(&["set-permission", "tester", "write"]));
    let rules = json!([{"check": "build", "fail_if": null, "conclusion": "failure",
        "failing_tests": ["t::a"], "log": "boom", "times": 1}]);
    ok(&fake(&["set-ci", &rules.to_string()]));
    assert_eq!(ok(&fake(&["prs"])), "[]\n");
    assert_eq!(ok(&fake(&["forbidden"])), "[]\n");
    let out = fake(&["close", "7"]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert!(out.stderr.contains("no pull request #7"), "{}", out.stderr);

    let ctl = FakeGithubCtl::open(&dir);
    assert!(ctl.prs().is_empty());
    assert!(ctl.forbidden().is_empty());
    assert!(ctl.calls().is_empty(), "no gh call was made");
    let state: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("github.json")).unwrap()).unwrap();
    assert_eq!(state["repo"]["owner"], "fake");
    assert_eq!(state["logged_in"], json!(["github.com"]));
    assert_eq!(
        state["permissions"]["tester"],
        serde_json::to_value(RepoPermission::Write).unwrap()
    );
    assert_eq!(state["ci"][0]["check"], "build");
    assert!(
        h.snapshot().runs.is_empty(),
        "the daemon was not asked anything"
    );
}

#[test]
fn run_fake_github_refuses_without_dir() {
    let h = RunHarness::new("");
    let out = h.anthrex(&["run", "fake-github", "prs"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--dir <DIR>"), "{stderr}");

    let missing = h.dir.path().join("no-such-github");
    let d = missing.display().to_string();
    for args in [vec!["prs"], vec!["log-in", "github.com"]] {
        let mut all = vec!["run", "fake-github", "--dir", d.as_str()];
        all.extend_from_slice(&args);
        let out = h.anthrex(&all);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stderr.trim_end(),
            format!("fake-github: {d} is not a directory"),
        );
    }
    assert!(!missing.exists(), "nothing was created");
}

/// Task M9.2.14 fix round 1 (m1): what the CLI prints drops the hidden format
/// characters a planted text carries: a goal with a bidi override and a zero-width
/// joiner in `run status` (text and JSON), and a refusal that echoes the argument.
#[test]
fn run_output_drops_hidden_format_characters() {
    let h = RunHarness::new("");
    let toml = one_task().replace("goal = \"Add a\"", "goal = \"Add a\\u202Ecod.exe\\u200Dx\"");
    let path = h.plan(&toml).display().to_string();
    let id = out(pr_start(
        &h,
        &[
            "run",
            "start",
            "--plan",
            &path,
            "--dir",
            &h.repo.display().to_string(),
        ],
    ));
    let id = ok(&id).trim().to_string();
    let goal = h.run(&id).unwrap().goal;
    assert!(
        goal.contains('\u{202E}') && goal.contains('\u{200D}'),
        "planted: {goal:?}"
    );
    let hidden = |text: &str| text.contains('\u{202E}') || text.contains('\u{200D}');
    for args in [
        vec!["status", id.as_str()],
        vec!["status", id.as_str(), "--json"],
    ] {
        let text = run(&h, &args);
        assert!(
            ok(&text).contains("Add acod.exex"),
            "{args:?}: {}",
            text.stdout
        );
        assert!(!hidden(&text.stdout), "{args:?}: {:?}", text.stdout);
    }
    let echoed = run(&h, &["prs", "\u{202E}nope\u{200D}"]);
    refused_with(&echoed, "no run matches 'nope'");
}
