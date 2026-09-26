//! `anthrex profile …` (milestone 8b decision 10, the brief's CLI section): the
//! repository profile's status, detection, the stored profile or the proposal, and its
//! confirmation, rejection and correction. Every request waits
//! [`RUN_REQUEST_TIMEOUT`]; every refusal prints the daemon's message and exits 1.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use proto::{
    ClientMsg, DaemonMsg, ProfileReply, ProfileRequest, ProfileStatus, ProposalState, RunReply,
    RunRequest,
};

use crate::client::CliClient;
use crate::run_cmd::RUN_REQUEST_TIMEOUT;

#[derive(Args, Debug)]
pub struct ProfileArgs {
    #[command(subcommand)]
    command: ProfileCommand,
}

#[derive(Subcommand, Debug)]
enum ProfileCommand {
    /// Show whether this repository has a stored profile, and any detection
    Status {
        /// Print the status as pretty JSON
        #[arg(long)]
        json: bool,
    },
    /// Detect the profile with a read-only onboarding scout, then verify its commands
    Detect {
        /// Accept the repository's tracked agent settings
        #[arg(long)]
        trust_project: bool,
        /// Where this platform cannot confine the verification, run it unconfined anyway
        #[arg(long)]
        unconfined_checks: bool,
    },
    /// Print the stored profile, or the proposal
    Show {
        #[arg(long)]
        proposed: bool,
        #[arg(long)]
        json: bool,
    },
    /// Store the ready proposal
    Confirm {
        /// Do not ask first
        #[arg(long)]
        yes: bool,
    },
    /// Stop a detection and delete the proposal
    Reject,
    /// Correct one value of the stored profile (a new proposal)
    Edit {
        /// A profile key, or env.<NAME>
        key: String,
        /// The value, read as TOML (a bare word is a string)
        #[arg(required_unless_present = "unset")]
        value: Option<String>,
        /// Remove the key instead
        #[arg(long, conflicts_with = "value")]
        unset: bool,
        /// Store it as soon as verification passes
        #[arg(long)]
        yes: bool,
        /// Where this platform cannot confine the verification, run it unconfined anyway
        #[arg(long)]
        unconfined_checks: bool,
    },
}

/// Runs one `anthrex profile` command; any error is printed as it is and exits 1.
pub async fn main(args: ProfileArgs, socket: PathBuf, dir: Option<PathBuf>) -> anyhow::Result<()> {
    if let Err(error) = dispatch(args.command, &socket, dir).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
    Ok(())
}

async fn request(client: &mut CliClient, request: ProfileRequest) -> anyhow::Result<ProfileReply> {
    let msg = ClientMsg::Run(RunRequest::Profile(request));
    match client
        .request_with_timeout(msg, RUN_REQUEST_TIMEOUT)
        .await?
    {
        DaemonMsg::Run(RunReply::Profile(reply)) => Ok(*reply),
        DaemonMsg::Run(RunReply::Refused { message, .. }) => anyhow::bail!(message),
        DaemonMsg::Error { message, .. } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// `Done` prints its message; `Refused` is the command's error.
fn done(reply: ProfileReply) -> anyhow::Result<()> {
    match reply {
        ProfileReply::Done { message } => {
            println!("{message}");
            Ok(())
        }
        ProfileReply::Refused { message } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

async fn dispatch(
    command: ProfileCommand,
    socket: &Path,
    dir: Option<PathBuf>,
) -> anyhow::Result<()> {
    let dir = crate::resolve_dir(dir)?;
    tui::spawn::ensure_daemon(&std::env::current_exe()?, socket).await?;
    let mut client = CliClient::connect(socket).await?;
    match command {
        ProfileCommand::Status { json } => {
            let status = status(&mut client, &dir).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print!("{}", status_text(&status));
            }
            Ok(())
        }
        ProfileCommand::Detect {
            trust_project,
            unconfined_checks,
        } => done(
            request(
                &mut client,
                ProfileRequest::Detect {
                    dir,
                    trust_project,
                    unconfined_checks,
                },
            )
            .await?,
        ),
        ProfileCommand::Show { proposed, json } => {
            let reply = request(&mut client, ProfileRequest::Show { dir, proposed }).await?;
            show(reply, json)
        }
        ProfileCommand::Confirm { yes } => {
            let shown = request(
                &mut client,
                ProfileRequest::Show {
                    dir: dir.clone(),
                    proposed: true,
                },
            )
            .await?;
            show(shown, false)?;
            if !yes {
                let project = status(&mut client, &dir).await?.project;
                let question = format!("store this profile for {}? [y/N] ", project.display());
                if !ask(&question).await? {
                    anyhow::bail!("not stored");
                }
            }
            done(request(&mut client, ProfileRequest::Confirm { dir }).await?)
        }
        ProfileCommand::Reject => done(request(&mut client, ProfileRequest::Reject { dir }).await?),
        ProfileCommand::Edit {
            key,
            value,
            unset: _,
            yes,
            unconfined_checks,
        } => done(
            request(
                &mut client,
                ProfileRequest::Edit {
                    dir,
                    key,
                    value,
                    yes,
                    unconfined_checks,
                },
            )
            .await?,
        ),
    }
}

async fn status(client: &mut CliClient, dir: &Path) -> anyhow::Result<ProfileStatus> {
    match request(
        client,
        ProfileRequest::Status {
            dir: dir.to_path_buf(),
        },
    )
    .await?
    {
        ProfileReply::Status(status) => Ok(status),
        other => done(other).and_then(|_| anyhow::bail!("no status")),
    }
}

fn show(reply: ProfileReply, json: bool) -> anyhow::Result<()> {
    match reply {
        ProfileReply::Shown { toml, .. } if !json => {
            print!("{toml}");
            Ok(())
        }
        shown @ ProfileReply::Shown { .. } => {
            println!("{}", serde_json::to_string_pretty(&shown)?);
            Ok(())
        }
        other => done(other),
    }
}

/// Asks `question` on stderr and reads one line of stdin; yes only for `y`/`yes`.
async fn ask(question: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        eprintln!("stdin is not a terminal; pass --yes");
    }
    eprint!("{question}");
    let _ = std::io::stderr().flush();
    let line = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map(|_| line)
    })
    .await??;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// `YYYY-MM-DD HH:MM` in UTC.
fn minute(unix: u64) -> String {
    let at = daemon::run::report::format_utc(unix);
    at.get(..16).unwrap_or(&at).to_string()
}

/// `profile status`'s text (the brief's CLI section).
pub fn status_text(status: &ProfileStatus) -> String {
    let mut out = format!("profile: {}\n", status.project.display());
    match (status.confirmed_at, &status.unparseable) {
        (_, Some(problem)) => out.push_str(&format!("  stored: does not parse: {problem}\n")),
        (Some(at), None) => out.push_str(&format!(
            "  stored: yes, confirmed {} ({})\n",
            minute(at),
            status.repo_dir.join("profile.toml").display()
        )),
        (None, None) => out.push_str("  stored: no\n"),
    }
    if !status.stale.is_empty() {
        out.push_str(&format!(
            "  stale: {} changed since it was confirmed\n",
            status.stale.join(", ")
        ));
    }
    match &status.proposal {
        None => out.push_str("  detection: none\n"),
        Some(record) => match &record.state {
            ProposalState::Failed { reason } => {
                out.push_str(&format!("  detection: failed: {reason}\n"));
            }
            state => {
                let mut line = format!(
                    "  detection: {} since {}",
                    daemon::profile::service::state_label(state),
                    minute(record.updated_at).get(11..).unwrap_or_default()
                );
                if let (Some(id), Some(window)) = (&record.scout_id, record.window_id) {
                    line.push_str(&format!(" (scout {id}, window {window})"));
                }
                out.push_str(&line);
                out.push('\n');
            }
        },
    }
    if status.verify_confined {
        out.push_str("  verification: confined, as runs are\n");
    } else {
        out.push_str(
            "  verification: unconfined (this platform cannot confine it, or worker_sandbox is off)\n",
        );
    }
    out
}
