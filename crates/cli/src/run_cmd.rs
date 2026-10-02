//! `anthrex run …` (M8a.23): the one-shot commands that drive the daemon's run API
//! (`RunRequest`/`RunReply`, decisions 20 and 53, the brief's CLI section). Every request
//! waits [`RUN_REQUEST_TIMEOUT`] ([`FINISH_REQUEST_TIMEOUT`] for accept and discard); every `Refused` prints the daemon's message and exits 1.

mod adapt;
mod delivery;
mod finish;
mod orch;
mod status;
mod status_orch;

use finish::{accept, confirm_id};

use crate::client::CliClient;
use clap::{Args, Subcommand};
use proto::{
    ClientMsg, DaemonMsg, FinishAction, PlanEdit, RunInfo, RunReply, RunRequest, RunsSnapshot,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Every run request's reply bound: `run start`'s preflight alone is six git calls at the
/// 60 s default `git_timeout_secs`, so a shorter bound could expire on a request the
/// daemon is still answering.
pub const RUN_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// `run start`'s reply bound (task M9.2.12 fix round 1, I2): M8a's git preflight
/// ([`RUN_REQUEST_TIMEOUT`]'s term), then in `pr` mode the host preflight, bounded by the
/// daemon's own `PREFLIGHT_BOUND` (not a copy), and a 30 s margin for the run's build
/// after them. A CLI that gave up sooner would report a timeout for a run the daemon then
/// starts.
pub const RUN_START_TIMEOUT: Duration = RUN_REQUEST_TIMEOUT
    .saturating_add(daemon::host::PREFLIGHT_BOUND)
    .saturating_add(Duration::from_secs(30));

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
        RunRequest::Start { .. } => RUN_START_TIMEOUT,
        RunRequest::StartGoal { .. } => adapt::GOAL_REQUEST_TIMEOUT,
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
    /// Start a run from a plan file, or a goal triage may put on the fast path, and print its id
    #[command(group(clap::ArgGroup::new("source").required(true).args(["plan", "goal"])))]
    Start {
        #[arg(long)]
        plan: Option<PathBuf>,
        /// A goal for the triage decider; one small task runs at once, with no plan gate
        #[arg(long)]
        goal: Option<String>,
        /// Approve the plan at once
        #[arg(long)]
        yes: bool,
        /// Accept the repository's tracked agent settings (decision 53)
        #[arg(long)]
        trust_project: bool,
        /// Where this platform cannot confine checks, proofs and setup (which run code
        /// the workers wrote), run them unconfined anyway
        #[arg(long)]
        unconfined_checks: bool,
        /// A goal's orchestrator: claude or codex, optionally :<model>
        #[arg(long, value_name = "RUNTIME[:MODEL]")]
        orchestrator: Option<String>,
        /// Deliver by pull request (pr) or by run accept (local); the profile's by default
        #[arg(long, value_name = "pr|local")]
        delivery: Option<String>,
    },
    /// Show each stage's pull request: its state, CI, threads and fix tasks
    Prs(delivery::PrsArgs),
    /// Run tier 3 on a stage now, and open its pull request once it is green
    Deliver(delivery::DeliverArgs),
    /// Stop or resume watching a run's pull requests
    Watch(delivery::WatchArgs),
    #[command(hide = true)]
    FakeGithub(delivery::FakeGithubArgs),
    /// Show every run, or one, newest first
    Status {
        run: Option<String>,
        /// Print the snapshot as pretty JSON
        #[arg(long)]
        json: bool,
    },
    /// Approve a run's plan, or with --hold one hold of work added after it
    Approve {
        run: String,
        /// The hold to approve (`anthrex run status` lists them)
        #[arg(long)]
        hold: Option<String>,
    },
    /// Reject a run's plan: remove its worktrees and delete its branches; with --hold,
    /// cancel that hold's tasks, none of which has started
    Reject {
        run: String,
        /// The run id, instead of typing it
        #[arg(long, conflicts_with = "hold")]
        confirm: Option<String>,
        /// The hold to reject
        #[arg(long)]
        hold: Option<String>,
    },
    /// Apply a file of plan edits, or submit a plan being written
    #[command(group(clap::ArgGroup::new("what").required(true).multiple(true).args(["file", "submit"])))]
    Edit {
        run: String,
        #[arg(long)]
        file: Option<PathBuf>,
        /// After the file's edits, submit the plan for approval (a run being planned)
        #[arg(long)]
        submit: bool,
    },
    /// Send a message to a task's worker, delivered when its current turn ends
    Message {
        run: String,
        /// info: context; change: the plan or code around the task changed;
        /// stop_and_wait: finish the current step, commit and wait (give it before TO)
        #[arg(long, value_enum, default_value = "info")]
        kind: orch::KindArg,
        /// TO is a task id (or several, comma-separated), stage:<n>, or running for
        /// every task with a live worker. TEXT is every word after TO, flags included,
        /// joined with one space
        #[arg(
            required = true,
            num_args = 2..,
            value_names = ["TO", "TEXT"],
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        to_and_text: Vec<String>,
    },
    /// Merge the run's latest merged work into a task's branch at its next turn
    /// boundary; then tell it why with anthrex run message --kind change
    Refresh { run: String, task: String },
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
    /// Promote a fast-path run to a planned run with an orchestrator
    Promote {
        run: String,
        /// Its orchestrator: claude or codex, optionally :<model>
        #[arg(long, value_name = "RUNTIME[:MODEL]")]
        orchestrator: Option<String>,
    },
    /// Summarise this repository's run history, by task class
    Stats {
        /// Print the summary as pretty JSON
        #[arg(long)]
        json: bool,
    },
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
        /// Merge only if the base is at this sha (a moved base's listed head, or the run's own base)
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
        eprintln!("{}", status::printable(&error.to_string()));
        std::process::exit(1);
    }
    Ok(())
}

async fn dispatch(command: RunCommand, socket: &Path, dir: Option<PathBuf>) -> anyhow::Result<()> {
    if let RunCommand::Start {
        plan,
        goal,
        yes,
        trust_project,
        unconfined_checks,
        orchestrator,
        delivery,
    } = command
    {
        let delivery = delivery::mode(delivery.as_deref())?;
        let choice = orch::start_orchestrator(plan.as_ref(), orchestrator.as_deref())?;
        let flags = (yes, trust_project, unconfined_checks);
        return match (plan, goal) {
            (Some(plan), _) => start(socket, dir, &plan, flags, delivery).await,
            (None, goal) => {
                let goal = goal.unwrap_or_default();
                adapt::start_goal(socket, dir, goal, flags, (choice, delivery)).await
            }
        };
    }
    // Decision 14: the fake GitHub's control, which talks to no daemon.
    if let RunCommand::FakeGithub(args) = command {
        return delivery::fake_github(args);
    }
    // Every flag is checked before the daemon is asked anything.
    let promote_choice = match &command {
        RunCommand::Promote { orchestrator, .. } => orch::orchestrator(orchestrator.as_deref())?,
        _ => None,
    };
    let message = match &command {
        RunCommand::Message {
            kind, to_and_text, ..
        } => Some(orch::message_edit(
            &to_and_text[0],
            *kind,
            &to_and_text[1..],
        )?),
        _ => None,
    };
    let mut runs = Runs::connect(socket).await?;
    match command {
        RunCommand::Start { .. } | RunCommand::FakeGithub(_) => unreachable!("handled above"),
        RunCommand::Prs(args) => delivery::prs(&mut runs, args).await,
        RunCommand::Deliver(args) => delivery::deliver(&mut runs, args).await,
        RunCommand::Watch(args) => delivery::watch(&mut runs, args).await,
        RunCommand::Status { run, json } => {
            let mut snapshot = runs.list().await?;
            if let Some(run) = run {
                let id = resolve_run(&snapshot.runs, &run).map_err(anyhow::Error::msg)?;
                snapshot.runs.retain(|r| r.run_id == id);
            }
            // Task M9.2.14 fix round 1 (m1): everything `run status` prints is sanitised.
            if json {
                let text = serde_json::to_string_pretty(&snapshot)?;
                println!("{}", status::printable(&text));
            } else if snapshot.runs.is_empty() {
                eprintln!("no runs");
            } else {
                let text = status::render(&snapshot.runs, tui::local_utc_offset_secs());
                print!("{}", status::printable(&text));
            }
            Ok(())
        }
        RunCommand::Approve { run, hold } => orch::approve(&mut runs, &run, hold).await,
        RunCommand::Reject {
            run,
            hold: Some(hold),
            ..
        } => orch::reject_hold(&mut runs, &run, hold).await,
        RunCommand::Reject { run, confirm, .. } => {
            let run_id = runs.resolve(&run).await?;
            let prompt =
                format!("reject run {run_id}: remove its worktrees and delete its branches?");
            confirm_id(&run_id, confirm, &prompt).await?;
            runs.done(RunRequest::Reject { run_id }).await
        }
        RunCommand::Edit { run, file, submit } => {
            orch::edit(&mut runs, &run, file.as_deref(), submit).await
        }
        RunCommand::Message { run, .. } => {
            let edit = message.expect("parsed above");
            orch::edit_one(&mut runs, &run, edit).await
        }
        RunCommand::Refresh { run, task } => {
            let edit = PlanEdit::Refresh { task_id: task };
            orch::edit_one(&mut runs, &run, edit).await
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
        RunCommand::Promote { run, .. } => orch::promote(&mut runs, &run, promote_choice).await,
        RunCommand::Stats { json } => adapt::stats(&mut runs, dir, json).await,
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
    (yes, trust_project, unconfined_checks): (bool, bool, bool),
    delivery: Option<proto::DeliveryMode>,
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
            unconfined_checks,
            delivery,
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
            err.push_str(&status::run_block(run, tui::local_utc_offset_secs()));
        }
    }
    eprint!("{}", status::printable(&err));
    Ok(())
}

/// `Done` prints its message; `Refused` and anything else is the command's error.
fn print_outcome(reply: RunReply) -> anyhow::Result<()> {
    match reply {
        RunReply::Done { message, .. } => {
            if !message.is_empty() {
                println!("{}", status::printable(&message));
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
mod tests;
