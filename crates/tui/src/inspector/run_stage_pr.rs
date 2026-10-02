//! Milestone 9.2 decisions 36 and 42: a `pr`-mode run in the inspector. A stage's
//! pull request (`pr`, `url`), its CI per check of the most recent head, its threads
//! and its fix tasks by origin; the run's `delivery` row, and a single-stage run's PR
//! lines (it has no stage node); and the header's `running (delivering)`. Pure: the
//! time is formatted with the offset the `App` was given (`local_hhmm`), and every
//! text the host or an agent wrote goes through `clean`.

use super::run_format::{clean, local_hhmm};
use super::{Field, field};
use crate::app::App;
use crate::theme::{self, Role, ci_look};
use proto::{
    CiState, DeliveryInfo, DeliveryMode, PrState, RunInfo, RunState, StageInfo, StagePrInfo,
    TaskInfo,
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

/// Ruling R-13 (task M9.2.15's fix round): a `pr`-mode task's OUTCOME rows, 9.0.7's
/// heir to 9.0.5's STATUS and RESULT. `pr`: its stage's PR, `#142 open · ci red`,
/// `#141 merged`, `#140 closed`, else `skipped` or `no PR yet`; then `fixes`, what a
/// fix task fixes (the engine's text, cleaned). None in a local run.
pub(super) fn task_rows(run: &RunInfo, task: &TaskInfo) -> Vec<(&'static str, String)> {
    let Some(d) = delivery(run) else {
        return Vec::new();
    };
    let pr = run
        .stages
        .iter()
        .find(|s| s.n == task.stage)
        .and_then(|s| s.pr.as_ref());
    let text = match pr {
        Some(pr) if pr.state == PrState::Open => {
            format!("#{} open · ci {}", pr.number, ci_word(pr.ci))
        }
        Some(pr) if pr.state == PrState::Merged => format!("#{} merged", pr.number),
        Some(pr) => format!("#{} closed", pr.number),
        None if d.skipped_stages.contains(&task.stage) => "skipped".to_owned(),
        None => "no PR yet".to_owned(),
    };
    let mut rows = vec![("pr", text)];
    if let Some(fixes) = &task.fixes {
        rows.push(("fixes", clean(fixes)));
    }
    rows
}

/// A PR's CI as a word.
fn ci_word(ci: CiState) -> &'static str {
    match ci {
        CiState::None => "no checks",
        CiState::Pending => "pending",
        CiState::Green => "green",
        CiState::Red => "red",
    }
}

/// `pr`, `url`, `ci`, `threads` and `fix tasks`.
fn pr_fields(pr: &StagePrInfo, app: &App) -> Vec<Field> {
    let ascii = app.palette().ascii;
    let mut fields = vec![
        field("pr", pr_text(pr, app)),
        field("url", clean(&pr.url)),
        {
            let (value, marks) = ci_text(pr, ascii);
            Field {
                label: "ci",
                value,
                wrap: true,
                marks,
            }
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
/// with none, the PR's CI as a word. With it, the word index of each check's mark and
/// the role of its state (deferred from task 15): the panel colours those words, so a
/// check named `✓` is never drawn as a green mark. In ASCII the host's text is folded
/// here, so `inspect`'s fold changes no word count and the indexes hold.
fn ci_text(pr: &StagePrInfo, ascii: bool) -> (String, Vec<(usize, Role)>) {
    if pr.checks.is_empty() {
        return (ci_word(pr.ci).to_owned(), Vec::new());
    }
    let host = |text: &str| {
        let text = clean(text);
        if ascii {
            theme::ascii_twins(&theme::fold(&text, true))
        } else {
            text
        }
    };
    let (mut value, mut marks) = (String::new(), Vec::new());
    for (i, c) in pr.checks.iter().enumerate() {
        if i > 0 {
            value.push_str(" · ");
        }
        let (mark, role) = ci_look(c.state, ascii);
        marks.push((value.split_whitespace().count(), role));
        value.push_str(&format!("{mark} {}", host(&c.name)));
        if let Some(fix) = &c.fix_task {
            value.push_str(&format!(" ({})", host(fix)));
        }
    }
    (value, marks)
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
