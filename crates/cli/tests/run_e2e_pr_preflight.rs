//! Milestone 9.2, task M9.2.16: a `pr` start refused end to end, through the real binary
//! and a real daemon on `/tmp` paths, against `FakeHost` (`PrRig`): 9.1's stage
//! validation, and each of preflight's refusals with its text, leaving nothing behind.
//! Nothing here can reach GitHub or run a real `gh`. Split from `run_e2e_pr.rs`
//! (AGENTS.md rule 8, move-only).

mod support;

use support::run_harness::RunHarness;
use support::run_plans::*;
use support::run_pr::*;
use support::run_pr_cli::*;

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
    // Fix round 1 (m5): a push or fetch could reach only `file://`, whatever order
    // preflight's checks run in.
    let h = RunHarness::with_env("", &[("GIT_ALLOW_PROTOCOL", "file")], true);
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
