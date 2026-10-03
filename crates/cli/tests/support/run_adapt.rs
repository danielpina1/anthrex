//! Milestone 8b's additions to the run harness (brief, "Shared test conventions"): the
//! repository profile's requests and the onboarding scout's scripts (M8b.11), and the
//! deciders, the stored profile, the stored onboarding report and `run start --goal`
//! (M8b.18).

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use proto::{
    Effort, ProfileMeta, ProfileReply, ProfileRequest, ProfileStatus, RepoProfile, Route, RunReply,
    RunRequest, Runtime, ScoutKind, ScoutReport, Strength,
};
use serde_json::{Value, json};

use super::RunningCommand;
use super::fake_agent_bin;
use super::run_daemon::DAEMON_START_WAIT;
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

/// `GOAL_WAIT` (brief, "Shared test conventions"; `docs/timing-budgets.md`): `run start
/// --goal`'s reply in a test. `REQUEST_WAIT` (60 s, `build_plan`'s git calls) + the
/// harness's `deciders.timeout_secs` (5 s) + the kill grace (2 s) + `StartGoal`'s own
/// preflight and `git ls-files` (the brief's seven calls at 5 s, 35 s) = 102 s, rounded
/// up. As landed they are nine (M8b.14's count), 112 s in all; milestone 9.5's start
/// tuning adds up to `TUNING_START_BOUND` (10 s, ruling T9-3): 122 s, so 130 s.
pub const GOAL_WAIT: Duration = Duration::from_secs(130);

/// The brief's stored profile for M8b.18 and M8b.19 (the default `protected`), with
/// `check_timeout_secs = 10` so every check keeps `RUN_WAIT`'s derivation (a stored
/// profile replaces the harness's `[orchestrator.profile]`, M8b decision 6).
pub const STORED_PROFILE: &str = "check = \"sh check.sh\"\ncheck_timeout_secs = 10\nsingle_test = \"sh tests/{test}.sh\"\ntest_passed = \"PASS {test}\"\nsample_test = \"t_ok\"\nfilter_prefixes = [\"sh tests/\"]\ngenerated = [\"Cargo.lock\"]\n";

/// The base commit's files the stored profile's commands need.
pub const ADAPT_FILES: &[(&str, &str)] = &[
    ("check.sh", "echo check ok\n"),
    ("tests/t_ok.sh", "echo PASS t_ok\n"),
];

/// A triage answer of scale `single`: one S `check`-mode code task owning `owns`.
pub fn triage_single(owns: &[&str]) -> Value {
    json!({"answer": {
        "kinds": ["code"],
        "scale": "single",
        "reason": "one small change",
        "task": {
            "title": "Add a",
            "brief": "Create the file the goal asks for.",
            "acceptance": ["the file exists"],
            "owns": owns,
            "size": "S",
            "interface_change": false,
            "test_mode": "check",
            "test_mode_reason": "a text file",
            "test_to_write": null,
        },
    }})
}

/// The id of the onboarding report [`RunHarness::stored_onboarding_report`] writes.
pub const ONBOARDING_REPORT_ID: &str = "onboarding-1";

/// The `[orchestrator]` lines and the daemon environment of a test that runs deciders
/// (brief, `with_deciders`): `mode` (`claude`, `codex` or `off`), a 5 s call timeout, a
/// 2 s slot wait, `fake-agent` as the decider command, and its scripts under `dir`.
pub fn with_deciders(mode: &str, dir: &Path) -> (String, Vec<(String, String)>) {
    let env = vec![
        (
            "ANTHREX_DECIDER_BIN".to_string(),
            fake_agent_bin().display().to_string(),
        ),
        (
            "FAKE_AGENT_DECIDER_DIR".to_string(),
            dir.display().to_string(),
        ),
    ];
    (decider_lines(mode), env)
}

/// [`with_deciders`]'s `[orchestrator]` lines.
fn decider_lines(mode: &str) -> String {
    format!("deciders.mode = \"{mode}\"\ndeciders.timeout_secs = 5\ndeciders.slot_wait_secs = 2\n")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

impl RunHarness {
    /// A harness whose daemon runs deciders in `mode` ([`with_deciders`], scripts under
    /// `<tmp>/deciders`) and records every `bash` step in `<tmp>/bash.jsonl`
    /// (`FAKE_AGENT_BASH_LOG`), with `orchestrator` lines and `files` in the base
    /// commit.
    pub fn adapt(
        mode: &str,
        orchestrator: &str,
        env: &[(&str, &str)],
        files: &[(&str, &str)],
    ) -> Self {
        let lines = format!("{}{orchestrator}", decider_lines(mode));
        let mut harness = Self::unstarted(&lines, env, true, files);
        let deciders = harness.decider_dir();
        std::fs::create_dir_all(&deciders).unwrap();
        harness.env.extend(with_deciders(mode, &deciders).1);
        let bash_log = harness.bash_log_path().display().to_string();
        harness
            .env
            .push(("FAKE_AGENT_BASH_LOG".to_string(), bash_log));
        if let Err(error) = harness.start_daemon(DAEMON_START_WAIT) {
            panic!("the daemon did not start: {error}\n{}", harness.log_tail());
        }
        harness
    }

    /// `<tmp>/deciders`: `FAKE_AGENT_DECIDER_DIR`.
    pub fn decider_dir(&self) -> PathBuf {
        self.dir.path().join("deciders")
    }

    /// `<tmp>/bash.jsonl`: `FAKE_AGENT_BASH_LOG`.
    pub fn bash_log_path(&self) -> PathBuf {
        self.dir.path().join("bash.jsonl")
    }

    /// Every `bash` step recorded, in order.
    pub fn bash_log(&self) -> Vec<Value> {
        jsonl(&self.bash_log_path())
    }

    /// Writes the `n`th scripted answer of a `kind` decider,
    /// `<tmp>/deciders/<kind>-<n>.json`.
    pub fn decider(&self, kind: &str, n: u32, value: Value) {
        let path = self.decider_dir().join(format!("{kind}-{n}.json"));
        std::fs::write(path, value.to_string()).unwrap();
    }

    /// Every decider call `fake-agent` recorded (`calls.jsonl`: kind, argv, prompt).
    pub fn decider_calls(&self) -> Vec<Value> {
        jsonl(&self.decider_dir().join("calls.jsonl"))
    }

    /// This repository's data directory, from `profile status` (decision 4).
    pub fn repo_dir(&self) -> PathBuf {
        self.profile_status().repo_dir
    }

    /// Stores `toml` as this repository's confirmed profile, as `anthrex profile
    /// confirm` would: `<repo_dir>/profile.toml`, and a `profile.meta.json` with the
    /// fingerprint of the conventions and manifests it names.
    pub fn stored_profile(&self, toml_text: &str) {
        let profile: RepoProfile = toml::from_str(toml_text).expect("a valid profile");
        let status = self.profile_status();
        let named: Vec<String> = profile
            .conventions
            .iter()
            .chain(profile.manifests.iter())
            .cloned()
            .collect();
        let meta = ProfileMeta {
            confirmed_at: unix_now(),
            report: None,
            verification: None,
            fingerprint: daemon::profile::store::fingerprint(&status.project, &named),
            edited_keys: Vec::new(),
            project: Some(status.project.clone()),
        };
        std::fs::create_dir_all(&status.repo_dir).unwrap();
        let meta_path = status.repo_dir.join(daemon::profile::store::META_FILE);
        std::fs::write(meta_path, serde_json::to_string_pretty(&meta).unwrap()).unwrap();
        let path = status.repo_dir.join(daemon::profile::store::PROFILE_FILE);
        std::fs::write(path, toml_text).unwrap();
    }

    /// Stores an onboarding report ([`ONBOARDING_REPORT_ID`]) for this repository, with
    /// `fields` (`summary`, `files`, `modules`, …) over a minimal report, and names it in
    /// the stored profile's meta, as a confirmed detection would (the size cross-check's
    /// evidence, decision 19). Call it after [`Self::stored_profile`]. The brief names
    /// it `onboarding_report`, which M8b.11's scout-script helper already is.
    pub fn stored_onboarding_report(&self, fields: Value) {
        let repo_dir = self.repo_dir();
        let mut report = serde_json::to_value(ScoutReport {
            id: ONBOARDING_REPORT_ID.to_string(),
            kind: ScoutKind::Onboarding,
            run_id: None,
            question: "What is this repository, and how is it built and checked?".into(),
            summary: String::new(),
            files: Vec::new(),
            modules: Vec::new(),
            interfaces: Vec::new(),
            risks: Vec::new(),
            profile: None,
            route: Route {
                runtime: Runtime::Claude,
                model: "fake".into(),
                strength: Strength::Fast,
                effort: Effort::Low,
            },
            window_id: None,
            started_at: unix_now(),
            finished_at: unix_now(),
            tool_calls: 0,
            usage: Default::default(),
        })
        .unwrap();
        for (key, value) in fields.as_object().expect("report fields").clone() {
            report[key] = value;
        }
        let scouts = repo_dir.join("scouts");
        std::fs::create_dir_all(&scouts).unwrap();
        std::fs::write(
            scouts.join(format!("{ONBOARDING_REPORT_ID}.json")),
            report.to_string(),
        )
        .unwrap();
        let meta_path = repo_dir.join(daemon::profile::store::META_FILE);
        let mut meta: Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
        meta["report"] = json!(ONBOARDING_REPORT_ID);
        std::fs::write(meta_path, meta.to_string()).unwrap();
    }

    /// `anthrex run start --goal <goal> --dir <repo>` plus `flags`, waiting at most
    /// [`GOAL_WAIT`].
    pub fn start_goal(&self, goal: &str, flags: &[&str]) -> Output {
        let repo = self.repo.display().to_string();
        let mut args = vec!["run", "start", "--goal", goal];
        args.extend_from_slice(flags);
        args.extend_from_slice(&["--dir", &repo]);
        RunningCommand::start(&mut self.command(&args)).finish(GOAL_WAIT)
    }

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
            RunReply::Profile { reply, .. } => *reply,
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

/// Every JSON line of `path` (none when it does not exist).
fn jsonl(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect()
}
