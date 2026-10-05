//! Milestone 8b's `anthrex run` commands: `run start --goal` (decision 22), `run
//! promote` (decision 25) and `run stats` (decision 35).

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{RunReply, RunRequest};

use super::{Runs, print_outcome};

/// `run start --goal`'s reply bound (decision 22): [`super::RUN_START_TIMEOUT`] for
/// the start path (M8a's git preflight, and task M9.2.12's host preflight, which a `pr`
/// goal runs before triage), the largest configurable `deciders.timeout_secs` (600 s) for
/// the triage call, and 30 s for the request's own preflight and `git ls-files`; and
/// (milestone 9.5 ruling I6) the installed probe the triage call routes over first.
pub const GOAL_REQUEST_TIMEOUT: Duration = super::RUN_START_TIMEOUT
    .saturating_add(Duration::from_secs(600 + 30))
    .saturating_add(daemon::run::driver::INSTALLED_PROBE_TIMEOUT);

/// `run start --goal … --continue`'s reply bound (milestone 9.3's final fix wave, B-I1):
/// the daemon's own deadline on a continued start's steps before `Start`
/// (`CONTINUE_START_BOUND`, `run start`'s terms; a continue is not triaged), then 30 s
/// for its engine step and the reply. Past the deadline the daemon refuses, saying
/// nothing was started, so this wait always hears a true answer.
pub const CONTINUE_REQUEST_TIMEOUT: Duration =
    daemon::run::chain::CONTINUE_START_BOUND.saturating_add(Duration::from_secs(30));

/// `run start --goal`: on the fast path the run id on stdout and triage's message on
/// stderr, and so on the plan and large paths with the planned message (milestone 9
/// decision 26); any refusal is the command's error (exit 1). `orchestrator` is
/// `--orchestrator`'s choice (decision 6), `delivery` `--delivery`'s (M9.2 decision 3).
/// With `--continue <run>` (milestone 9.3 decision 22) the run is resolved first and the
/// goal continues its chain, untriaged: `Started`'s run id on stdout.
pub(super) async fn start_goal(
    socket: &Path,
    dir: Option<PathBuf>,
    goal: String,
    flags: (bool, bool, bool),
    (orchestrator, delivery, continue_from, design): (
        Option<proto::OrchestratorChoice>,
        Option<proto::DeliveryMode>,
        Option<String>,
        Option<proto::DesignMode>,
    ),
) -> anyhow::Result<()> {
    continue_checked(&goal, continue_from.is_some())?;
    let dir = crate::resolve_dir(dir)?;
    tui::spawn::ensure_daemon(&std::env::current_exe()?, socket).await?;
    let mut runs = Runs::connect(socket).await?;
    let continue_from = match continue_from {
        Some(run) => Some(runs.resolve(&run).await?),
        None => None,
    };
    let options = (orchestrator, delivery, design);
    let request = goal_request(goal, dir, flags, options, continue_from);
    match runs.request(request).await? {
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
        RunReply::Started { run_id, .. } => {
            println!("{run_id}");
            Ok(())
        }
        other => print_outcome(other),
    }
}

/// The final fix wave (B-M6): a continued goal over `GOAL_MAX_CHARS` is refused before
/// connecting, with the daemon's own text, as `run iterate`'s request is.
pub(super) fn continue_checked(goal: &str, continuing: bool) -> anyhow::Result<()> {
    if continuing && goal.chars().count() > proto::GOAL_MAX_CHARS {
        anyhow::bail!(daemon::run::orch::contract_rounds::GOAL_TOO_LONG);
    }
    Ok(())
}

/// The `StartGoal` request `run start --goal` sends; `continue_from` is the resolved run
/// id of `--continue`, which conflicts with `--orchestrator` (decision 31), and `design`
/// is `--design`'s (milestone 9.6 decision 3).
pub(super) fn goal_request(
    goal: String,
    dir: PathBuf,
    (yes, trust_project, unconfined_checks): (bool, bool, bool),
    (orchestrator, delivery, design): (
        Option<proto::OrchestratorChoice>,
        Option<proto::DeliveryMode>,
        Option<proto::DesignMode>,
    ),
    continue_from: Option<String>,
) -> RunRequest {
    RunRequest::StartGoal {
        goal,
        dir,
        yes,
        trust_project,
        unconfined_checks,
        orchestrator,
        delivery,
        continue_from,
        design,
    }
}

/// `run stats --json`'s text, through `printable` like everything else the CLI prints
/// of the daemon's (deferred from task 14: a history's path and problems are text).
pub(super) fn stats_json(stats: &proto::HistoryStats) -> anyhow::Result<String> {
    Ok(super::status::printable(&serde_json::to_string_pretty(
        stats,
    )?))
}

/// `run stats`' tuning flags (milestone 9.5 decision 11).
#[derive(clap::Args, Debug, Default)]
pub struct TuningFlags {
    /// Apply these tuning proposals, after asking
    #[arg(long, value_name = "ID", num_args = 1..)]
    apply: Vec<String>,
    /// Stop proposing these tuning proposals at their proposed values
    #[arg(long, value_name = "ID", num_args = 1..)]
    dismiss: Vec<String>,
    /// Apply without asking
    #[arg(long)]
    yes: bool,
}

/// `--apply` or `--dismiss` when the stats came without their tuning block (task
/// M9.5.11's fix round, review I1): the daemon logs why.
const TUNING_UNAVAILABLE: &str =
    "tuning is not available right now (see the daemon's log); nothing applied or dismissed";

/// What the daemon took, one line each, naming the repository the tuning belongs to
/// (`dir` only from a daemon that does not say); a dismissed proposal's value is the
/// one the daemon stored (whole-branch review C, m-2).
fn taken(report: Option<&proto::TuningReport>, dir: &Path) -> Vec<String> {
    let Some(report) = report else {
        return Vec::new();
    };
    let project = report.project.as_deref().unwrap_or(dir);
    let applied = (report.applied.iter())
        .map(|id| format!("applied {id}: new runs in {} use it", project.display()));
    let dismissed = (report.dismissed.iter()).map(|d| {
        format!(
            "dismissed {}: it is not proposed again while it would propose {}",
            d.id, d.value
        )
    });
    applied.chain(dismissed).collect()
}

/// `--apply`'s question when stdin is not a terminal.
const NOT_A_TERMINAL_APPLY: &str = "stdin is not a terminal; pass --yes";

/// `run stats`: the daemon's summary of the history of `dir`'s repository, as
/// `stats::render` lays it out and then the tuning block (`refit_render::render`), or
/// with `--json` as `HistoryStats`. With `--apply` or `--dismiss` (milestone 9.5
/// decision 11) the current proposals are fetched first; each applied one is asked
/// `apply <id>: <text>? [y/N]` unless `--yes`, and the ids answered yes, each with the
/// value it was confirmed at, with every dismissed one, go to the daemon, which takes
/// only proposals still current at that value and refuses the whole request over an
/// unknown or changed one. What was applied or dismissed is printed before the stats.
pub(super) async fn stats(
    runs: &mut Runs,
    dir: Option<PathBuf>,
    json: bool,
    flags: TuningFlags,
) -> anyhow::Result<()> {
    let dir = crate::resolve_dir(dir)?;
    let request = |apply: Vec<proto::ProposalValue>, dismiss: Vec<String>| RunRequest::Stats {
        dir: dir.clone(),
        apply,
        dismiss,
        read_only: false,
    };
    let TuningFlags {
        apply,
        dismiss,
        yes,
    } = flags;
    let reply = if apply.is_empty() && dismiss.is_empty() {
        runs.request(request(Vec::new(), dismiss)).await?
    } else {
        // Review I1: with no tuning block, nothing can be confirmed, so nothing is sent.
        let current = match runs.request(request(Vec::new(), Vec::new())).await? {
            RunReply::Stats { stats, .. } => match stats.tuning {
                Some(tuning) => tuning.proposals,
                None => anyhow::bail!(TUNING_UNAVAILABLE),
            },
            other => return print_outcome(other),
        };
        let mut chosen = Vec::new();
        for id in apply {
            // An id not current is sent with no value: the daemon refuses it with its
            // text. Whole-branch review C, m-2: a current one goes with the value the
            // user confirmed, which the daemon refuses once the proposal has changed.
            let fetched = current.iter().find(|p| p.id == id);
            if let Some(p) = fetched.filter(|_| !yes) {
                let question = format!("apply {id}: {}? [y/N] ", p.text);
                if !super::finish::ask_yes(&question, NOT_A_TERMINAL_APPLY).await? {
                    continue;
                }
            }
            let value = fetched.map(|p| p.proposed.clone()).unwrap_or_default();
            chosen.push(proto::ProposalValue { id, value });
        }
        if chosen.is_empty() && dismiss.is_empty() {
            anyhow::bail!("nothing applied");
        }
        runs.request(request(chosen, dismiss)).await?
    };
    match reply {
        RunReply::Stats { stats, .. } => {
            for line in taken(stats.tuning.as_deref(), &dir) {
                if json {
                    eprintln!("{}", super::status::printable(&line));
                } else {
                    println!("{}", super::status::printable(&line));
                }
            }
            if json {
                println!("{}", stats_json(&stats)?);
            } else {
                let mut text = daemon::run::stats::render(&stats);
                if let Some(tuning) = &stats.tuning {
                    text.push_str(&daemon::run::refit_render::render(tuning));
                }
                print!("{}", super::status::printable(&text));
            }
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

    /// Whole-branch review C, m-2: the `dismissed` line prints the value the daemon
    /// stored, not the one fetched before the request.
    #[test]
    fn the_dismissed_line_prints_the_stored_value() {
        let report = proto::TuningReport {
            path: "/d/tuning.toml".into(),
            min_samples: 30,
            refit_budgets: true,
            classes: Vec::new(),
            proposals: Vec::new(),
            moved_bad_file: None,
            applied: Vec::new(),
            dismissed: vec![proto::ProposalValue {
                id: "thresholds.s".into(),
                value: "38".into(),
            }],
            orchestrator_list: None,
            parse_error: None,
            project: None,
        };
        assert_eq!(
            super::taken(Some(&report), std::path::Path::new("/r")),
            ["dismissed thresholds.s: it is not proposed again while it would propose 38"]
        );
    }

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
            delivery: None,
            continue_from: None,
            design: None,
        };
        assert_eq!(request_timeout(&goal), GOAL_REQUEST_TIMEOUT);
        assert_eq!(
            GOAL_REQUEST_TIMEOUT,
            super::super::RUN_START_TIMEOUT + Duration::from_secs(600 + 30 + 5)
        );
        assert_eq!(GOAL_REQUEST_TIMEOUT, Duration::from_secs(1220));
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
        let stats = RunRequest::Stats {
            dir: "/r".into(),
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: false,
        };
        assert_eq!(request_timeout(&stats), super::super::RUN_REQUEST_TIMEOUT);
    }

    /// Milestone 9.5 decision 11: `run stats` takes `--apply <id>…`, `--dismiss <id>…`
    /// and `--yes`, and its help lists them.
    #[test]
    fn run_help_lists_the_stats_flags() {
        use clap::CommandFactory;
        let mut cli = Cli::command();
        let help = cli
            .find_subcommand_mut("stats")
            .expect("stats")
            .render_long_help()
            .to_string();
        for flag in ["--apply <ID>", "--dismiss <ID>", "--yes"] {
            assert!(help.contains(flag), "{flag} in:\n{help}");
        }
        let both = format!(
            "{:?}",
            parse(&[
                "stats",
                "--apply",
                "a",
                "b",
                "--apply",
                "c",
                "--dismiss",
                "d",
                "--yes"
            ])
        );
        assert!(
            both.contains(r#"apply: ["a", "b", "c"]"#)
                && both.contains(r#"dismiss: ["d"]"#)
                && both.contains("yes: true"),
            "{both}"
        );
        let plain = format!("{:?}", parse(&["stats"]));
        assert!(
            plain.contains("apply: []") && plain.contains("yes: false"),
            "{plain}"
        );
    }
}
