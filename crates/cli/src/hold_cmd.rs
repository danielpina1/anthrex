//! `anthrex hold on|off|status` (milestone 9.16 decision 34): hold or release dispatch
//! daemon-wide, or show the hold and the quota brake. `on` and `off` print the daemon's
//! message, and a refusal exits 1; `status` prints [`brake_lines`], which `anthrex run
//! status` shares. Every request waits [`HOLD_REQUEST_TIMEOUT`] (ruling C34).

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use common::numbers::{Zone, clock};
use proto::brakes::{BrakesInfo, QuotaInfo};
use proto::safe_text::one_line;
use proto::{ClientMsg, DaemonMsg, RunReply, RunRequest};

use crate::client::CliClient;
use crate::run_cmd::printable;

/// Ruling C34: the client's default bound (5 s) equals the daemon's `BRAKES_IO_WAIT`, so
/// `hold on` could time out while the daemon still saves `brakes.json`.
pub const HOLD_REQUEST_TIMEOUT: Duration = crate::run_cmd::RUN_REQUEST_TIMEOUT;

#[derive(Args, Debug)]
pub struct HoldArgs {
    #[command(subcommand)]
    command: HoldCommand,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
enum HoldCommand {
    /// Start no new agent session anywhere until `anthrex hold off`; running ones continue
    On,
    /// Release the hold: new sessions may start again
    Off,
    /// Show the hold and the quota brake
    Status,
}

/// Runs one `anthrex hold` command; any error is printed as it is and exits 1.
pub async fn main(args: HoldArgs, socket: PathBuf) -> anyhow::Result<()> {
    if let Err(error) = dispatch(args.command, &socket).await {
        eprintln!("{}", printable(&error.to_string()));
        std::process::exit(1);
    }
    Ok(())
}

async fn dispatch(command: HoldCommand, socket: &Path) -> anyhow::Result<()> {
    tui::spawn::ensure_daemon(&std::env::current_exe()?, socket).await?;
    let mut client = CliClient::connect(socket).await?;
    // Task 10's review I1: what `hold off` releases, read before it.
    let mut release = None;
    let request = match command {
        HoldCommand::On => RunRequest::Hold { on: true },
        HoldCommand::Off => {
            if let RunReply::Snapshot(before) = request_of(&mut client, RunRequest::List).await? {
                release = queued_line(before.brakes.hold.is_some(), before.queued_goals.len());
            }
            RunRequest::Hold { on: false }
        }
        HoldCommand::Status => RunRequest::List,
    };
    match request_of(&mut client, request).await? {
        RunReply::Held { message, hold, .. } => {
            println!("{}", printable(&message));
            if let (None, Some(line)) = (hold, release) {
                println!("{line}");
            }
        }
        RunReply::Snapshot(snapshot) => {
            // Milestone 9.19 decision 3: the viewer's zone, read once per command.
            let zone = Zone::Local(common::localtime::offset_secs(snapshot.now));
            for line in brake_lines(&snapshot.brakes, snapshot.now, zone) {
                println!("{}", printable(&line));
            }
        }
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
    Ok(())
}

/// Task 10's review I1: after `hold off`'s reply, `<n> queued goals will start` when it
/// released a hold (`was_held`) with `queued` goals waiting; `None` otherwise.
fn queued_line(was_held: bool, queued: usize) -> Option<String> {
    match (was_held, queued) {
        (false, _) | (_, 0) => None,
        (true, 1) => Some("1 queued goal will start".to_owned()),
        (true, n) => Some(format!("{n} queued goals will start")),
    }
}

/// One request; a refusal is the error, in the daemon's words.
async fn request_of(client: &mut CliClient, request: RunRequest) -> anyhow::Result<RunReply> {
    let msg = ClientMsg::Run(request);
    match client
        .request_with_timeout(msg, HOLD_REQUEST_TIMEOUT)
        .await?
    {
        DaemonMsg::Run(RunReply::Refused { message, .. }) => anyhow::bail!(message),
        DaemonMsg::Run(reply) => Ok(reply),
        DaemonMsg::Error { message, .. } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// Decision 34: `hold: off` or `hold: on since <clock> (<by>)`; then `quota brake:
/// off` when disabled, else one line per runtime. Every clock time is the viewer's, in
/// `zone`, against the daemon's `now` (milestone 9.19 decision 3).
pub fn brake_lines(brakes: &BrakesInfo, now: u64, zone: Zone) -> Vec<String> {
    let mut lines = vec![match &brakes.hold {
        Some(hold) => format!(
            "hold: on since {} ({})",
            clock(hold.since, now, zone),
            one_line(&hold.by)
        ),
        None => "hold: off".to_owned(),
    }];
    if !brakes.quota_brake {
        lines.push("quota brake: off".to_owned());
        return lines;
    }
    lines.extend(
        brakes
            .quota
            .iter()
            .map(|quota| quota_line(quota, now, zone)),
    );
    lines
}

/// `<runtime> quota: 5h <n>% · 7d <n>% (read <m> min ago) · not braked`, `… · braked
/// until <clock> (<window> at <n>% ≥ <t>%)`, or `<runtime> quota: unmeasured · not
/// braked`.
fn quota_line(quota: &QuotaInfo, now: u64, zone: Zone) -> String {
    let runtime = quota.runtime;
    let (Some(read_at), false) = (quota.read_at, quota.windows.is_empty()) else {
        return format!("{runtime} quota: unmeasured \u{b7} not braked");
    };
    let windows: Vec<String> = (quota.windows.iter())
        .map(|w| format!("{} {}%", w.window.label(), w.used.percent()))
        .collect();
    let state = match &quota.trip {
        Some(trip) => format!(
            "braked until {} ({} at {}% \u{2265} {}%)",
            clock(trip.until, now, zone),
            trip.window.label(),
            trip.used.percent(),
            trip.threshold_percent
        ),
        None => "not braked".to_owned(),
    };
    format!(
        "{runtime} quota: {} (read {} min ago) \u{b7} {state}",
        windows.join(" \u{b7} "),
        now.saturating_sub(read_at) / 60
    )
}

#[cfg(test)]
#[path = "hold_cmd_tests.rs"]
mod tests;
