//! Milestone 9.9 decisions 14 and 24 (OFA §4.3): answering the orchestrator's `ask_user`
//! — a number key in the Alerts view, or Enter typed into its window. Pure: no I/O.

use super::alerts::AlertKey;
use super::{App, Effect};
use proto::{ClientMsg, RunInfo, RunRequest};

/// `choice` as the answer to `run`'s pending ask (`None`: answered in the window); none
/// when it asks nothing.
pub fn answer_request(run: &RunInfo, choice: Option<u32>) -> Option<RunRequest> {
    let ask = run.orchestrator.as_ref()?.ask.as_ref()?;
    Some(RunRequest::AnswerAsk {
        run_id: run.run_id.clone(),
        ask: ask.id,
        choice,
    })
}

/// Milestone 9.9 (OFA §4.6): the Alerts view footer's line for `run` — its title, what its
/// orchestrator handled, and where to read it; none when it handled nothing.
pub fn handled_line(run: &RunInfo) -> Option<String> {
    let n = run.orchestrator.as_ref()?.handled_total;
    (n > 0).then(|| {
        format!(
            "{}: orchestrator handled {n} · o on its alerts opens the run",
            crate::safe_text::one_line(crate::tree::run_title(run))
        )
    })
}

impl App {
    /// Keys typed into a focused PTY window: the input, and where that window is an
    /// orchestrator with a pending ask and the key is Enter, the answer "in the window"
    /// (decision 14: the user's reply is on its way, whatever it says).
    pub(super) fn forward(&mut self, window_id: u32, bytes: Vec<u8>) -> Vec<Effect> {
        let enter = bytes.contains(&b'\r');
        let mut effects = vec![Effect::Send(ClientMsg::Input { window_id, bytes })];
        if enter
            && let Some(request) = (self.runs.runs.iter())
                .find(|run| {
                    run.orchestrator
                        .as_ref()
                        .is_some_and(|orch| orch.window_id == Some(window_id))
                })
                .and_then(|run| answer_request(run, None))
        {
            effects.push(Effect::Send(ClientMsg::Run(request)));
        }
        effects
    }

    /// `1`-`9` in the Alerts view on a selected ask: option `n`, or a toast.
    pub(super) fn answer_key(&mut self, key: &AlertKey, n: u32) -> Vec<Effect> {
        let AlertKey::OrchestratorAsks(run_id) = key else {
            return vec![];
        };
        let options = (self.runs.runs.iter())
            .find(|run| run.run_id == *run_id)
            .and_then(|run| run.orchestrator.as_ref()?.ask.as_ref())
            .map_or(0, |ask| ask.options.len());
        if n as usize > options {
            self.toast(format!("no option {n}"));
            return vec![];
        }
        let request = (self.runs.runs.iter())
            .find(|run| run.run_id == *run_id)
            .and_then(|run| answer_request(run, Some(n - 1)));
        request
            .map(|request| vec![Effect::Send(ClientMsg::Run(request))])
            .unwrap_or_default()
    }
}
