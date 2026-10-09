//! Milestone 9.9 decision 6 (OFA §4.2): the orchestrator's `accept_red`. An accepted red
//! tier 3 passes its stage's head like a green one does ([`passed_at`]), until the head
//! moves; the stage's record keeps `red_at`, so the snapshot still reads red, accepted.
//! Pure (design decision 2).

use super::log;
use crate::run::contract::sha7;
use crate::run::model::{Run, StageFull};

/// Tier 3 passed `head`: a green job ran on it, or the orchestrator accepted its red.
pub(crate) fn passed_at(full: &StageFull, head: &str) -> bool {
    full.green_at.as_deref() == Some(head) || full.accepted_red.as_deref() == Some(head)
}

/// Stage `n`'s red tier 3 on its head is accepted. Returns the head's short sha, or
/// `None` when the stage does not exist. The caller checked `rules::accept_red`.
pub(crate) fn accept_red(run: &mut Run, n: u16, now: u64) -> Option<String> {
    let head = run.stage(n)?.head.clone();
    run.stages.iter_mut().find(|s| s.n == n)?.full.accepted_red = Some(head.clone());
    let short = sha7(&head);
    log(
        run,
        now,
        format!("stage {n}: tier 3 red on {short} accepted by the orchestrator"),
    );
    Some(short.to_string())
}
