//! Milestone 9's additions to the run harness (brief, "Shared test helpers"), the part
//! task M9.13 first needs: a planned run started from a goal, its orchestrator window,
//! a client's typing into it, and what `fake-agent` read from its terminal. M9.16 adds
//! the rest.

use std::time::{Duration, Instant};

use proto::{ClientMsg, PlanEdit, RunInfo, RunReply, RunRequest, WindowInfo};
use serde_json::{Value, json};

use super::run_adapt::{ADAPT_FILES, STORED_PROFILE};
use super::run_harness::{RunHarness, connect};
use super::runtime;

/// `ORCH_WAIT` (brief, "Shared test helpers"; `docs/timing-budgets.md`): a sub-planner's
/// `timeout_secs = 120`, the longest a scripted orchestrator waits.
pub const ORCH_WAIT: Duration = Duration::from_secs(120);

/// The brief's test configuration for every M9 end-to-end test.
pub const ORCH_LINES: &str = "wake_quiet_secs = 1\nplanners.timeout_secs = 120\n";

/// A triage answer of scale `plan`.
pub fn triage_plan() -> Value {
    json!({"answer": {"kinds": ["code"], "scale": "plan", "reason": "several modules", "task": null}})
}

impl RunHarness {
    /// A harness running Claude deciders, with the brief's M9 test configuration plus
    /// `orchestrator` lines, the stored profile and `files` over its base files, and a
    /// triage that answers `plan` first.
    pub fn orch(orchestrator: &str, files: &[(&str, &str)]) -> RunHarness {
        let mut all: Vec<(&str, &str)> = ADAPT_FILES.to_vec();
        all.extend_from_slice(files);
        let h = RunHarness::adapt("claude", &format!("{ORCH_LINES}{orchestrator}"), &[], &all);
        h.stored_profile(STORED_PROFILE);
        h.decider("triage", 1, triage_plan());
        h
    }

    /// `anthrex run start --goal <goal> <extra…>`; the run id from stdout.
    pub fn start_goal_id(&self, goal: &str, extra: &[&str]) -> String {
        let out = self.start_goal(goal, extra);
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert!(
            out.status.success() && !stdout.is_empty() && !stdout.contains('\n'),
            "exit {:?}\nstdout: {stdout}\nstderr: {}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr),
            self.log_tail()
        );
        stdout
    }

    /// Waits, at most [`ORCH_WAIT`], until run `run`'s orchestrator is live in a window;
    /// its id.
    pub fn orchestrator_window(&self, run: &str) -> u32 {
        let live = |r: &RunInfo| {
            r.orchestrator
                .as_ref()
                .is_some_and(|o| o.live && o.window_id.is_some())
        };
        let info = self.wait_run(run, live, ORCH_WAIT);
        info.orchestrator.unwrap().window_id.unwrap()
    }

    /// The listed window `id`.
    pub fn window(&self, id: u32) -> Option<WindowInfo> {
        self.windows().into_iter().find(|w| w.id == id)
    }

    /// Waits, at most `wait`, until window `id` satisfies `pred`.
    pub fn wait_window(
        &self,
        id: u32,
        what: &str,
        pred: impl Fn(&WindowInfo) -> bool,
        wait: Duration,
    ) {
        self.wait_windows(what, |ws| ws.iter().any(|w| w.id == id && pred(w)), wait);
    }

    /// A real client's keystrokes into window `id` (`ClientMsg::Input`).
    pub fn type_into(&self, id: u32, bytes: &[u8]) {
        let socket = self.socket();
        let bytes = bytes.to_vec();
        runtime().block_on(async move {
            let mut stream = connect(&socket).await;
            proto::write_frame(
                &mut stream,
                &ClientMsg::Input {
                    window_id: id,
                    bytes,
                },
            )
            .await
            .unwrap();
            // A list request after it: once answered, the input was handled.
            proto::write_frame(&mut stream, &ClientMsg::ListWindows)
                .await
                .unwrap();
            loop {
                match proto::read_frame::<_, proto::DaemonMsg>(&mut stream).await {
                    Ok(Some(proto::DaemonMsg::WindowsChanged { .. })) => return,
                    Ok(Some(_)) => {}
                    other => panic!("connection ended: {other:?}"),
                }
            }
        });
    }

    /// Every message `fake-agent`'s `read_message` took from the terminal of the
    /// session claimed as `name` (`<io>/<name>.stdin`, one JSON line each).
    pub fn read_messages(&self, name: &str) -> Vec<Value> {
        self.io_lines(name, "stdin")
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v.get("raw").is_some())
            .collect()
    }

    /// Waits, at most `wait`, until `name` has read `n` messages; when the `n`th came.
    pub fn wait_messages(&self, name: &str, n: usize, wait: Duration) -> Instant {
        let deadline = Instant::now() + wait;
        loop {
            if self.read_messages(name).len() >= n {
                return Instant::now();
            }
            assert!(
                Instant::now() < deadline,
                "{name} read {:?}, not {n} messages, within {wait:?}\n{}",
                self.read_messages(name),
                self.log_tail()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// A user edit adding task `id` in its own module: a wake note (decision 39).
    pub fn add_task(&self, run: &str, id: &str) {
        let task = json!({
            "id": id, "title": format!("Task {id}"), "brief": "Do it.",
            "acceptance": ["done"], "owns": [format!("{id}.txt")], "size": "S",
            "test_mode": "check", "test_mode_reason": "a text file",
        });
        let edit: PlanEdit =
            serde_json::from_value(json!({"op": "add_task", "task": task})).unwrap();
        match self.request(RunRequest::Edit {
            run_id: run.to_string(),
            edits: vec![edit],
            submit: false,
        }) {
            RunReply::Done { .. } => {}
            other => panic!("the edit was refused: {other:?}"),
        }
    }

    /// The run's `run.json`.
    pub fn run_json(&self, run: &str) -> Value {
        let path = self.data().join("runs").join(run).join("run.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }
}

/// An orchestrator script's first steps: a turn begins (the window is `Working`), and
/// waits for a line the test types, so the test decides when the turn ends.
pub fn working_until_typed() -> Vec<Value> {
    vec![
        json!({"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}}),
        json!({"read_line": true}),
    ]
}

/// `read_message` with no timeout.
pub fn read_message() -> Value {
    json!({"read_message": {}})
}

/// The bracketed paste of `text` followed by the `\r` that submits it.
pub fn framed(text: &str) -> String {
    format!("\x1b[200~{text}\x1b[201~\r")
}
