//! `run accept`'s confirmations (decision 20) and the typed run id of `run reject` and
//! `run discard`, with the prompts they read from stdin.

use super::{Runs, print_outcome, status};
use proto::{FinishAction, RunInfo, RunReply, RunState};
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

/// The text a wrong typed or `--confirm` id gets.
const CONFIRM_MISMATCH: &str = "confirmation does not match the run id";

/// Printed once, before the first prompt, when stdin is not a terminal (ruling
/// T23-minors, M5): a pipe that stays open would otherwise wait with no explanation.
const NOT_A_TERMINAL: &str = "stdin is not a terminal; pass --yes or --confirm";

/// How many times `run accept` lists a base that keeps moving before it gives up
/// (ruling T23-minors, M7). Each round is one more commit on the base between the
/// listing and the answer.
const ACCEPT_ROUNDS: usize = 5;

/// The shortest `--base` prefix of the listed head (ruling T23-I2): the seven
/// characters the listing shows.
const BASE_PREFIX_MIN: usize = 7;

/// `run accept` (decision 20).
///
/// - A run that is not `complete` is refused before any question, in the daemon's
///   words (ruling T23-minors, M2).
/// - A moved base is listed before any question (M3). The merge question follows,
///   unless `--yes`; then the moved-base question, which only `--base` naming the
///   listed head (or a prefix of it) answers. A yes resends `confirm = "<id>@<to>"`.
pub(super) async fn accept(
    runs: &mut Runs,
    info: &RunInfo,
    yes: bool,
    base: Option<&str>,
) -> anyhow::Result<()> {
    let run_id = &info.run_id;
    if info.state != RunState::Complete {
        anyhow::bail!(
            "run {run_id} is {}; accept applies only to a complete run",
            info.state.label()
        );
    }
    let mut asked = yes;
    let mut confirm = None;
    for _ in 0..ACCEPT_ROUNDS {
        let reply = runs
            .finish(run_id, FinishAction::Accept, confirm.take())
            .await?;
        let RunReply::ConfirmNeeded {
            prompt, base_moved, ..
        } = reply
        else {
            return print_outcome(reply);
        };
        if let Some(moved) = &base_moved {
            eprint!("{}", status::base_moved_listing(&info.base_branch, moved));
        }
        if let (Some(moved), Some(sha)) = (&base_moved, base)
            && !base_matches(sha, &moved.to)
        {
            anyhow::bail!(
                "--base {sha} is not the listed head {}; not merged",
                moved.to
            );
        }
        if !asked {
            if !ask_yes(&format!("{prompt} [y/N] ")).await? {
                anyhow::bail!("not merged");
            }
            asked = true;
        }
        confirm = Some(match base_moved {
            None => run_id.clone(),
            Some(moved) => {
                if base.is_none() {
                    let question = status::base_moved_question(&info.base_branch, &moved);
                    if !ask_yes(&question).await? {
                        anyhow::bail!("not merged");
                    }
                }
                format!("{run_id}@{}", moved.to)
            }
        });
    }
    anyhow::bail!(
        "{} kept moving; not merged; run accept again",
        info.base_branch
    )
}

/// Whether `--base` names the listed head `to`: the full sha, or a hex prefix of it of
/// at least [`BASE_PREFIX_MIN`] characters. Only one head is listed, so any prefix of it
/// is unambiguous; the resend carries the full sha, which the daemon checks.
pub(super) fn base_matches(given: &str, to: &str) -> bool {
    given.len() >= BASE_PREFIX_MIN
        && given.chars().all(|c| c.is_ascii_hexdigit())
        && to.starts_with(&given.to_ascii_lowercase())
}

/// `--confirm`, or the run id typed after `prompt`; anything else is refused.
pub(super) async fn confirm_id(
    run_id: &str,
    given: Option<String>,
    prompt: &str,
) -> anyhow::Result<()> {
    let typed = match given {
        Some(given) => given,
        None => read_answer(&format!("{prompt}\ntype the run id to confirm: ")).await?,
    };
    if typed.trim() != run_id {
        anyhow::bail!(CONFIRM_MISMATCH);
    }
    Ok(())
}

async fn ask_yes(question: &str) -> anyhow::Result<bool> {
    let answer = read_answer(question).await?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Prints `prompt` on stderr and reads one line of stdin (empty at end of input). When
/// the answer did not end the prompt's line on the screen (end of input, or stdin that
/// is not a terminal and so echoes nothing), a newline does, so what follows starts
/// on its own line.
async fn read_answer(prompt: &str) -> anyhow::Result<String> {
    static HINTED: AtomicBool = AtomicBool::new(false);
    let terminal = std::io::stdin().is_terminal();
    if !terminal && !HINTED.swap(true, Ordering::Relaxed) {
        eprintln!("{NOT_A_TERMINAL}");
    }
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let (read, line) = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map(|n| (n, line))
    })
    .await??;
    if read == 0 || !terminal {
        eprintln!();
    }
    Ok(line)
}
