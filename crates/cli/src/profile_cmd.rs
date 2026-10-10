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
    /// Use the ready proposal: store it, then start any queued goals
    #[command(alias = "confirm")]
    Use {
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
        /// Store it once verification has run, even if a check fails (implies --yes)
        #[arg(long)]
        anyway: bool,
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
        DaemonMsg::Run(RunReply::Profile { reply, .. }) => Ok(*reply),
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
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                print!("{}", status_text(&status, now));
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
        ProfileCommand::Use { yes } => {
            let shown = request(
                &mut client,
                ProfileRequest::Show {
                    dir: dir.clone(),
                    proposed: true,
                },
            )
            .await?;
            // Review m3: the daemon stores this proposal only if it still reads so.
            let text = match &shown {
                ProfileReply::Shown { toml, .. } => Some(toml.clone()),
                _ => None,
            };
            show(shown, false)?;
            if !yes {
                let project = status(&mut client, &dir).await?.project;
                let question = format!("store this profile for {}? [y/N] ", project.display());
                if !ask(&question).await? {
                    anyhow::bail!("not stored");
                }
            }
            done(request(&mut client, ProfileRequest::Confirm { dir, shown: text }).await?)
        }
        ProfileCommand::Reject => done(request(&mut client, ProfileRequest::Reject { dir }).await?),
        ProfileCommand::Edit {
            key,
            value,
            unset: _,
            yes,
            anyway,
            unconfined_checks,
        } => done(
            request(
                &mut client,
                edit_request(dir, key, value, yes, anyway, unconfined_checks),
            )
            .await?,
        ),
    }
}

/// `profile edit`'s request; `--anyway` implies `--yes` (decision 37).
fn edit_request(
    dir: PathBuf,
    key: String,
    value: Option<String>,
    yes: bool,
    anyway: bool,
    unconfined_checks: bool,
) -> ProfileRequest {
    ProfileRequest::Edit {
        dir,
        key,
        value,
        yes: yes || anyway,
        unconfined_checks,
        anyway,
        on_proposal: false,
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

/// A daemon-supplied string as one terminal line (goals, reasons, labels, paths).
fn one(text: &str) -> String {
    crate::run_cmd::printable(&proto::safe_text::one_line(text))
}

/// `profile status`'s text (decision 38): the project, the status line, today's lines,
/// then the waiting and dropped goals. `now` is Unix seconds, read once by the caller.
/// Every daemon-supplied string goes through `printable`.
pub fn status_text(status: &ProfileStatus, now: u64) -> String {
    crate::run_cmd::printable(&status_body(status, now))
}

fn status_body(status: &ProfileStatus, now: u64) -> String {
    let mut out = format!("profile: {}\n", one(&status.project.display().to_string()));
    out.push_str(&format!(
        "  {}\n",
        one(&tui::profile_words::status_line(status, now))
    ));
    match (status.confirmed_at, &status.unparseable) {
        (_, Some(problem)) => {
            out.push_str(&format!("  stored: does not parse: {}\n", one(problem)));
        }
        (Some(at), None) => out.push_str(&format!(
            "  stored: yes, confirmed {} ({})\n",
            minute(at),
            one(&status.repo_dir.join("profile.toml").display().to_string())
        )),
        (None, None) => out.push_str("  stored: no\n"),
    }
    if !status.stale.is_empty() {
        out.push_str(&format!(
            "  stale: {} changed since it was confirmed\n",
            status
                .stale
                .iter()
                .map(|f| one(f))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    match &status.proposal {
        None => out.push_str("  detection: none\n"),
        Some(record) => match &record.state {
            ProposalState::Failed { reason } => {
                out.push_str(&format!("  detection: failed: {}\n", one(reason)));
            }
            state => {
                let mut line = format!(
                    "  detection: {} since {}",
                    daemon::profile::service::state_label(state),
                    minute(record.updated_at).get(11..).unwrap_or_default()
                );
                if let (Some(id), Some(window)) = (&record.scout_id, record.window_id) {
                    line.push_str(&format!(" (scout {}, window {window})", one(id)));
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
    for queued in &status.queued {
        out.push_str(&format!("  waiting: {}\n", one(&queued.goal)));
    }
    for dropped in &status.dropped_goals {
        out.push_str(&format!(
            "  dropped goal \"{}\": {}\n",
            one(&dropped.goal),
            one(&dropped.reason)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{DroppedGoal, ProfileSource, QueuedGoalInfo, SetupState};

    fn status() -> ProfileStatus {
        ProfileStatus {
            project: PathBuf::from("/work/app"),
            repo_dir: PathBuf::from("/data/repos/app"),
            source: ProfileSource::Stored,
            confirmed_at: Some(900),
            stale: Vec::new(),
            unparseable: None,
            proposal: None,
            scout: None,
            verify_confined: true,
            queued: Vec::new(),
            checking: None,
            verified_at: Some(940),
            unreadable_text: None,
            dropped_goals: Vec::new(),
        }
    }

    fn queued(goal: &str) -> QueuedGoalInfo {
        QueuedGoalInfo {
            id: "q1".into(),
            project: PathBuf::from("/work/app"),
            goal: goal.into(),
            queued_at: 950,
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            setup: SetupState::NeedsReview,
        }
    }

    #[test]
    fn status_text_has_the_status_line_second() {
        let mut s = status();
        s.queued = vec![queued("add a")];
        s.dropped_goals = vec![DroppedGoal {
            goal: "add b".into(),
            reason: "the profile was rejected".into(),
            at: 960,
        }];
        let text = status_text(&s, 1000);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "profile: /work/app");
        assert_eq!(lines[1], "  Ready · verified 1m ago");
        assert!(lines[2].starts_with("  stored: yes, confirmed "), "{text}");
        assert!(text.contains("  detection: none\n"), "{text}");
        assert!(text.contains("  verification: confined, as runs are\n"));
        let tail = &lines[lines.len() - 2..];
        assert_eq!(tail[0], "  waiting: add a");
        assert_eq!(
            tail[1],
            "  dropped goal \"add b\": the profile was rejected"
        );
    }

    #[test]
    fn status_text_draws_daemon_text_safely() {
        let mut s = status();
        s.project = PathBuf::from("/work/\u{1b}[31mapp");
        s.stale = vec!["a\u{1b}[2Jb".into(), "x\ny".into()];
        s.queued = vec![queued("go\n  stored: forged\u{1b}]0;t\u{7}\u{202e}")];
        s.dropped_goals = vec![DroppedGoal {
            goal: "g\r\n  waiting: forged".into(),
            reason: "r\u{1b}[0m\nline".into(),
            at: 1,
        }];
        s.unparseable = Some("bad\u{1b}[1m toml".into());
        let text = status_text(&s, 1000);
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{text:?}"
        );
        assert!(!text.contains('\u{202e}'), "{text:?}");
        // A goal or reason cannot start a line of its own.
        for line in text.lines() {
            assert!(
                !line.starts_with("  stored: forged") && !line.starts_with("  waiting: forged"),
                "{text:?}"
            );
        }
        assert_eq!(
            text.lines().filter(|l| l.starts_with("  waiting:")).count(),
            1
        );
    }

    #[test]
    fn edit_anyway_sends_the_request_with_yes() {
        let request = edit_request(
            PathBuf::from("/work/app"),
            "check".into(),
            Some("false".into()),
            false,
            true,
            false,
        );
        assert_eq!(
            request,
            ProfileRequest::Edit {
                dir: PathBuf::from("/work/app"),
                key: "check".into(),
                value: Some("false".into()),
                yes: true,
                unconfined_checks: false,
                anyway: true,
                on_proposal: false,
            }
        );
    }
}
