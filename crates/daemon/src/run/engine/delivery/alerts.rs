//! Ruling R-13 (task M9.2.15's fix round): the run's delivery attention lines, each
//! with the alert kind it is when the user must act on it. One list feeds both the
//! attention lines ([`attention`]) and the snapshot's typed alerts ([`alerts`]), so the
//! two cannot drift. A line's kind comes from the key the engine filed it under, never
//! from its text. Pure.

use proto::{DeliveryAlert, DeliveryAlertKind};

use super::pr;
use crate::run::model::Run;

/// One delivery attention line, and its alert kind and stage when it is an alert.
pub(crate) struct Line {
    pub kind: Option<DeliveryAlertKind>,
    pub stage: Option<u16>,
    pub text: String,
}

/// Every delivery attention line of a `pr` run, in the order `run status` shows them:
/// the keyed lines (a lost login, an op failing again and again, a red handed to the
/// user, a closed PR, a review round over the cap, a dropped reply, a refused fix, a
/// full view page), each held stage (ruling R-11), then the review lines (a fix hold
/// awaiting the user, threads not addressed: a STATUS line, never an alert).
pub(crate) fn lines(run: &Run) -> Vec<Line> {
    if !pr(run) {
        return Vec::new();
    }
    let mut out: Vec<Line> = (run.delivery.alerts.iter())
        .map(|(key, text)| {
            let (kind, stage) = classify(key);
            Line {
                kind,
                stage,
                text: text.clone(),
            }
        })
        .collect();
    for (n, s) in (1u16..).zip(&run.delivery.stages) {
        if let Some(reason) = &s.held {
            out.push(Line {
                kind: Some(DeliveryAlertKind::HostOpHeld),
                stage: Some(n),
                text: super::watch::hold_line(run, n, reason),
            });
        }
    }
    out.extend(super::review::attention(run).into_iter().map(|text| Line {
        kind: None,
        stage: None,
        text,
    }));
    out
}

/// The run's delivery attention lines (decision 11, ruling R-11, decisions 26 and 31).
pub(crate) fn attention(run: &Run) -> Vec<String> {
    lines(run).into_iter().map(|line| line.text).collect()
}

/// The snapshot's `DeliveryInfo.alerts`: the lines that are alerts, their text one line.
pub(crate) fn alerts(run: &Run) -> Vec<DeliveryAlert> {
    (lines(run).into_iter())
        .filter_map(|line| {
            Some(DeliveryAlert {
                kind: line.kind?,
                stage: line.stage,
                text: proto::safe_text::one_line(&line.text),
                // Milestone 9.9 decision 18: set by M9.9.5.
                user_only: false,
            })
        })
        .collect()
}

/// A `RunDelivery.alerts` key's kind and stage: `auth` (decision 11), `<n>/ci/<key>`
/// (`fix.rs::to_user`), `<n>/closed` (`land.rs::closed`), `<n>/cap/<key>`
/// (`review.rs::over_cap`), and a failure key `<n|run>/<op>` (`failure_key`, past
/// `FAILURES_BEFORE_ATTENTION`) whose `<op>` is one of [`super::OP_NAMES`]. An
/// unlanded stage (`<n>/unlanded`), a red base sync (`<n>/sync`), a dropped reply
/// (`<n>/reply/…`), a refused fix (`<n>/review/…`), a full page (`<n>/page/…`) and a
/// fix that missed the merge (`<n>/missed`, the final fix wave's I-1) are attention
/// lines only, and so is any key not listed here: R-13's five kinds are the
/// whole list, never a guess (ruling, task 15 fix round 2).
fn classify(key: &str) -> (Option<DeliveryAlertKind>, Option<u16>) {
    use DeliveryAlertKind::*;
    if key == super::watch::AUTH {
        return (Some(GhLoggedOut), None);
    }
    let Some((stage, rest)) = key.split_once('/') else {
        return (None, None);
    };
    let n = stage.parse::<u16>().ok();
    let kind = match rest.split_once('/') {
        None if rest == "closed" && n.is_some() => Some(PrClosedUnmerged),
        None if super::OP_NAMES.contains(&rest) && (stage == "run" || n.is_some()) => {
            Some(HostOpHeld)
        }
        None => None,
        Some(("ci", _)) => Some(CiHandedToUser),
        Some(("cap", _)) => Some(ReviewRoundsOverCap),
        Some(_) => None,
    };
    (kind, kind.and(n))
}

/// Fix round m3 of task M9.2.8, widened by the final fix wave's B m-1: an op that keeps
/// failing is no longer due takes its failures in a row, and its attention line, with
/// it. A failure key `<n|run>/<op>` is due while the delivery pass would still issue
/// that op: `failed_logs`, which gives a log up after `FAILURES_BEFORE_ATTENTION`
/// tries, is the one that always happens.
pub(super) fn settle(run: &mut Run) {
    let keys: Vec<String> = (run.delivery.failures.keys())
        .chain(run.delivery.alerts.keys())
        .filter(|k| !due(run, k))
        .cloned()
        .collect();
    for key in keys {
        run.delivery.failures.remove(&key);
        run.delivery.alerts.remove(&key);
    }
}

/// Whether failure key `key`'s op is still due; any key that is not a failure key is.
fn due(run: &Run, key: &str) -> bool {
    use proto::PrState::{Merged, Open};
    let Some((stage, op)) = key.split_once('/') else {
        return true;
    };
    if !super::OP_NAMES.contains(&op) {
        return true;
    }
    let d = &run.delivery;
    let Ok(n) = stage.parse::<u16>() else {
        return match (stage, op) {
            ("run", "permission") => (d.stages.iter())
                .flat_map(|s| &s.threads)
                .any(|t| !t.candidates.is_empty()),
            ("run", "fetch") => super::sync::fetch_due(run),
            _ => true,
        };
    };
    let Some(s) = d.stage(n) else {
        return false;
    };
    let pr = s.pr.as_ref();
    let open = pr.is_some_and(|p| p.state == Open);
    let opening = pr.is_none() && !s.skipped;
    let record = super::ci::active(run, n).and_then(|i| s.ci.get(i));
    let phase = record.map(|r| r.phase);
    match op {
        "push" => opening || open && run.stage_head(n) != pr.map(|p| p.pushed_head.as_str()),
        "fetch" => s.remote_head.is_some(),
        "open_pr" => opening,
        "failed_logs" => record.is_some_and(|r| {
            r.phase == crate::run::delivery::CiPhase::Logs
                && r.ci_runs.iter().any(|c| !r.fetched.contains(c))
        }),
        "rerun_failed" => phase == Some(crate::run::delivery::CiPhase::Rerunning),
        "reply" => !s.replies.is_empty(),
        "retarget" => open,
        "delete_branch" => {
            d.limits.delete_merged_branches
                && pr.is_some_and(|p| p.state == Merged && !p.branch_deleted)
        }
        _ => pr.is_some(),
    }
}

#[cfg(test)]
#[path = "alerts_tests.rs"]
mod tests;
