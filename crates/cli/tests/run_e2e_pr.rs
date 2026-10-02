//! Milestone 9.2, task M9.2.16: stacked-PR delivery end to end, through the real binary
//! and a real daemon on `/tmp` paths, against `FakeHost` (`PrRig`): stage PRs opening in
//! order, preflight's refusals, the refusals of `pr` mode, cancel, and local mode making
//! no host call. Nothing here can reach GitHub or run a real `gh`; every `PrRig` drop
//! asserts `forbidden.jsonl` is empty. The CI scenarios are in `run_e2e_pr_ci.rs`.

mod support;

use daemon::host::Conclusion;
use daemon::host::fake::{CiRule, MergeMethodArg};
use proto::{PrState, RunInfo, RunState};
use serde_json::json;
use support::run_harness::{FINISH_WAIT, RunHarness, git_in};
use support::run_plans::*;
use support::run_pr::*;

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

/// `run start --plan <toml> --delivery <mode> --yes`, waited for as a `pr` start.
fn start_out(h: &RunHarness, toml: &str, mode: &str) -> Out {
    let plan = h.plan(toml).display().to_string();
    let repo = h.repo.display().to_string();
    let args = [
        "run",
        "start",
        "--plan",
        &plan,
        "--dir",
        &repo,
        "--delivery",
        mode,
        "--yes",
    ];
    out(pr_start(h, &args))
}

/// [`start_out`] that must start; the run id.
fn start(h: &RunHarness, toml: &str, mode: &str) -> String {
    let started = start_out(h, toml, mode);
    assert_eq!(
        started.code,
        0,
        "start failed: {}{}",
        started.stderr,
        h.log_tail()
    );
    started.stdout.trim().to_string()
}

fn one_task() -> String {
    plan("", &[task("t1", &["a.txt"], "")])
}

/// A one-task `pr` run whose stage PR is open and green; its id and PR number.
fn open_one(h: &RunHarness, rig: &PrRig) -> (String, u64) {
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    green_scripts(&h.repo);
    let id = start(h, &one_task(), "pr");
    rig.wait_stage(h, &id, 1, "/state", &json!("open"));
    let entry = rig.wait_stage(h, &id, 1, "/ci", &json!("green"));
    (id, entry["number"].as_u64().expect("a PR number"))
}

fn stage_branch(id: &str, n: u16) -> String {
    format!("anthrex/{id}/stage-{n}")
}

/// The run's engine log lines (`run.json`'s `log`).
fn log_lines(run: &RunInfo) -> Vec<String> {
    run_json(run)["log"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| e["text"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn wait_complete(h: &RunHarness, id: &str, wait: std::time::Duration) -> RunInfo {
    h.wait_run(
        id,
        |r| r.state.is_terminal() || r.state == RunState::Complete,
        wait,
    )
}

#[test]
fn e2e_pr_stage_dependency_on_a_later_stage_is_rejected() {
    // Pinning: 9.1's validation, unchanged in `pr` mode.
    let (h, rig) = pr_harness("");
    let toml = plan(
        "",
        &[
            task("t1", &["a.txt"], "deps = [\"t2\"]"),
            task("t2", &["b.txt"], "stage = 2"),
        ],
    );
    let refused = start_out(&h, &toml, "pr");
    assert_eq!(refused.code, 1, "{}", refused.stderr);
    assert!(
        refused
            .stderr
            .contains("stage 1 cannot depend on t2 in stage 2"),
        "{}",
        refused.stderr
    );
    assert!(h.snapshot().runs.is_empty(), "no run");
    assert!(no_run_branches(&h.repo));
    assert_eq!(rig.remote_refs(), ["refs/heads/main"]);
    assert!(rig.ctl().prs().is_empty());
}

#[test]
fn e2e_pr_stage_prs_open_in_order_with_the_right_bases() {
    let (h, rig) = pr_harness("");
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    for (id, file) in [("t1", "a.txt"), ("t2", "b.txt"), ("t3", "c.txt")] {
        h.script(
            &format!("worker-{id}-1"),
            &[commit(file, &format!("{id}\n")), done(id)],
        );
        h.script(&format!("reviewer-{id}-1"), &[approve()]);
    }
    let toml = plan(
        "",
        &[
            task("t1", &["a.txt"], ""),
            task("t2", &["b.txt"], ""),
            task("t3", &["c.txt"], "stage = 2"),
        ],
    );
    let id = start(&h, &toml, "pr");
    rig.wait_stage(&h, &id, 2, "/state", &json!("open"));
    let prs = rig.ctl().prs();
    assert_eq!(prs.len(), 2, "{prs:#?}");
    let short = &id[id.len() - 4..];
    let (one, two) = (&prs[0], &prs[1]);
    // In order: stage 1's PR first, on the base branch; stage 2's on stage 1's branch.
    assert_eq!(
        (one.number, one.base.as_str(), one.head.clone()),
        (1, "main", stage_branch(&id, 1))
    );
    assert_eq!(
        (two.number, two.base.clone(), two.head.clone()),
        (2, stage_branch(&id, 1), stage_branch(&id, 2))
    );
    assert_eq!(
        one.title,
        format!("[anthrex r{short} 1/2] Task t1 (+1 more)")
    );
    assert_eq!(two.title, format!("[anthrex r{short} 2/2] Task t3"));
    let (body1, body2) = (&one.body, &two.body);
    assert!(
        body1.starts_with(&format!(
            "<!-- anthrex:pr {id} stage 1 -->\n**Goal:** Add a\n\n**Stage 1 of 2:** Task t1 (+1 more)\n**Stack:** based on the base branch main\n"
        )),
        "{body1}"
    );
    assert!(
        body2.starts_with(&format!(
            "<!-- anthrex:pr {id} stage 2 -->\n**Goal:** Add a\n\n**Stage 2 of 2:** Task t3\n**Stack:** based on stage 1, #1\n"
        )),
        "{body2}"
    );
    for (body, tasks) in [(body1, &["t1", "t2"][..]), (body2, &["t3"][..])] {
        for t in tasks {
            assert!(
                body.contains(&format!("\n| {t} | Task {t} | S | check |")),
                "{body}"
            );
        }
        assert!(
            body.contains("**anthrex never merges this pull request**"),
            "{body}"
        );
    }
    // Each PR's head is its stage's head, and the remote holds exactly the two stage
    // branches beside the base.
    for (pr, n) in [(one, 1), (two, 2)] {
        assert_eq!(pr.head_oid, h.git(&["rev-parse", &stage_branch(&id, n)]));
    }
    let mut expected = vec![
        format!("refs/heads/{}", stage_branch(&id, 1)),
        format!("refs/heads/{}", stage_branch(&id, 2)),
        "refs/heads/main".to_string(),
    ];
    expected.sort();
    assert_eq!(rig.remote_refs(), expected);
    assert!(rig.contains(&two.head_oid, &one.head_oid), "stacked");
}

/// No run, no local branch, nothing new on the remote and no PR after a refused start.
fn left_nothing(h: &RunHarness, rig: Option<&PrRig>, case: &str) {
    assert!(h.snapshot().runs.is_empty(), "{case}: a run was left");
    assert!(no_run_branches(&h.repo), "{case}: a branch was left");
    assert_eq!(
        h.git(&["for-each-ref", "refs/anthrex/"]),
        "",
        "{case}: a private ref was left"
    );
    if let Some(rig) = rig {
        let anthrex: Vec<String> = (rig.remote_refs().into_iter())
            .filter(|r| r.starts_with("refs/heads/anthrex/"))
            .collect();
        assert!(anthrex.is_empty(), "{case}: pushed {anthrex:?}");
        assert!(rig.ctl().prs().is_empty(), "{case}: a PR was opened");
    }
}

#[test]
fn e2e_pr_preflight_refuses_each_failure_with_its_text() {
    let toml = one_task();
    // `gh` missing, on the real `GhHost`: `ANTHREX_CODE_HOST=gh` and the harness's pinned
    // `ANTHREX_GH_BIN`, a path that does not exist. No PATH lookup, never a real `gh`.
    let h = RunHarness::new("");
    assert!(!std::path::Path::new(daemon::manager::TEST_GH_BIN).exists());
    h.git(&["config", "remote.origin.url", URL]);
    refused_with(
        &start_out(&h, &toml, "pr"),
        &format!(
            "gh is not installed (looked for {}); install it, or use --delivery local",
            daemon::manager::TEST_GH_BIN
        ),
    );
    left_nothing(&h, None, "gh missing");
    drop(h);

    // The rest on `FakeHost`, whose GitHub starts knowing nothing.
    let (h, rig) = pr_harness_unscripted("");
    h.git(&["config", "--unset", "remote.origin.url"]);
    refused_with(
        &start_out(&h, &toml, "pr"),
        "remote origin is not set in this repository; use --delivery local",
    );
    left_nothing(&h, Some(&rig), "no remote");

    let gitlab = "https://gitlab.example.com/fake/app.git";
    h.git(&["config", "remote.origin.url", gitlab]);
    refused_with(
        &start_out(&h, &toml, "pr"),
        &format!("remote origin is not a GitHub repository ({gitlab}); use --delivery local"),
    );
    left_nothing(&h, Some(&rig), "not GitHub");

    h.git(&["config", "remote.origin.url", URL]);
    refused_with(
        &start_out(&h, &toml, "pr"),
        "gh is not logged in to github.com; run gh auth login, or use --delivery local",
    );
    left_nothing(&h, Some(&rig), "logged out");

    rig.ctl().log_in("github.com");
    refused_with(
        &start_out(&h, &toml, "pr"),
        "gh cannot see fake/app: GraphQL: Could not resolve to a Repository with the name 'fake/app'. (repository)",
    );
    left_nothing(&h, Some(&rig), "repository missing");

    // The push URL leads nowhere: git's own last line, quoted.
    rig.ctl().create_repo("fake", "app", &rig.bare, "main");
    let bare = rig.bare.display().to_string();
    let nowhere = h
        .dir
        .path()
        .join("no-such-remote.git")
        .display()
        .to_string();
    h.git(&["config", "--unset", &format!("url.{bare}.pushInsteadOf")]);
    h.git(&["config", &format!("url.{nowhere}.pushInsteadOf"), URL]);
    refused_with(
        &start_out(&h, &toml, "pr"),
        "a dry-run push to origin was refused, so anthrex cannot push there; git said: \"and the repository exists.\"",
    );
    left_nothing(&h, Some(&rig), "dry-run push refused");
    h.git(&["config", "--unset", &format!("url.{nowhere}.pushInsteadOf")]);
    h.git(&["config", &format!("url.{bare}.pushInsteadOf"), URL]);

    rig.bare_git(&["update-ref", "-d", "refs/heads/main"]);
    refused_with(
        &start_out(&h, &toml, "pr"),
        "the base branch main does not exist on origin; push it first",
    );
    left_nothing(&h, Some(&rig), "base missing");
    assert!(rig.remote_refs().is_empty(), "{:?}", rig.remote_refs());
}

#[test]
fn e2e_pr_rewritten_remote_stage_branch_halts_without_forcing() {
    let (h, rig) = pr_harness("");
    // Every update of the bare repository's refs is logged from here on.
    rig.bare_git(&["config", "core.logAllRefUpdates", "always"]);
    let (id, number) = open_one(&h, &rig);
    let branch = stage_branch(&id, 1);
    let pushed = rig.bare_git(&["rev-parse", &branch]);
    let rewritten = rig.ctl().force_rewrite(&branch);
    let run = h.wait_run(
        &id,
        |r| r.state == RunState::Halted,
        VIEW_WAIT.saturating_add(FETCH_WAIT),
    );
    let (r7, p7) = (&rewritten[..7], &pushed[..7]);
    assert_eq!(
        run.halted_reason.as_deref(),
        Some(
            format!(
                "remote stage branch {branch} moved: someone else pushed to it or rewrote it; stage 1 (PR #{number}) is at {r7} on origin, which does not contain {p7}, and anthrex never forces a push"
            )
            .as_str()
        )
    );
    let line = format!("stage 1 (PR #{number}): origin has {r7}, which does not contain {p7}");
    assert!(log_lines(&run).contains(&line), "{:#?}", log_lines(&run));

    // No forced push: the remote branch is still the rewrite, and its reflog holds
    // anthrex's one push (a creation) and then the rewrite, nothing after it.
    assert_eq!(rig.bare_git(&["rev-parse", &branch]), rewritten);
    let reflog = rig.bare_git(&[
        "reflog",
        "show",
        "--format=%H",
        &format!("refs/heads/{branch}"),
    ]);
    assert_eq!(
        reflog.lines().collect::<Vec<_>>(),
        [rewritten.as_str(), pushed.as_str()],
        "newest first"
    );
    // And no `gh` call pushed, forced or rewrote anything.
    for argv in rig.ctl().calls() {
        assert!(
            !argv
                .iter()
                .any(|a| a.contains("force") || a.starts_with('+') || a == "push"),
            "{argv:?}"
        );
    }
    assert_eq!(rig.wait_pr(number, |_| true).state, PrState::Open);
}

#[test]
fn e2e_pr_user_commit_on_a_stage_branch_is_adopted_and_propagated() {
    let (h, rig) = pr_harness("");
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    for (id, file) in [("t1", "a.txt"), ("t2", "b.txt")] {
        h.script(
            &format!("worker-{id}-1"),
            &[commit(file, &format!("{id}\n")), done(id)],
        );
        h.script(&format!("reviewer-{id}-1"), &[approve()]);
    }
    let toml = plan(
        "",
        &[
            task("t1", &["a.txt"], ""),
            task("t2", &["b.txt"], "stage = 2"),
        ],
    );
    let id = start(&h, &toml, "pr");
    rig.wait_stage(&h, &id, 2, "/state", &json!("open"));
    let one = stage_branch(&id, 1);
    let sha = rig
        .ctl()
        .commit(&one, "user.txt", "by the user\n", "a user's own fix");
    // Adopted: the local stage branch is the user's commit, and PR 1 keeps it.
    until("the adopt", VIEW_WAIT.saturating_add(FETCH_WAIT), || {
        (git_read(&h.repo, &["rev-parse", &one]).as_deref() == Some(sha.as_str())).then_some(())
    });
    // Propagated: stage 2's PR head then contains it.
    let two = rig.wait_pr(2, |p| rig.contains(&p.head_oid, &sha));
    assert_eq!(two.base, one, "still stacked on stage 1");
    let first = rig.wait_pr(1, |_| true);
    assert_eq!(
        first.head_oid, sha,
        "nothing was pushed over the user's commit"
    );
    let run = h.run(&id).unwrap();
    assert!(
        log_lines(&run).contains(&format!(
            "stage 1 (PR #1): adopted {} from {one}",
            &sha[..7]
        )),
        "{:#?}",
        log_lines(&run)
    );
    assert_eq!(run.state, RunState::Running, "{:?}", run.halted_reason);
}

/// Decision 38's and 39's texts for run `id`.
fn accept_text(id: &str) -> String {
    format!(
        "run {id} is delivered by pull request; merge its pull requests on GitHub (anthrex never merges)"
    )
}

fn discard_text(id: &str) -> String {
    format!(
        "run {id} is delivered by pull request; its branches back its pull requests, so discard is refused (anthrex run cancel stops its agents)"
    )
}

#[test]
fn e2e_pr_run_accept_and_discard_are_refused() {
    let (h, rig) = pr_harness("");
    let (id, number) = open_one(&h, &rig);
    refused_with(&run(&h, &["accept", &id, "--yes"]), &accept_text(&id));
    refused_with(
        &run(&h, &["discard", &id, "--confirm", &id]),
        &discard_text(&id),
    );
    assert_eq!(h.run(&id).unwrap().state, RunState::Running);

    // Merged by the user on GitHub: the run completes. `run accept` is still refused;
    // ruling R-1: `run discard` cleans up locally and makes no host call.
    rig.ctl().merge(number, MergeMethodArg::Merge, false);
    let run = wait_complete(&h, &id, VIEW_WAIT.saturating_add(FETCH_WAIT));
    assert_eq!(run.state, RunState::Complete, "{:?}", run.halted_reason);
    refused_with(&run_long(&h, &["accept", &id, "--yes"]), &accept_text(&id));
    let before = rig.ctl().calls();
    let remote_before = rig.remote_refs();
    let discarded = run_long(&h, &["discard", &id, "--confirm", &id]);
    ok(&discarded);
    let run = h.wait_run(&id, |r| r.state == RunState::Discarded, FINISH_WAIT);
    assert_eq!(run.state, RunState::Discarded);
    assert_eq!(rig.ctl().calls(), before, "the discard made no host call");
    assert!(no_run_branches(&h.repo), "local branches removed");
    assert_eq!(
        h.git(&[
            "for-each-ref",
            &format!("refs/remotes/origin/anthrex/{id}/")
        ]),
        ""
    );
    assert_eq!(rig.remote_refs(), remote_before, "the remote is untouched");
    assert_eq!(rig.wait_pr(number, |_| true).state, PrState::Merged);
}

/// `anthrex run <args> --dir <repo>`, waiting as `run accept` and `run discard` may.
fn run_long(h: &RunHarness, args: &[&str]) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--dir", &repo]);
    out(h.anthrex_input(&all, ""))
}

/// The check: in the tier-3 checkout (`.full`) it says it is waiting, then waits for
/// `go` at most 7 s (inside the harness's 10 s `check_timeout_secs`) and passes either
/// way; anywhere else it passes at once.
const GATE_SH: &str = r#"case "$PWD" in
  */.full)
    : > "$GATE_DIR/waiting"
    end=$(( $(date +%s) + 7 ))
    while [ ! -e "$GATE_DIR/go" ] && [ "$(date +%s)" -lt "$end" ]; do sleep 0.1; done
    ;;
esac
exit 0
"#;

#[test]
fn e2e_pr_run_deliver_requests_tier3_before_the_pr_opens() {
    let (h, rig) = pr_harness_with("", &[("gate.sh", GATE_SH)], "", None);
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    green_scripts(&h.repo);
    let gate = h.cache_dir();
    let toml = format!(
        "goal = \"Add a\"\n\n[profile]\ncheck = \"sh gate.sh\"\nenv = {{ GATE_DIR = {:?} }}\n{}",
        gate.display().to_string(),
        task("t1", &["a.txt"], "")
    );
    let id = start(&h, &toml, "pr");
    until("tier 3 on the stage head", PR_OPEN_WAIT, || {
        gate.join("waiting").exists().then_some(())
    });
    let out = run(&h, &["deliver", &id, "--stage", "1"]);
    std::fs::write(gate.join("go"), "").unwrap();
    assert_eq!(
        ok(&out),
        "stage 1: tier 3 requested; its PR opens when tier 3 is green\n"
    );
    let entry = rig.wait_stage(&h, &id, 1, "/state", &json!("open"));
    let url = entry["url"].as_str().unwrap().to_string();
    let out = run(&h, &["deliver", &id, "--stage", "1"]);
    assert_eq!(ok(&out), format!("stage 1's PR is already open: {url}\n"));
}

#[test]
fn e2e_pr_cancel_leaves_prs_open() {
    let (h, rig) = pr_harness("");
    let (id, number) = open_one(&h, &rig);
    ok(&run(&h, &["cancel", &id]));
    let run = h.wait_run(
        &id,
        |r| {
            r.outcome
                .as_deref()
                .is_some_and(|o| o.starts_with("cancelled"))
        },
        FINISH_WAIT,
    );
    assert_eq!(
        run.outcome.as_deref(),
        Some(format!("cancelled; 1 pull request left open: #{number}").as_str())
    );
    let pr = rig.wait_pr(number, |_| true);
    assert_eq!(pr.state, PrState::Open, "the PR stays open");
    let branch = format!("refs/heads/{}", stage_branch(&id, 1));
    assert!(rig.remote_refs().contains(&branch), "its branch stays");
    assert!(
        rig.calls_of(&["pr", "close"]).is_empty(),
        "{:?}",
        rig.ctl().calls()
    );
}

#[test]
fn e2e_local_mode_makes_no_host_call() {
    let (h, rig) = pr_harness("");
    let calls = rig.github.join("calls.jsonl");
    assert!(!calls.exists(), "the daemon's start made no host call");
    green_scripts(&h.repo);
    let id = start(&h, &one_task(), "local");
    let run = wait_complete(&h, &id, support::run_harness::RUN_WAIT);
    assert_eq!(run.state, RunState::Complete, "{}", report(&run));
    assert!(run.delivery.is_none());
    ok(&run_long(&h, &["accept", &id, "--yes"]));
    h.wait_run(&id, |r| r.state == RunState::Accepted, FINISH_WAIT);
    assert_eq!(git_in(&h.repo, &["show", "main:a.txt"]), "a");
    assert!(!calls.exists(), "a local run made a host call");
    assert!(rig.ctl().prs().is_empty());
}
