//! Milestone 9.2 decisions 36 and 42: a `pr`-mode run in the inspector. A stage's
//! pull request (`pr`, `url`), its CI per check of the most recent head, its threads
//! and its fix tasks by origin; the run's `delivery` row, and a single-stage run's PR
//! lines (it has no stage node); and the header's `running (delivering)`. Pure: the
//! time is formatted with the offset the `App` was given (`local_hhmm`), and every
//! text the host or an agent wrote goes through `clean`.

use super::run_format::{clean, local_hhmm};
use super::{Field, field};
use crate::app::App;
use crate::theme::ci_look;
use proto::{
    CiState, DeliveryInfo, DeliveryMode, PrState, RunInfo, RunState, StageInfo, StagePrInfo,
};

/// The run's delivery, in `pr` mode only.
fn delivery(run: &RunInfo) -> Option<&DeliveryInfo> {
    run.delivery.as_ref().filter(|d| d.mode == DeliveryMode::Pr)
}

/// The stage inspector's PR lines; none without a PR, or in a local run.
pub(super) fn stage_fields(run: &RunInfo, stage: &StageInfo, app: &App) -> Vec<Field> {
    match (delivery(run), &stage.pr) {
        (Some(_), Some(pr)) => pr_fields(pr, app),
        _ => Vec::new(),
    }
}

/// The run inspector's `delivery` row in `pr` mode (`pr to origin (fake/app) ·
/// watching every 60 s`), then, for a run with no stage node, its one PR's lines.
pub(super) fn run_fields(run: &RunInfo, app: &App) -> Vec<Field> {
    let Some(d) = delivery(run) else {
        return Vec::new();
    };
    let watching = if d.watching {
        format!("watching every {} s", d.poll_secs)
    } else {
        "not watching".to_owned()
    };
    let text = format!(
        "pr to {} ({}) · {watching}",
        clean(&d.remote),
        clean(&d.repo)
    );
    let mut fields = vec![field("delivery", text)];
    // Milestone 9.1: only a run of more than one stage has stage nodes.
    if run.stages.len() <= 1
        && let Some(pr) = run.stages.first().and_then(|s| s.pr.as_ref())
    {
        fields.extend(pr_fields(pr, app));
    }
    fields
}

/// Decision 36: a running `pr` run every stage of which has an open or merged PR (or
/// was skipped), one of them open, as the daemon says (`DeliveryInfo.delivering`).
pub(crate) fn delivering(run: &RunInfo) -> bool {
    run.state == RunState::Running && delivery(run).is_some_and(|d| d.delivering)
}

/// The header's state word: `running (delivering)` while decision 36 holds.
pub(super) fn state_text(run: &RunInfo) -> String {
    let state = crate::app::state_text(run.state);
    if delivering(run) {
        format!("{state} (delivering)")
    } else {
        state.to_owned()
    }
}

/// `pr`, `url`, `ci`, `threads` and `fix tasks`.
fn pr_fields(pr: &StagePrInfo, app: &App) -> Vec<Field> {
    let ascii = app.palette().ascii;
    let mut fields = vec![
        field("pr", pr_text(pr, app)),
        field("url", clean(&pr.url)),
        Field {
            label: "ci",
            value: ci_text(pr, ascii),
            wrap: true,
        },
        field("threads", threads_text(pr)),
    ];
    if let Some(fixes) = fix_tasks_text(&pr.fix_tasks) {
        fields.push(field("fix tasks", fixes));
    }
    fields
}

/// `#142 open, based on <base>, opened 14:02`, then `, merged 15:10` once merged and
/// ` · paused` while a lower stage's PR is closed.
fn pr_text(pr: &StagePrInfo, app: &App) -> String {
    let state = match pr.state {
        PrState::Open => "open",
        PrState::Merged => "merged",
        PrState::Closed => "closed",
    };
    let at = |secs| local_hhmm(secs, app.utc_offset_secs);
    let mut text = format!(
        "#{} {state}, based on {}, opened {}",
        pr.number,
        clean(&pr.base),
        at(pr.opened_at)
    );
    if let Some(merged) = pr.merged_at.filter(|_| pr.state == PrState::Merged) {
        text.push_str(&format!(", merged {}", at(merged)));
    }
    if pr.paused {
        text.push_str(" · paused");
    }
    text
}

/// Each check of the most recent head, `✓ build · ✗ test (fix3)` (the row wraps, so
/// the checks are `·`-separated rather than two spaces apart, which a wrap would fold);
/// with none, the PR's CI as a word.
fn ci_text(pr: &StagePrInfo, ascii: bool) -> String {
    if pr.checks.is_empty() {
        return match pr.ci {
            CiState::None => "no checks",
            CiState::Pending => "pending",
            CiState::Green => "green",
            CiState::Red => "red",
        }
        .to_owned();
    }
    let checks: Vec<String> = (pr.checks.iter())
        .map(|c| {
            let mut text = format!("{} {}", ci_look(c.state, ascii).0, clean(&c.name));
            if let Some(fix) = &c.fix_task {
                text.push_str(&format!(" ({})", clean(fix)));
            }
            text
        })
        .collect();
    checks.join(" · ")
}

/// `2 open, 1 addressed, 1 replied`, then `, 4 ignored` (non-writers and bots); `none`
/// before the first thread.
fn threads_text(pr: &StagePrInfo) -> String {
    let t = &pr.threads;
    if (t.new, t.tasked, t.replied, t.ignored) == (0, 0, 0, 0) {
        return "none".to_owned();
    }
    let mut text = format!(
        "{} open, {} addressed, {} replied",
        t.new, t.tasked, t.replied
    );
    if t.ignored > 0 {
        text.push_str(&format!(", {} ignored", t.ignored));
    }
    text
}

/// The snapshot's `"<id> <origin> <state>"` grouped by origin, in the order the
/// snapshot lists them (unfinished first): `ci: fix3 working; review: fix4 merged`. An
/// entry of another shape is shown as it came, cleaned.
fn fix_tasks_text(fix_tasks: &[String]) -> Option<String> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for entry in fix_tasks {
        let words: Vec<&str> = entry.split(' ').collect();
        let (origin, item) = match words.as_slice() {
            [id, origin, state] => (
                clean(origin),
                format!("{} {}", clean(id), clean(&state.replace('_', " "))),
            ),
            _ => (String::new(), clean(entry)),
        };
        match groups
            .iter_mut()
            .find(|(o, _)| *o == origin && !o.is_empty())
        {
            Some((_, items)) => items.push(item),
            None => groups.push((origin, vec![item])),
        }
    }
    let texts: Vec<String> = (groups.into_iter())
        .map(|(origin, items)| match origin.as_str() {
            "" => items.join(", "),
            _ => format!("{origin}: {}", items.join(", ")),
        })
        .collect();
    (!texts.is_empty()).then(|| texts.join("; "))
}
