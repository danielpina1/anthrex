//! Milestone 9.2's `PrRig` (brief, "Shared test helpers"): a harness repository whose
//! `origin` reads as `https://github.com/fake/app.git` but reaches a local bare
//! repository (`insteadOf`), and a daemon on `FakeHost` (`ANTHREX_CODE_HOST=fake`) whose
//! scripted GitHub lives in `<tmp>/github`. Nothing here, or in the daemon it starts,
//! can reach GitHub or run a real `gh`.
//!
//! Dropping the rig asserts that nothing asked the fake GitHub to merge, approve or
//! enable auto-merge (`forbidden.jsonl` stays empty).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use daemon::host::RepoPermission;
use daemon::host::fake::{FakeGithubCtl, FakePr};
use serde_json::Value;

use super::run_daemon::DAEMON_START_WAIT;
use super::run_harness::{RUN_WAIT, RunHarness, git_in};

/// The URL the repository's `origin` names; `insteadOf` sends it to the bare remote.
pub const URL: &str = "https://github.com/fake/app.git";

/// The brief's test configuration for every `pr` end-to-end test.
pub const DELIVERY_TOML: &str =
    "[delivery]\npoll_secs = 1\npoll_max_secs = 2\nreview_batch_secs = 1\n";

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
        ctl.create_repo("fake", "app", &bare, "main");
        ctl.log_in("github.com");
        ctl.set_permission("tester", RepoPermission::Write);
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
        ]
    }

    pub fn ctl(&self) -> &FakeGithubCtl {
        &self.ctl
    }

    /// Waits until pull request `number` satisfies `pred`.
    pub fn wait_pr(&self, number: u64, pred: impl Fn(&FakePr) -> bool) -> FakePr {
        let deadline = Instant::now() + RUN_WAIT;
        loop {
            let prs = self.ctl.prs();
            if let Some(pr) = prs.iter().find(|p| p.number == number && pred(p)) {
                return pr.clone();
            }
            assert!(
                Instant::now() < deadline,
                "PR #{number} did not get there within {RUN_WAIT:?}: {prs:#?}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Waits until the fake GitHub holds `count` pull requests.
    pub fn wait_prs(&self, count: usize) -> Vec<FakePr> {
        let deadline = Instant::now() + RUN_WAIT;
        loop {
            let prs = self.ctl.prs();
            if prs.len() >= count {
                return prs;
            }
            assert!(
                Instant::now() < deadline,
                "{count} PRs did not open within {RUN_WAIT:?}: {prs:#?}"
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
        let deadline = Instant::now() + RUN_WAIT;
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
                "stage {stage}'s {pointer} is not {value} within {RUN_WAIT:?}: {stdout}\n{}",
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
    let mut harness = RunHarness::unstarted("", &[], true, &[]);
    let config = harness.dir.path().join("config.toml");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{text}\n{DELIVERY_TOML}{extra_toml}\n")).unwrap();
    let rig = PrRig::new(&harness);
    harness.env.extend(rig.env());
    if let Err(error) = harness.start_daemon(DAEMON_START_WAIT) {
        panic!("the daemon did not start: {error}\n{}", harness.log_tail());
    }
    (harness, rig)
}
