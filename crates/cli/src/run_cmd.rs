//! `anthrex run …` (M8a.23): the one-shot commands that drive the daemon's run API
//! (`RunRequest`/`RunReply`, decisions 20 and 53, the brief's CLI section). Every request
//! waits [`RUN_REQUEST_TIMEOUT`] ([`FINISH_REQUEST_TIMEOUT`] for accept and discard); every `Refused` prints the daemon's message and exits 1.

mod finish;
mod status;

use finish::{accept, confirm_id};

use crate::client::CliClient;
use clap::{Args, Subcommand};
use proto::{
    ClientMsg, DaemonMsg, EditFile, FinishAction, RunInfo, RunReply, RunRequest, RunsSnapshot,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Every run request's reply bound: `run start`'s preflight alone is six git calls at the
/// 60 s default `git_timeout_secs`, so a shorter bound could expire on a request the
/// daemon is still answering.
pub const RUN_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// `run accept` and `run discard`'s reply bound (ruling T23-I1): the daemon's merge runs
/// under `ACCEPT_MERGE_TIMEOUT` and is never shortened (the user's hooks and signing run
/// inside it); 60 s more covers the request's own git reads and the clean-up's first
/// calls. A CLI that gave up sooner would report a failure for an accept the daemon then
/// completes.
pub const FINISH_REQUEST_TIMEOUT: Duration =
    daemon::run::git::ACCEPT_MERGE_TIMEOUT.saturating_add(Duration::from_secs(60));

/// The reply bound for `request`.
fn request_timeout(request: &RunRequest) -> Duration {
    match request {
        RunRequest::Finish { .. } => FINISH_REQUEST_TIMEOUT,
        _ => RUN_REQUEST_TIMEOUT,
    }
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[command(subcommand)]
    command: RunCommand,
}

#[derive(Subcommand, Debug)]
enum RunCommand {
    /// Start a run from a plan file and print its id
    Start {
        #[arg(long)]
        plan: PathBuf,
        /// Approve the plan at once
        #[arg(long)]
        yes: bool,
        /// Accept the repository's tracked agent settings (decision 53)
        #[arg(long)]
        trust_project: bool,
    },
    /// Show every run, or one, newest first
    Status {
        run: Option<String>,
        /// Print the snapshot as pretty JSON
        #[arg(long)]
        json: bool,
    },
    /// Approve a run's plan
    Approve { run: String },
    /// Reject a run's plan: remove its worktrees and delete its branches
    Reject {
        run: String,
        /// The run id, instead of typing it
        #[arg(long)]
        confirm: Option<String>,
    },
    /// Apply a file of plan edits
    Edit {
        run: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Retry a task
    Retry { run: String, task: String },
    /// Merge a task without its review's approval
    Override {
        run: String,
        task: String,
        #[arg(long)]
        reason: String,
    },
    /// Cancel a run
    Cancel { run: String },
    /// Resume a paused or halted run
    Resume {
        run: String,
        /// Take the base branch's and run branch's current heads as the new baseline
        #[arg(long)]
        rebaseline: bool,
    },
    /// Merge a complete run into its base branch
    Accept {
        run: String,
        /// Do not ask before merging (a moved base still needs --base)
        #[arg(long)]
        yes: bool,
        /// Merge onto a moved base whose listed head is this sha
        #[arg(long)]
        base: Option<String>,
    },
    /// Discard a run: remove its worktrees and delete its branches
    Discard {
        run: String,
        /// The run id, instead of typing it
        #[arg(long)]
        confirm: Option<String>,
    },
}

/// Runs one `anthrex run` command; any error is printed as it is and exits 1.
pub async fn main(args: RunArgs, socket: PathBuf, dir: Option<PathBuf>) -> anyhow::Result<()> {
    if let Err(error) = dispatch(args.command, &socket, dir).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
    Ok(())
}

async fn dispatch(command: RunCommand, socket: &Path, dir: Option<PathBuf>) -> anyhow::Result<()> {
    if let RunCommand::Start {
        plan,
        yes,
        trust_project,
    } = command
    {
        return start(socket, dir, &plan, yes, trust_project).await;
    }
    let mut runs = Runs::connect(socket).await?;
    match command {
        RunCommand::Start { .. } => unreachable!("handled above"),
        RunCommand::Status { run, json } => {
            let mut snapshot = runs.list().await?;
            if let Some(run) = run {
                let id = resolve_run(&snapshot.runs, &run).map_err(anyhow::Error::msg)?;
                snapshot.runs.retain(|r| r.run_id == id);
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else if snapshot.runs.is_empty() {
                eprintln!("no runs");
            } else {
                print!("{}", status::render(&snapshot.runs));
            }
            Ok(())
        }
        RunCommand::Approve { run } => {
            let run_id = runs.resolve(&run).await?;
            runs.done(RunRequest::Approve { run_id }).await
        }
        RunCommand::Reject { run, confirm } => {
            let run_id = runs.resolve(&run).await?;
            let prompt =
                format!("reject run {run_id}: remove its worktrees and delete its branches?");
            confirm_id(&run_id, confirm, &prompt).await?;
            runs.done(RunRequest::Reject { run_id }).await
        }
        RunCommand::Edit { run, file } => {
            let run_id = runs.resolve(&run).await?;
            let text = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", file.display()))?;
            let edits: EditFile =
                toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
            runs.done(RunRequest::Edit {
                run_id,
                edits: edits.edits,
            })
            .await
        }
        RunCommand::Retry { run, task } => {
            let run_id = runs.resolve(&run).await?;
            runs.done(RunRequest::Retry {
                run_id,
                task_id: task,
            })
            .await
        }
        RunCommand::Override { run, task, reason } => {
            let run_id = runs.resolve(&run).await?;
            runs.done(RunRequest::Override {
                run_id,
                task_id: task,
                reason,
            })
            .await
        }
        RunCommand::Cancel { run } => {
            let run_id = runs.resolve(&run).await?;
            runs.done(RunRequest::Cancel { run_id }).await
        }
        RunCommand::Resume { run, rebaseline } => {
            let run_id = runs.resolve(&run).await?;
            runs.done(RunRequest::Resume { run_id, rebaseline }).await
        }
        RunCommand::Accept { run, yes, base } => {
            let info = runs.resolve_info(&run).await?;
            accept(&mut runs, &info, yes, base.as_deref()).await
        }
        RunCommand::Discard { run, confirm } => {
            let run_id = runs.resolve(&run).await?;
            if confirm.is_none() {
                // The daemon's own wording of what a discard does.
                let prompt = match runs.finish(&run_id, FinishAction::Discard, None).await? {
                    RunReply::ConfirmNeeded { prompt, .. } => prompt,
                    other => return print_outcome(other),
                };
                confirm_id(&run_id, None, &prompt).await?;
            } else {
                confirm_id(&run_id, confirm, "").await?;
            }
            let reply = runs
                .finish(&run_id, FinishAction::Discard, Some(run_id.clone()))
                .await?;
            print_outcome(reply)
        }
    }
}

/// `run start`: the daemon auto-started, the id on stdout; on stderr the plan's
/// protected-path warnings (decision 56, rule 6.protected), then the next step.
async fn start(
    socket: &Path,
    dir: Option<PathBuf>,
    plan: &Path,
    yes: bool,
    trust_project: bool,
) -> anyhow::Result<()> {
    let plan_toml = std::fs::read_to_string(plan)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", plan.display()))?;
    let dir = crate::resolve_dir(dir)?;
    tui::spawn::ensure_daemon(&std::env::current_exe()?, socket).await?;
    let mut runs = Runs::connect(socket).await?;
    let reply = runs
        .request(RunRequest::Start {
            plan_toml,
            dir,
            yes,
            trust_project,
        })
        .await?;
    let run_id = match reply {
        RunReply::Started { run_id, .. } => run_id,
        other => return print_outcome(other),
    };
    println!("{run_id}");
    let run = runs
        .list()
        .await?
        .runs
        .into_iter()
        .find(|r| r.run_id == run_id);
    let mut err = String::new();
    if let Some(run) = &run {
        for task in &run.tasks {
            for note in task
                .notes
                .iter()
                .filter(|n| n.contains("(rule 6.protected)"))
            {
                err.push_str(&format!("warning: {}: {note}\n", task.id));
            }
        }
    }
    if yes {
        err.push_str(&format!("watch with: anthrex run status {run_id}\n"));
    } else {
        err.push_str(&format!("approve with: anthrex run approve {run_id}\n"));
        if let Some(run) = &run {
            err.push_str(&status::run_block(run));
        }
    }
    eprint!("{err}");
    Ok(())
}

/// `Done` prints its message; `Refused` and anything else is the command's error.
fn print_outcome(reply: RunReply) -> anyhow::Result<()> {
    match reply {
        RunReply::Done { message, .. } => {
            if !message.is_empty() {
                println!("{message}");
            }
            Ok(())
        }
        RunReply::Refused { message, .. } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// `run` names a run by its id, or by a prefix or suffix of it matching exactly one run.
pub fn resolve_run(runs: &[RunInfo], run: &str) -> Result<String, String> {
    if run.is_empty() {
        return Err("no run named ''".to_string());
    }
    if let Some(exact) = runs.iter().find(|r| r.run_id == run) {
        return Ok(exact.run_id.clone());
    }
    let matches: Vec<&str> = runs
        .iter()
        .map(|r| r.run_id.as_str())
        .filter(|id| id.starts_with(run) || id.ends_with(run))
        .collect();
    match matches.as_slice() {
        [] => Err(format!("no run matches '{run}'")),
        [one] => Ok(one.to_string()),
        many => Err(format!(
            "'{run}' matches more than one run: {}",
            many.join(", ")
        )),
    }
}

/// One connection to the daemon for one command.
pub(crate) struct Runs {
    client: CliClient,
}

impl Runs {
    async fn connect(socket: &Path) -> anyhow::Result<Self> {
        Ok(Runs {
            client: CliClient::connect(socket).await?,
        })
    }

    async fn request(&mut self, request: RunRequest) -> anyhow::Result<RunReply> {
        let timeout = request_timeout(&request);
        match self
            .client
            .request_with_timeout(ClientMsg::Run(request), timeout)
            .await?
        {
            DaemonMsg::Run(reply) => Ok(reply),
            DaemonMsg::Error { message, .. } => anyhow::bail!(message),
            other => anyhow::bail!("unexpected reply: {other:?}"),
        }
    }

    /// A request answered by `Done` (printed) or `Refused` (the error).
    async fn done(&mut self, request: RunRequest) -> anyhow::Result<()> {
        let reply = self.request(request).await?;
        print_outcome(reply)
    }

    pub(crate) async fn finish(
        &mut self,
        run_id: &str,
        action: FinishAction,
        confirm: Option<String>,
    ) -> anyhow::Result<RunReply> {
        self.request(RunRequest::Finish {
            run_id: run_id.to_string(),
            action,
            confirm,
        })
        .await
    }

    async fn list(&mut self) -> anyhow::Result<RunsSnapshot> {
        match self.request(RunRequest::List).await? {
            RunReply::Snapshot(snapshot) => Ok(snapshot),
            other => print_outcome(other).and_then(|_| anyhow::bail!("no snapshot")),
        }
    }

    async fn resolve_info(&mut self, run: &str) -> anyhow::Result<RunInfo> {
        let runs = self.list().await?.runs;
        let id = resolve_run(&runs, run).map_err(anyhow::Error::msg)?;
        Ok(runs.into_iter().find(|r| r.run_id == id).expect("resolved"))
    }

    async fn resolve(&mut self, run: &str) -> anyhow::Result<String> {
        Ok(self.resolve_info(run).await?.run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::finish::base_matches;
    use super::{RUN_REQUEST_TIMEOUT, request_timeout, resolve_run, status};
    use daemon::run::git::ACCEPT_MERGE_TIMEOUT;
    use proto::{FinishAction, RunInfo, RunRequest};
    use std::time::Duration;

    /// Ruling T23-I1: accept and discard outwait the daemon's own merge bound, whatever
    /// it becomes; every other request keeps `RUN_REQUEST_TIMEOUT`.
    #[test]
    fn finish_requests_outwait_the_accept_merge() {
        for action in [FinishAction::Accept, FinishAction::Discard] {
            let finish = RunRequest::Finish {
                run_id: "r".into(),
                action,
                confirm: None,
            };
            assert!(
                request_timeout(&finish) >= ACCEPT_MERGE_TIMEOUT + Duration::from_secs(60),
                "{action:?}: {:?}",
                request_timeout(&finish)
            );
        }
        assert_eq!(request_timeout(&RunRequest::List), RUN_REQUEST_TIMEOUT);
        let approve = RunRequest::Approve { run_id: "r".into() };
        assert_eq!(request_timeout(&approve), RUN_REQUEST_TIMEOUT);
    }

    /// Ruling T23-I2: `--base` takes the listed head or any hex prefix of it of at least
    /// seven characters.
    #[test]
    fn base_accepts_the_listed_head_or_a_prefix_of_it() {
        let to = "0a04f693cbc5ae9b25fd8a2f3c5ad94ae2252ecb";
        for given in [to, "0a04f69", "0a04f693cbc5", "0A04F69"] {
            assert!(base_matches(given, to), "{given}");
        }
        for given in [
            "0a04f6",
            "",
            "0a04f6x",
            "1a04f69",
            "0a04f693cbc5ae9b25fd8a2f3c5ad94ae2252ecb0",
        ] {
            assert!(!base_matches(given, to), "{given}");
        }
    }

    fn runs(ids: &[&str]) -> Vec<RunInfo> {
        ids.iter()
            .map(|id| RunInfo {
                run_id: id.to_string(),
                ..status::tests::example()
            })
            .collect()
    }

    #[test]
    fn resolve_run_by_id_suffix_and_prefix() {
        let all = runs(&["add-reset-3f9a", "add-login-77b0", "fix-reset-3f9b"]);
        assert_eq!(
            resolve_run(&all, "add-reset-3f9a").unwrap(),
            "add-reset-3f9a"
        );
        assert_eq!(resolve_run(&all, "3f9a").unwrap(), "add-reset-3f9a");
        assert_eq!(resolve_run(&all, "add-l").unwrap(), "add-login-77b0");
        assert_eq!(resolve_run(&all, "fix").unwrap(), "fix-reset-3f9b");
        assert_eq!(
            resolve_run(&all, "add").unwrap_err(),
            "'add' matches more than one run: add-reset-3f9a, add-login-77b0"
        );
        assert_eq!(
            resolve_run(&all, "reset").unwrap_err(),
            "no run matches 'reset'"
        );
        assert_eq!(
            resolve_run(&all, "nope").unwrap_err(),
            "no run matches 'nope'"
        );
        // An exact id wins over a longer id it prefixes.
        let nested = runs(&["a-1", "a-1b"]);
        assert_eq!(resolve_run(&nested, "a-1").unwrap(), "a-1");
    }
}
