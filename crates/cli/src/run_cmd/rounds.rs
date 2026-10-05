//! Milestone 9.3's `anthrex run` parts (KG §7, design decision 31): `run iterate`'s
//! request and `run status`'s round lines. `run start --continue` is `adapt::start_goal`'s.

use std::path::PathBuf;

use daemon::run::orch::contract_rounds::REQUEST_TOO_LONG;
use proto::{GOAL_MAX_CHARS, RoundOrigin, RoundOutcome, RunInfo, RunRequest, safe_text};

use super::Runs;

/// `run iterate`'s request, from exactly one source (clap's group): the text, `-` for
/// stdin, or `--file`. Read before the daemon is asked anything, so a request longer
/// than the daemon takes is refused here, in the daemon's words, before connecting. The
/// text is sent as it was read; the daemon cleans it.
pub(super) fn request_text(
    text: &Option<String>,
    file: &Option<PathBuf>,
) -> anyhow::Result<String> {
    let read = match (text.as_deref(), file) {
        (Some("-"), _) => std::io::read_to_string(std::io::stdin())
            .map_err(|e| anyhow::anyhow!("cannot read the request from stdin: {e}"))?,
        (Some(text), _) => text.to_string(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?,
        (None, None) => anyhow::bail!("no request: give a text, - or --file"),
    };
    if read.chars().count() > GOAL_MAX_CHARS {
        anyhow::bail!(REQUEST_TOO_LONG);
    }
    Ok(read)
}

/// `run iterate <run>`: `Done`'s message on stdout; every refusal (decision 9's, D17's)
/// is the daemon's text, the command's error (exit 1).
/// Milestone 9.6 (decision 28): `design` is `--design`'s, `None` the daemon's default.
pub(super) async fn iterate(
    runs: &mut Runs,
    run: &str,
    goal: String,
    design: Option<proto::RoundDesign>,
) -> anyhow::Result<()> {
    let run = runs.resolve(run).await?;
    runs.done(RunRequest::Iterate { run, goal, design }).await
}

/// KG §7: for a run with more than one round, `round <n> of <total>` and one line a
/// round, `round <n> · <origin> · <outcome or running> · <summary head or ->`, after
/// the goal; nothing for a one-round run. The summary head is drawn on its one line
/// without hidden carriers (decision 33).
pub(super) fn status_lines(run: &RunInfo) -> String {
    if run.rounds.len() < 2 {
        return String::new();
    }
    let mut out = format!("  round {} of {}\n", run.round, run.rounds.len());
    for r in &run.rounds {
        let origin = match r.origin {
            RoundOrigin::User => "user",
            RoundOrigin::Orchestrator => "orchestrator",
        };
        let outcome = match r.outcome {
            None => "running",
            Some(RoundOutcome::Completed) => "completed",
            Some(RoundOutcome::Rejected) => "rejected",
            Some(RoundOutcome::Cancelled) => "cancelled",
        };
        let summary = r
            .summary_head
            .as_deref()
            .map_or("-".into(), safe_text::one_line);
        out.push_str(&format!(
            "  round {} · {origin} · {outcome} · {summary}\n",
            r.n
        ));
    }
    out
}

#[cfg(test)]
#[path = "rounds_tests.rs"]
mod tests;
