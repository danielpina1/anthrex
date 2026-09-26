//! Milestone 8b's additions to the run harness (brief, "Shared test conventions"): the
//! repository profile's requests and the onboarding scout's scripts. M8b.11 adds what
//! its tests need; later tasks add the decider helpers.

use std::process::Output;
use std::time::{Duration, Instant};

use proto::{ProfileReply, ProfileRequest, ProfileStatus, RunReply, RunRequest};
use serde_json::{Value, json};

use super::run_harness::RunHarness;

/// `PROFILE_WAIT` (brief, "Shared test conventions"; `docs/timing-budgets.md`):
/// `scouts.timeout_secs = 60` + three verification commands at
/// `onboarding.verify_timeout_secs = 10` + at most 40 engine git calls at 5 s = 290 s,
/// rounded up.
pub const PROFILE_WAIT: Duration = Duration::from_secs(300);

/// The `[orchestrator]` lines of a profile test: automatic onboarding on, verification
/// commands bounded at 10 s, scouts at their shortest timeout.
pub const PROFILE_LINES: &str =
    "onboarding.auto = true\nonboarding.verify_timeout_secs = 10\nscouts.timeout_secs = 60\n";

impl RunHarness {
    /// `anthrex profile <args> --dir <repo>`.
    pub fn profile(&self, args: &[&str]) -> Output {
        self.profile_input(args, "")
    }

    /// `anthrex profile <args> --dir <repo>` with `input` on stdin.
    pub fn profile_input(&self, args: &[&str], input: &str) -> Output {
        let repo = self.repo.display().to_string();
        let mut all = vec!["profile"];
        all.extend_from_slice(args);
        all.extend_from_slice(&["--dir", &repo]);
        self.anthrex_input(&all, input)
    }

    /// One raw profile request for the harness repository.
    pub fn profile_request(&self, request: ProfileRequest) -> ProfileReply {
        match self.request(RunRequest::Profile(request)) {
            RunReply::Profile(reply) => *reply,
            other => panic!("a profile request answered {other:?}"),
        }
    }

    pub fn profile_status(&self) -> ProfileStatus {
        match self.profile_request(ProfileRequest::Status {
            dir: self.repo.clone(),
        }) {
            ProfileReply::Status(status) => status,
            other => panic!("profile status answered {other:?}"),
        }
    }

    /// Polls `profile status` until `pred` holds, for at most `wait`.
    pub fn wait_profile(
        &self,
        what: &str,
        pred: impl Fn(&ProfileStatus) -> bool,
        wait: Duration,
    ) -> ProfileStatus {
        let deadline = Instant::now() + wait;
        loop {
            let status = self.profile_status();
            if pred(&status) {
                return status;
            }
            if Instant::now() >= deadline {
                panic!(
                    "{what}: not within {wait:?}; last status:\n{}\n--- daemon.log:\n{}",
                    serde_json::to_string_pretty(&status).unwrap(),
                    self.log_tail()
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Polls until `pred` holds for the listed windows, for at most `wait`.
    pub fn wait_windows(
        &self,
        what: &str,
        pred: impl Fn(&[proto::WindowInfo]) -> bool,
        wait: Duration,
    ) {
        let deadline = Instant::now() + wait;
        loop {
            let windows = self.windows();
            if pred(&windows) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: not within {wait:?}; windows: {windows:?}\n{}",
                self.log_tail()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// The `n`th onboarding scout's script (any onboarding id takes it, M8b decision 37).
    pub fn onboarding_script(&self, n: u32, steps: &[Value]) {
        self.script(&format!("scout-onboarding-{n}"), steps);
    }

    /// The `n`th onboarding scout reports `profile` at once.
    pub fn onboarding_report(&self, n: u32, profile: Value) {
        self.onboarding_script(
            n,
            &[json!({"mcp_call": {"tool": "submit_scout_report", "args": {
                "summary": "A shell project; check.sh checks it.",
                "files": [{"path": "check.sh", "why": "the check"}],
                "profile": profile,
            }}})],
        );
    }
}
