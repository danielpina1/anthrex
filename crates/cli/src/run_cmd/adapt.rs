//! Milestone 8b's `anthrex run` commands: `run start --goal` (decision 22), `run
//! promote` (decision 25) and `run stats` (decision 35).

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{RunReply, RunRequest};

use super::{Runs, print_outcome};

/// `run start --goal`'s reply bound (decision 22): [`super::RUN_REQUEST_TIMEOUT`] for
/// M8a's start path, the largest configurable `deciders.timeout_secs` (600 s) for the
/// triage call, and 30 s for the request's own preflight and `git ls-files`.
pub const GOAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(810);

/// `run start --goal`: on the fast path the run id on stdout and triage's message on
/// stderr, and so on the plan and large paths with the planned message (milestone 9
/// decision 26); any refusal is the command's error (exit 1). `orchestrator` is
/// `--orchestrator`'s choice (decision 6).
pub(super) async fn start_goal(
    socket: &Path,
    dir: Option<PathBuf>,
    goal: String,
    (yes, trust_project, unconfined_checks): (bool, bool, bool),
    orchestrator: Option<proto::OrchestratorChoice>,
) -> anyhow::Result<()> {
    let dir = crate::resolve_dir(dir)?;
    tui::spawn::ensure_daemon(&std::env::current_exe()?, socket).await?;
    let mut runs = Runs::connect(socket).await?;
    let reply = runs
        .request(RunRequest::StartGoal {
            goal,
            dir,
            yes,
            trust_project,
            unconfined_checks,
            orchestrator,
        })
        .await?;
    match reply {
        RunReply::Triaged {
            run_id: Some(run_id),
            message,
            ..
        } => {
            println!("{run_id}");
            eprintln!("{}", super::status::printable(&message));
            Ok(())
        }
        RunReply::Triaged { message, .. } => anyhow::bail!(message),
        other => print_outcome(other),
    }
}

/// `run stats`: the daemon's summary of the history of `dir`'s repository, as
/// `stats::render` lays it out, or with `--json` as `HistoryStats`.
pub(super) async fn stats(runs: &mut Runs, dir: Option<PathBuf>, json: bool) -> anyhow::Result<()> {
    let dir = crate::resolve_dir(dir)?;
    match runs.request(RunRequest::Stats { dir }).await? {
        RunReply::Stats { stats, .. } if json => {
            println!("{}", serde_json::to_string_pretty(&stats)?);
            Ok(())
        }
        RunReply::Stats { stats, .. } => {
            print!("{}", daemon::run::stats::render(&stats));
            Ok(())
        }
        other => print_outcome(other),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use clap::error::ErrorKind;
    use proto::RunRequest;

    use super::super::{RunCommand, request_timeout};
    use super::GOAL_REQUEST_TIMEOUT;
    use std::time::Duration;

    #[derive(Parser, Debug)]
    struct Cli {
        #[command(subcommand)]
        command: RunCommand,
    }

    fn parse(args: &[&str]) -> Result<RunCommand, ErrorKind> {
        let mut all = vec!["run"];
        all.extend_from_slice(args);
        Cli::try_parse_from(all)
            .map(|cli| cli.command)
            .map_err(|e| e.kind())
    }

    #[test]
    fn goal_and_plan_are_mutually_exclusive_and_one_is_required() {
        assert_eq!(
            parse(&["start", "--plan", "p.toml", "--goal", "fix it"]).unwrap_err(),
            ErrorKind::ArgumentConflict
        );
        assert_eq!(
            parse(&["start", "--yes"]).unwrap_err(),
            ErrorKind::MissingRequiredArgument
        );
        let goal = format!(
            "{:?}",
            parse(&["start", "--goal", "fix it", "--yes"]).unwrap()
        );
        assert!(goal.contains("goal: Some(\"fix it\")"), "{goal}");
        let plan = format!("{:?}", parse(&["start", "--plan", "p.toml"]).unwrap());
        assert!(plan.contains("plan: Some(\"p.toml\")"), "{plan}");
        let promote = format!("{:?}", parse(&["promote", "3f9a"]).unwrap());
        assert!(promote.contains("Promote"), "{promote}");
    }

    #[test]
    fn goal_uses_the_goal_request_timeout() {
        let goal = RunRequest::StartGoal {
            goal: "g".into(),
            dir: "/r".into(),
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: None,
        };
        assert_eq!(request_timeout(&goal), GOAL_REQUEST_TIMEOUT);
        assert_eq!(
            GOAL_REQUEST_TIMEOUT,
            super::super::RUN_REQUEST_TIMEOUT + Duration::from_secs(600 + 30)
        );
        let promote = RunRequest::Promote {
            run_id: "r".into(),
            orchestrator: None,
        };
        assert_eq!(request_timeout(&promote), super::super::RUN_REQUEST_TIMEOUT);
    }

    /// M8b.17's acceptance: `anthrex run --help` lists `stats`, which takes `--json`.
    #[test]
    fn run_help_lists_stats_and_stats_takes_json() {
        use clap::CommandFactory;
        let help = Cli::command().render_help().to_string();
        assert!(
            help.lines().any(|l| l.trim_start().starts_with("stats ")),
            "{help}"
        );
        let json = format!("{:?}", parse(&["stats", "--json"]));
        assert!(
            json.contains("Stats") && json.contains("json: true"),
            "{json}"
        );
        let text = format!("{:?}", parse(&["stats"]));
        assert!(text.contains("json: false"), "{text}");
        let stats = RunRequest::Stats { dir: "/r".into() };
        assert_eq!(request_timeout(&stats), super::super::RUN_REQUEST_TIMEOUT);
    }
}
