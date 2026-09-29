//! Milestone 9's `anthrex run` commands (task M9.14): `--orchestrator` (decision 6),
//! hold verdicts (decision 28), `run message` and `run refresh` (decision 42h) and the
//! user's submit (decision 13). Every refusal of a request is the daemon's own text;
//! only the flags are checked here.

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use proto::{
    EditFile, HoldState, MessageKind, MessageTarget, OrchestratorChoice, PlanEdit, RunRequest,
    RunState, Runtime,
};

use super::Runs;

/// `--orchestrator`'s refusal of a value it cannot parse.
pub(super) const BAD_ORCHESTRATOR: &str =
    "--orchestrator: expected claude or codex, optionally :<model>";

/// `--orchestrator` beside `--plan`: a plan file's run has no orchestrator.
pub(super) const ORCHESTRATOR_WITH_PLAN: &str = "--orchestrator applies to --goal runs";

/// `claude`, `codex`, `claude:<model>` or `codex:<model>`; an empty model is the
/// runtime's default.
pub(super) fn parse_orchestrator(spec: &str) -> anyhow::Result<OrchestratorChoice> {
    let (runtime, model) = match spec.split_once(':') {
        Some((runtime, model)) => (runtime, Some(model)),
        None => (spec, None),
    };
    let runtime = match runtime {
        "claude" => Runtime::Claude,
        "codex" => Runtime::Codex,
        _ => anyhow::bail!(BAD_ORCHESTRATOR),
    };
    Ok(OrchestratorChoice {
        runtime,
        model: model.filter(|m| !m.is_empty()).map(str::to_string),
    })
}

/// `--orchestrator`, when given.
pub(super) fn orchestrator(spec: Option<&str>) -> anyhow::Result<Option<OrchestratorChoice>> {
    spec.map(parse_orchestrator).transpose()
}

/// A message's kind (decision 42a), as `--kind` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum KindArg {
    /// Context; the worker carries on
    Info,
    /// The plan or the code around the task changed; the worker says how it applied it
    Change,
    /// The worker finishes its current step, commits, and waits for the next message
    #[value(name = "stop_and_wait")]
    StopAndWait,
}

impl From<KindArg> for MessageKind {
    fn from(kind: KindArg) -> Self {
        match kind {
            KindArg::Info => MessageKind::Info,
            KindArg::Change => MessageKind::Change,
            KindArg::StopAndWait => MessageKind::StopAndWait,
        }
    }
}

/// `<task>[,<task>…]`, `stage:<n>` or `running`. A stage parses; the daemon refuses it
/// until milestone 9.1 (decision 42b).
pub(super) fn parse_target(to: &str) -> anyhow::Result<MessageTarget> {
    if to == "running" {
        return Ok(MessageTarget::Running);
    }
    if let Some(stage) = to.strip_prefix("stage:") {
        return match stage.parse() {
            Ok(n) => Ok(MessageTarget::Stage(n)),
            Err(_) => anyhow::bail!("expected stage:<n>, not {to}"),
        };
    }
    let tasks: Vec<String> = to
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    if tasks.is_empty() {
        anyhow::bail!("name a task, stage:<n> or running");
    }
    Ok(MessageTarget::Tasks(tasks))
}

/// The one `message` edit of `run message`: the text is the words joined with one space.
pub(super) fn message_edit(to: &str, kind: KindArg, words: &[String]) -> anyhow::Result<PlanEdit> {
    Ok(PlanEdit::Message {
        to: parse_target(to)?,
        text: words.join(" "),
        kind: kind.into(),
    })
}

/// `run message` and `run refresh` (decision 42h): M8a's `run edit` request with one
/// edit, the orchestrator's own edit and journal path. The daemon's reply is printed as
/// it is, with a line per refused recipient.
pub(super) async fn edit_one(runs: &mut Runs, run: &str, edit: PlanEdit) -> anyhow::Result<()> {
    let run_id = runs.resolve(run).await?;
    runs.done(RunRequest::Edit {
        run_id,
        edits: vec![edit],
        submit: false,
    })
    .await
}

/// `run edit`: the file's batch, then the user's submit (decision 13) with `--submit`.
pub(super) async fn edit(
    runs: &mut Runs,
    run: &str,
    file: Option<&Path>,
    submit: bool,
) -> anyhow::Result<()> {
    let run_id = runs.resolve(run).await?;
    let edits = match file {
        Some(file) => read_edits(file)?,
        None => Vec::new(),
    };
    runs.done(RunRequest::Edit {
        run_id,
        edits,
        submit,
    })
    .await
}

fn read_edits(file: &Path) -> anyhow::Result<Vec<PlanEdit>> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", file.display()))?;
    let edits: EditFile =
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    Ok(edits.edits)
}

/// `run approve`: a hold's verdict with `--hold`; without it M8a's plan approval,
/// except on a running run whose holds await the user, which names them.
pub(super) async fn approve(
    runs: &mut Runs,
    run: &str,
    hold: Option<String>,
) -> anyhow::Result<()> {
    let info = runs.resolve_info(run).await?;
    let run_id = info.run_id;
    if let Some(hold) = hold {
        return runs.done(RunRequest::ApproveHold { run_id, hold }).await;
    }
    let waiting: Vec<&str> = info
        .holds
        .iter()
        .filter(|h| h.state == HoldState::Awaiting)
        .map(|h| h.id.as_str())
        .collect();
    if info.state == RunState::Running && !waiting.is_empty() {
        anyhow::bail!(
            "run {run_id} has holds waiting for approval: {}; pass --hold <id>",
            waiting.join(", ")
        );
    }
    runs.done(RunRequest::Approve { run_id }).await
}

/// `run reject --hold`: no confirmation, since it cancels only held tasks that never
/// started (decision 28).
pub(super) async fn reject_hold(runs: &mut Runs, run: &str, hold: String) -> anyhow::Result<()> {
    let run_id = runs.resolve(run).await?;
    runs.done(RunRequest::RejectHold { run_id, hold }).await
}

/// `run promote [--orchestrator …]` (decision 29).
pub(super) async fn promote(
    runs: &mut Runs,
    run: &str,
    orchestrator: Option<OrchestratorChoice>,
) -> anyhow::Result<()> {
    let run_id = runs.resolve(run).await?;
    runs.done(RunRequest::Promote {
        run_id,
        orchestrator,
    })
    .await
}

/// `run start`'s flags, checked before anything reaches the daemon: `--orchestrator`
/// applies to a goal only, and must parse.
pub(super) fn start_orchestrator(
    plan: Option<&PathBuf>,
    spec: Option<&str>,
) -> anyhow::Result<Option<OrchestratorChoice>> {
    if plan.is_some() && spec.is_some() {
        anyhow::bail!(ORCHESTRATOR_WITH_PLAN);
    }
    orchestrator(spec)
}

#[cfg(test)]
#[path = "orch_tests.rs"]
mod tests;
