//! Milestone 9.2's `PrRig` (brief, "Shared test helpers"): a harness repository whose
//! `origin` reads as `https://github.com/fake/app.git` but reaches a local bare
//! repository (`insteadOf`), and a daemon on `FakeHost` (`ANTHREX_CODE_HOST=fake`) whose
//! scripted GitHub lives in `<tmp>/github`. Nothing here, or in the daemon it starts,
//! can reach GitHub or run a real `gh`.
//!
//! Dropping the rig asserts that nothing asked the fake GitHub to merge, approve or
//! enable auto-merge (`forbidden.jsonl` stays empty).

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

use daemon::host::fake::{FakeGithubCtl, FakePr};
use daemon::host::{
    HOST_READ_TIMEOUT, HOST_WRITE_TIMEOUT, LOG_TIMEOUT, PREFLIGHT_BOUND, PUSH_TIMEOUT,
    RepoPermission,
};
use serde_json::Value;

use super::RunningCommand;
use super::decider::{SPAWN_SLACK, TIMEOUT_SECS as DECIDER_TIMEOUT_SECS};
use super::run_adapt::{GOAL_WAIT, with_deciders};
use super::run_daemon::DAEMON_START_WAIT;
use super::run_harness::{
    CHECK_TIMEOUT_SECS, GIT_TIMEOUT_SECS, REQUEST_WAIT, RUN_WAIT, RunHarness, git_in,
};
use super::run_tiers::TIER_WAIT;

/// How long a one-task `pr` run may take to open its stage PR and show it (ruling "task
/// 14 fix round 1", I1; re-derived op by op in the final fix wave, review C M3): one
/// task path (`RUN_WAIT`); tier 3 on the stage head, its two check commands at the
/// harness's `check_timeout_secs` and its own git calls ([`TIER3_GIT_CALLS`] at
/// `git_timeout_secs`); then each host op's own bound, the executor's margin included:
/// the push ([`PUSH_WAIT`]), the open ([`OPEN_WAIT`]) and the first view after one poll
/// ([`VIEW_WAIT`]); and 3 s of scheduling. Every term is the daemon's or the harness's
/// own constant (`docs/timing-budgets.md`).
pub const PR_OPEN_WAIT: Duration = RUN_WAIT
    .saturating_add(Duration::from_secs(
        2 * CHECK_TIMEOUT_SECS + TIER3_GIT_CALLS * GIT_SECS + 3,
    ))
    .saturating_add(PUSH_WAIT)
    .saturating_add(OPEN_WAIT)
    .saturating_add(VIEW_WAIT);

/// The git calls of one tier-3 job (its checkout, diff and cache reads): at most 10,
/// as `run_tiers::TIER_WAIT` counts a test's tier jobs'.
const TIER3_GIT_CALLS: u64 = 10;

/// A PR's open (`OpenPr`'s bound: `pr list`, one read, then `pr create`, one write, plus
/// the margin).
pub const OPEN_WAIT: Duration = HOST_READ_TIMEOUT
    .saturating_add(HOST_WRITE_TIMEOUT)
    .saturating_add(Duration::from_secs(OP_MARGIN_SECS));

/// How long `anthrex run start --delivery pr` may take: a local start's request
/// (`REQUEST_WAIT`) plus the host preflight, bounded by the daemon's own
/// `PREFLIGHT_BOUND`.
pub const PR_START_WAIT: Duration = REQUEST_WAIT.saturating_add(PREFLIGHT_BOUND);

/// The harness's `git_timeout_secs`.
const GIT_SECS: u64 = GIT_TIMEOUT_SECS;
/// The executor's margin over each host op's commands (`host_ops::bound`, M9.2.12).
const OP_MARGIN_SECS: u64 = 5;

/// One view of a PR after it changed (task M9.2.16): the poll at the test
/// configuration's `poll_max_secs` ([`POLL_MAX_SECS`]), then the `ViewPr` op's own
/// bound (two reads at `HOST_READ_TIMEOUT`, plus the executor's margin).
pub const VIEW_WAIT: Duration = Duration::from_secs(POLL_MAX_SECS + OP_MARGIN_SECS)
    .saturating_add(HOST_READ_TIMEOUT)
    .saturating_add(HOST_READ_TIMEOUT);

/// A stage push (`Push`'s bound: `PUSH_TIMEOUT` and the seal's two reads, plus the
/// margin).
pub const PUSH_WAIT: Duration = PUSH_TIMEOUT.saturating_add(Duration::from_secs(
    2 * HOST_READ_TIMEOUT.as_secs() + OP_MARGIN_SECS,
));

/// An adopt's fetch (`Fetch`'s bound: `PUSH_TIMEOUT` and six reads, plus the margin).
pub const FETCH_WAIT: Duration = PUSH_TIMEOUT.saturating_add(Duration::from_secs(
    6 * HOST_READ_TIMEOUT.as_secs() + OP_MARGIN_SECS,
));

/// A CI re-run (`RerunFailed`'s bound: one write, plus the margin).
pub const RERUN_WAIT: Duration =
    HOST_WRITE_TIMEOUT.saturating_add(Duration::from_secs(OP_MARGIN_SECS));

/// The single-test runs a CI red may cost before its fix task: the reproduction, then
/// 9.1's bisect of at most four merges (the base, the head and two halvings, each red
/// probe run twice).
const PROBES: u64 = 1 + 2 * 4;

/// A red CI head to its fix task (task M9.2.16): the view that sees it
/// ([`VIEW_WAIT`]), the failed log (`LOG_TIMEOUT`, plus the margin), the `ci_summary`
/// decider (`with_deciders`'s 5 s timeout and 2 s slot wait, plus the spawn's
/// `SPAWN_SLACK`), then [`PROBES`] single-test runs in the tier-3 checkout, each a
/// checkout (five git calls at [`GIT_SECS`]) and one command at `CHECK_TIMEOUT_SECS`.
pub const CI_FIX_WAIT: Duration = VIEW_WAIT
    .saturating_add(LOG_TIMEOUT)
    .saturating_add(Duration::from_secs(
        OP_MARGIN_SECS + DECIDER_TIMEOUT_SECS + 2 + PROBES * (5 * GIT_SECS + CHECK_TIMEOUT_SECS),
    ))
    .saturating_add(SPAWN_SLACK);

/// A fix task's path to the PR (task M9.2.16): one task path with its tier commands
/// (`TIER_WAIT`), the push ([`PUSH_WAIT`]), and the view of the new head
/// ([`VIEW_WAIT`]).
pub const FIX_PUSH_WAIT: Duration = TIER_WAIT
    .saturating_add(PUSH_WAIT)
    .saturating_add(VIEW_WAIT);

/// A writer's new comment to its fix task (task M9.2.17): the view that sees it
/// ([`VIEW_WAIT`]), its author's `Permission` read (`HOST_READ_TIMEOUT`, plus the
/// executor's margin), the batch's quiet ([`REVIEW_BATCH_SECS`]) and the 1 s tick whose
/// pass closes it, and 2 s of scheduling.
pub const COMMENT_WAIT: Duration =
    VIEW_WAIT
        .saturating_add(HOST_READ_TIMEOUT)
        .saturating_add(Duration::from_secs(
            OP_MARGIN_SECS + REVIEW_BATCH_SECS + 1 + 2,
        ));

/// The test configuration's `poll_max_secs`.
pub const POLL_MAX_SECS: u64 = 2;

/// The test configuration's `review_batch_secs`: above `poll_max_secs + 1` (review C,
/// I2), so the view after a batch's first thread always lands before the batch closes,
/// and two comments written by two calls are one batch, as one review is on GitHub.
pub const REVIEW_BATCH_SECS: u64 = POLL_MAX_SECS + 2;

/// One reply on a thread (`Reply`'s bound: the viewer's login and the thread's listing,
/// two reads, then the post, one write, plus the margin).
pub const REPLY_WAIT: Duration = HOST_WRITE_TIMEOUT.saturating_add(Duration::from_secs(
    2 * HOST_READ_TIMEOUT.as_secs() + OP_MARGIN_SECS,
));

/// A retarget (`Retarget`'s bound: one write, plus the margin).
pub const RETARGET_WAIT: Duration =
    HOST_WRITE_TIMEOUT.saturating_add(Duration::from_secs(OP_MARGIN_SECS));

/// A stage's landing or a base move to its next push (task M9.2.17): the view that sees
/// it ([`VIEW_WAIT`]), the base's fetch ([`FETCH_WAIT`]), the base sync, bounded as one
/// task path with its tier commands (`TIER_WAIT`: it is a merge and tier 2 on it), the
/// push ([`PUSH_WAIT`]), and a retarget ([`RETARGET_WAIT`]).
pub const LAND_WAIT: Duration = VIEW_WAIT
    .saturating_add(FETCH_WAIT)
    .saturating_add(TIER_WAIT)
    .saturating_add(PUSH_WAIT)
    .saturating_add(RETARGET_WAIT);

/// How long a thread seen by a view may take to become a batch, and so a task or a
/// wake (task M9.2.17 fix round 1, m1): the test configuration's `review_batch_secs`
/// ([`REVIEW_BATCH_SECS`]) and one 1 s tick whose pass closes the batch.
pub const BATCH_QUIET: Duration = Duration::from_secs(REVIEW_BATCH_SECS + 1);

/// [`PrRig::quiet_views`]'s bound: three views, the batch's quiet, and one more view.
pub const QUIET_VIEWS_WAIT: Duration = VIEW_WAIT
    .saturating_add(VIEW_WAIT)
    .saturating_add(VIEW_WAIT)
    .saturating_add(BATCH_QUIET)
    .saturating_add(VIEW_WAIT);

/// How long `anthrex run start --goal … --delivery pr` may take: a goal start's reply
/// (`GOAL_WAIT`) plus the host preflight a `pr` goal runs before triage
/// (`PREFLIGHT_BOUND`).
pub const PR_GOAL_WAIT: Duration = GOAL_WAIT.saturating_add(PREFLIGHT_BOUND);

/// `anthrex <args>` (a `run start --delivery pr`), waiting at most [`PR_START_WAIT`].
pub fn pr_start(harness: &RunHarness, args: &[&str]) -> Output {
    RunningCommand::start(&mut harness.command(args)).finish(PR_START_WAIT)
}

/// The URL the repository's `origin` names; `insteadOf` sends it to the bare remote.
pub const URL: &str = "https://github.com/fake/app.git";

/// The brief's test configuration for every `pr` end-to-end test, with
/// `review_batch_secs` raised to [`REVIEW_BATCH_SECS`] (review C, I2).
pub const DELIVERY_TOML: &str =
    "[delivery]\npoll_secs = 1\npoll_max_secs = 2\nreview_batch_secs = 4\n";

pub struct PrRig {
    pub bare: PathBuf,
    pub github: PathBuf,
    ctl: FakeGithubCtl,
}

impl PrRig {
    /// The bare remote at `<tmp>/remote.git` holding the repository's `main`, the
    /// repository's `origin` pointed at it through [`URL`], and the fake GitHub's
    /// `fake/app` with `gh` logged in to github.com and `tester` a writer.
    pub fn new(harness: &RunHarness) -> PrRig {
        let rig = PrRig::unscripted(harness);
        rig.ctl.create_repo("fake", "app", &rig.bare, "main");
        rig.ctl.log_in("github.com");
        rig.ctl.set_permission("tester", RepoPermission::Write);
        rig
    }

    /// [`PrRig::new`]'s bare remote and `origin`, with a fake GitHub that knows
    /// nothing yet: no repository, no login (task M9.2.16's preflight cases).
    pub fn unscripted(harness: &RunHarness) -> PrRig {
        let tmp = harness.dir.path();
        let bare = tmp.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        git_in(&bare, &["init", "-q", "--bare", "-b", "main"]);
        git_in(&bare, &["config", "user.name", "Remote User"]);
        git_in(&bare, &["config", "user.email", "remote@example.com"]);
        let bare_s = bare.display().to_string();
        harness.git(&["config", "remote.origin.url", URL]);
        for key in ["insteadOf", "pushInsteadOf"] {
            harness.git(&["config", &format!("url.{bare_s}.{key}"), URL]);
        }
        // Both directions reach the bare repository, never GitHub.
        assert_eq!(harness.git(&["remote", "get-url", "origin"]), bare_s);
        assert_eq!(
            harness.git(&["remote", "get-url", "--push", "origin"]),
            bare_s
        );
        harness.git(&["push", "-q", "origin", "main"]);
        let github = tmp.join("github");
        let ctl = FakeGithubCtl::open(&github);
        PrRig { bare, github, ctl }
    }

    /// The daemon's host selection: the fake, on this rig's directory.
    pub fn env(&self) -> Vec<(String, String)> {
        vec![
            ("ANTHREX_CODE_HOST".into(), "fake".into()),
            (
                "ANTHREX_FAKE_HOST_DIR".into(),
                self.github.display().to_string(),
            ),
            // Ruling m4: a push or fetch that `insteadOf` did not rewrite to the bare
            // repository fails outright instead of reaching the network.
            ("GIT_ALLOW_PROTOCOL".into(), "file".into()),
        ]
    }

    pub fn ctl(&self) -> &FakeGithubCtl {
        &self.ctl
    }

    /// `git <args>` in the bare remote; its trimmed stdout.
    pub fn bare_git(&self, args: &[&str]) -> String {
        git_in(&self.bare, args)
    }

    /// Every branch the bare remote holds, `refs/heads/...`, sorted.
    pub fn remote_refs(&self) -> Vec<String> {
        let listed = self.bare_git(&["for-each-ref", "--format=%(refname)", "refs/heads/"]);
        let mut refs: Vec<String> = listed.lines().map(str::to_string).collect();
        refs.sort();
        refs
    }

    /// The `gh` calls (`calls.jsonl`) whose argv starts with `prefix`.
    pub fn calls_of(&self, prefix: &[&str]) -> Vec<Vec<String>> {
        let calls = self.ctl.calls();
        calls
            .into_iter()
            .filter(|argv| {
                argv.len() >= prefix.len() && argv.iter().zip(prefix).all(|(a, p)| a == p)
            })
            .collect()
    }

    /// Whether `ancestor` is in the history of `commit`, in the bare remote.
    pub fn contains(&self, commit: &str, ancestor: &str) -> bool {
        std::process::Command::new("git")
            .args(["merge-base", "--is-ancestor", ancestor, commit])
            .current_dir(&self.bare)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// The window a test watches for something that must *not* happen (a duplicate
    /// task, reply, batch or wake; task M9.2.17 fix round 1, m1), on a run with one PR.
    /// Two more views; the second has been answered once a third starts (one view of a
    /// PR is in flight at a time). [`BATCH_QUIET`] after that start, the next view a
    /// pass emits comes from a pass that has closed any batch the second view opened,
    /// and its task, hold or wake is recorded in that same step. So: wait for the third view, then the quiet,
    /// then one view that starts after it. A deadline loop on `calls.jsonl` (which
    /// records a call when it starts), at most [`QUIET_VIEWS_WAIT`].
    pub fn quiet_views(&self) {
        let views = || self.calls_of(&["pr", "view"]).len();
        let deadline = Instant::now() + QUIET_VIEWS_WAIT;
        let wait = |what: &str, done: &mut dyn FnMut() -> bool| loop {
            if done() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what} did not happen within {QUIET_VIEWS_WAIT:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        let start = views();
        wait("three more views", &mut || views() >= start + 3);
        let third = Instant::now();
        wait("the batch's quiet", &mut || third.elapsed() >= BATCH_QUIET);
        let after = views();
        wait("a view after the quiet", &mut || views() > after);
    }

    /// Stage `stage`'s entry of `anthrex run prs <run> --json`, now.
    pub fn stage_entry(&self, harness: &RunHarness, run: &str, stage: u16) -> Value {
        let out = harness.anthrex(&["run", "prs", run, "--json"]);
        let all: Value = serde_json::from_slice(&out.stdout).expect("run prs --json");
        all[usize::from(stage) - 1].clone()
    }

    /// Waits until pull request `number` satisfies `pred`.
    pub fn wait_pr(&self, number: u64, pred: impl Fn(&FakePr) -> bool) -> FakePr {
        let deadline = Instant::now() + PR_OPEN_WAIT;
        loop {
            let prs = self.ctl.prs();
            if let Some(pr) = prs.iter().find(|p| p.number == number && pred(p)) {
                return pr.clone();
            }
            assert!(
                Instant::now() < deadline,
                "PR #{number} did not get there within {PR_OPEN_WAIT:?}: {prs:#?}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Waits until the fake GitHub holds `count` pull requests.
    pub fn wait_prs(&self, count: usize) -> Vec<FakePr> {
        let deadline = Instant::now() + PR_OPEN_WAIT;
        loop {
            let prs = self.ctl.prs();
            if prs.len() >= count {
                return prs;
            }
            assert!(
                Instant::now() < deadline,
                "{count} PRs did not open within {PR_OPEN_WAIT:?}: {prs:#?}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Polls `anthrex run prs <run> --json` until stage `stage`'s entry, at the JSON
    /// `pointer`, equals `value`; that entry.
    pub fn wait_stage(
        &self,
        harness: &RunHarness,
        run: &str,
        stage: u16,
        pointer: &str,
        value: &Value,
    ) -> Value {
        self.wait_stage_within(harness, run, stage, pointer, value, PR_OPEN_WAIT)
    }

    /// [`PrRig::wait_stage`] for at most `wait`.
    pub fn wait_stage_within(
        &self,
        harness: &RunHarness,
        run: &str,
        stage: u16,
        pointer: &str,
        value: &Value,
        wait: Duration,
    ) -> Value {
        let deadline = Instant::now() + wait;
        loop {
            let out = harness.anthrex(&["run", "prs", run, "--json"]);
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let entry = serde_json::from_str::<Value>(&stdout)
                .ok()
                .and_then(|all| all.get(usize::from(stage) - 1).cloned());
            if let Some(entry) = &entry
                && entry.pointer(pointer) == Some(value)
            {
                return entry.clone();
            }
            assert!(
                Instant::now() < deadline,
                "stage {stage}'s {pointer} is not {value} within {wait:?}: {stdout}\n{}",
                harness.log_tail()
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Drop for PrRig {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let forbidden = self.ctl.forbidden();
        assert!(
            forbidden.is_empty(),
            "something asked the host to land: {forbidden:?}"
        );
    }
}

/// A harness with the brief's `[delivery]` test configuration (and `extra_toml`) whose
/// daemon starts on this rig's fake GitHub.
pub fn pr_harness(extra_toml: &str) -> (RunHarness, PrRig) {
    pr_harness_with("", &[], extra_toml, None)
}

/// [`pr_harness`] with `orchestrator` lines, `files` in the base commit and, with
/// `deciders`, M8b's deciders in that mode (`with_deciders`: `fake-agent` answering from
/// `<tmp>/deciders`, [`RunHarness::decider`]'s directory).
pub fn pr_harness_with(
    orchestrator: &str,
    files: &[(&str, &str)],
    extra_toml: &str,
    deciders: Option<&str>,
) -> (RunHarness, PrRig) {
    build(orchestrator, files, extra_toml, deciders, PrRig::new)
}

/// [`pr_harness`] whose fake GitHub knows nothing yet ([`PrRig::unscripted`]).
pub fn pr_harness_unscripted(extra_toml: &str) -> (RunHarness, PrRig) {
    build("", &[], extra_toml, None, PrRig::unscripted)
}

fn build(
    orchestrator: &str,
    files: &[(&str, &str)],
    extra_toml: &str,
    deciders: Option<&str>,
    rig: impl FnOnce(&RunHarness) -> PrRig,
) -> (RunHarness, PrRig) {
    // `with_deciders`'s lines do not depend on the directory; its environment does.
    let lines = deciders.map_or(String::new(), |mode| with_deciders(mode, Path::new("/")).0);
    let mut harness = RunHarness::unstarted(&format!("{lines}{orchestrator}"), &[], true, files);
    let config = harness.dir.path().join("config.toml");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{text}\n{DELIVERY_TOML}{extra_toml}\n")).unwrap();
    if let Some(mode) = deciders {
        let dir = harness.dir.path().join("deciders");
        std::fs::create_dir_all(&dir).unwrap();
        harness.env.extend(with_deciders(mode, &dir).1);
    }
    let rig = rig(&harness);
    harness.env.extend(rig.env());
    if let Err(error) = harness.start_daemon(DAEMON_START_WAIT) {
        panic!("the daemon did not start: {error}\n{}", harness.log_tail());
    }
    (harness, rig)
}

/// `run start --plan <toml> --delivery pr --yes` (task M9.2.17); the run id.
pub fn pr_run(h: &RunHarness, toml: &str) -> String {
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
        "pr",
        "--yes",
    ];
    let out = pr_start(h, &args);
    assert!(
        out.status.success(),
        "start failed: {}{}",
        String::from_utf8_lossy(&out.stderr),
        h.log_tail()
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A planned run's harness in `pr` mode (task M9.2.17): M9's orchestrator harness
/// (`RunHarness::orch`: Claude deciders, a triage that answers `plan`, the stored
/// profile) with the brief's `[delivery]` test configuration, its daemon restarted on
/// this rig's fake GitHub. Only the harness's own daemon is restarted, through its own
/// socket (`RunHarness::restart_daemon`).
pub fn orch_pr_harness() -> (RunHarness, PrRig) {
    let mut harness = RunHarness::orch("", &[]);
    let config = harness.dir.path().join("config.toml");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{text}\n{DELIVERY_TOML}\n")).unwrap();
    let rig = PrRig::new(&harness);
    let env = rig.env();
    let extra: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    harness.restart_daemon(&extra);
    (harness, rig)
}

/// `anthrex run start --goal <goal> --delivery pr` waited for as [`PR_GOAL_WAIT`]; the
/// run id.
pub fn pr_goal(h: &RunHarness, goal: &str) -> String {
    let repo = h.repo.display().to_string();
    let args = [
        "run",
        "start",
        "--goal",
        goal,
        "--delivery",
        "pr",
        "--dir",
        &repo,
    ];
    let out = RunningCommand::start(&mut h.command(&args)).finish(PR_GOAL_WAIT);
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        out.status.success() && !stdout.is_empty() && !stdout.contains('\n'),
        "exit {:?}\nstdout: {stdout}\nstderr: {}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
        h.log_tail()
    );
    stdout
}

/// The run's engine log lines (`run.json`'s `log`).
pub fn log_lines(h: &RunHarness, run: &str) -> Vec<String> {
    let path = h.data().join("runs").join(run).join("run.json");
    let all: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    (all["log"].as_array().into_iter().flatten())
        .filter_map(|e| e["text"].as_str().map(str::to_string))
        .collect()
}

/// Waits until the run's log holds `line`; the log.
pub fn wait_log_line(h: &RunHarness, run: &str, line: &str, wait: Duration) -> Vec<String> {
    let deadline = Instant::now() + wait;
    loop {
        let lines = log_lines(h, run);
        if lines.iter().any(|l| l == line) {
            return lines;
        }
        assert!(
            Instant::now() < deadline,
            "no log line {line:?} within {wait:?}: {lines:#?}\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}
