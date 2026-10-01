//! Milestone 9.1 decision 55: the run view's inspection of a stage (its progress,
//! branch, tier 3, bisect and fix tasks), and a task's `stage`, `origin` and `tier`
//! rows. Pure, as `run.rs` is.

use super::run_format::{clean, kind_glyph, rows};
use super::{Field, Inspection, field};
use crate::app::App;
use crate::tree::RowKind;
use proto::{FullState, RunInfo, StageInfo, TaskInfo, TaskOrigin, TierInfo};

/// Names a tier line shows before `+<k>`.
const NAMES_SHOWN: usize = 3;

/// A stage (decision 55): `<merged>/<tasks> merged`, the branch at its head or
/// `(not created)`, the tier-3 line, the bisect, and the fix tasks by origin.
pub(crate) fn stage_inspection(run: &RunInfo, stage: &StageInfo, app: &App) -> Inspection {
    let right = format!("{}/{} merged", stage.merged, stage.tasks);
    let branch = match &stage.head {
        Some(head) => format!("{} at {}", clean(&stage.branch), short(head)),
        None => format!("{} (not created)", clean(&stage.branch)),
    };
    let mut fields = vec![field("branch", branch), field("tier 3", tier3_text(stage))];
    if let Some(bisect) = bisect_text(stage) {
        fields.push(field("bisect", bisect));
    }
    if let Some(head) = &stage.propagate_red {
        let below = stage.n.saturating_sub(1);
        fields.push(field(
            "propagate",
            format!("stage {below} at {} is red here", short(head)),
        ));
    }
    if let Some(fixes) = fix_tasks_text(run, stage) {
        fields.push(field("fix tasks", fixes));
    }
    let glyph = kind_glyph(RowKind::Stage { run, stage }, app);
    rows(
        glyph,
        format!("stage {}/{}", stage.n, run.stages.len()),
        right,
        fields,
    )
}

fn short(sha: &str) -> String {
    clean(&sha.chars().take(7).collect::<String>())
}

/// `42s`, `2m10s`, `1h02m`.
pub(super) fn tier_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, secs % 3600 / 60)
    }
}

/// `a, b, c +2`.
fn names(list: &[String]) -> String {
    let shown: Vec<String> = list.iter().take(NAMES_SHOWN).map(|n| clean(n)).collect();
    let mut text = shown.join(", ");
    if list.len() > NAMES_SHOWN {
        text.push_str(&format!(" +{}", list.len() - NAMES_SHOWN));
    }
    text
}

/// `green at abc1234 · 41m12s · 2 shards · flaky a::x`; `not run` before any job.
fn tier3_text(stage: &StageInfo) -> String {
    let full = &stage.full;
    let mut parts = vec![
        match full.state {
            FullState::None => "not run",
            FullState::Running => "running",
            FullState::Green => "green",
            FullState::Red => "red",
            FullState::Bisecting => "bisecting",
        }
        .to_owned(),
    ];
    if let Some(commit) = &full.commit {
        parts[0].push_str(&format!(" at {}", short(commit)));
    }
    if let Some(secs) = full.secs {
        parts.push(tier_duration(secs));
    }
    if full.commit.is_some() {
        parts.push(match full.shards {
            1 => "1 shard".to_owned(),
            n => format!("{n} shards"),
        });
    }
    if !full.flaky.is_empty() {
        parts.push(format!("flaky {}", names(&full.flaky)));
    }
    if !full.failing.is_empty() {
        parts.push(format!("failing {}", names(&full.failing)));
    }
    parts.join(" · ")
}

/// While bisecting, or once a bisect made a fix task or gave up.
fn bisect_text(stage: &StageInfo) -> Option<String> {
    let full = &stage.full;
    let mut parts = Vec::new();
    if full.state == FullState::Bisecting {
        parts.push("running".to_owned());
    }
    match full.bisect_fixes {
        0 => {}
        1 => parts.push("1 fix task".to_owned()),
        n => parts.push(format!("{n} fix tasks")),
    }
    if let Some(note) = &full.note {
        parts.push(clean(note));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `fix1 (bisect of t2) · sync1 (sync)`, each with its origin.
fn fix_tasks_text(run: &RunInfo, stage: &StageInfo) -> Option<String> {
    let texts: Vec<String> = stage
        .fix_tasks
        .iter()
        .map(|id| match run.tasks.iter().find(|task| &task.id == id) {
            Some(task) => format!("{} ({})", clean(id), origin_text(task)),
            None => clean(id),
        })
        .collect();
    (!texts.is_empty()).then(|| texts.join(" · "))
}

fn origin_word(origin: TaskOrigin) -> &'static str {
    match origin {
        TaskOrigin::Plan => "plan",
        TaskOrigin::Bisect => "bisect",
        TaskOrigin::Sync => "sync",
        TaskOrigin::Ci => "ci",
        TaskOrigin::Review => "review",
    }
}

/// `bisect of t4` when the task says what it fixes, else its origin.
fn origin_text(task: &TaskInfo) -> String {
    match &task.fixes {
        Some(fixes) => clean(fixes),
        None => origin_word(task.origin).to_owned(),
    }
}

/// `tier 1: 3 modules · 2m10s · 1 cached · flaky t_x`, red marked `· red`.
fn tier_text(tier: &TierInfo) -> String {
    let mut parts = vec![
        format!("tier {}: {}", tier.tier, clean(&tier.affected)),
        tier_duration(tier.secs),
    ];
    if tier.cached > 0 {
        parts.push(format!("{} cached", tier.cached));
    }
    if !tier.flaky.is_empty() {
        parts.push(format!("flaky {}", names(&tier.flaky)));
    }
    if !tier.ok {
        parts.push("red".to_owned());
    }
    parts.join(" · ")
}

/// Decision 55's task rows: `stage` in a staged run, `origin` for a task the engine
/// made, and `tier` once a tier ran. A one-stage plan task gets none of them.
pub(super) fn task_fields(run: &RunInfo, task: &TaskInfo) -> Vec<Field> {
    let mut fields = Vec::new();
    if run.stages.len() > 1 {
        // Milestone 9.0.7 decision 15: the lifecycle is DETAIL's `phase`, so the stage
        // number is `stage`.
        fields.push(field(
            "stage",
            format!("{} of {}", task.stage, run.stages.len()),
        ));
    }
    if task.origin != TaskOrigin::Plan {
        let origin = origin_word(task.origin);
        let text = match &task.fixes {
            Some(fixes) => format!("{origin} · fixes: {}", clean(fixes)),
            None => origin.to_owned(),
        };
        fields.push(field("origin", text));
    }
    if let Some(tier) = &task.tier {
        fields.push(field("tier", tier_text(tier)));
    }
    fields
}
