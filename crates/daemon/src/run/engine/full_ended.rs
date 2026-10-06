//! Ruling C-27 (4): a stage red at its head whose last bisect fix task ended without
//! merging. A child of `full.rs`, split out to keep that file under 600 lines. Pure.

use proto::TaskState;

use super::super::{bisect, wake};
use super::{active, lacks_green, red_at_head, stage_mut};
use crate::run::contract::sha7;
use crate::run::model::{FixOf, Run};

/// Ruling C-27 (4), every scheduler pass of a running run: a stage red at its head
/// whose last bisect fix task ended without merging (cancelled or reported) waits
/// with a note, `fix task <id> ended without merging; tier 3 is red at <head7>`, shown
/// in `attention`, and the orchestrator is woken with it once (the note, once set, is
/// the record). Completion still waits on the red head.
pub(in crate::run::engine) fn fix_ended_pass(run: &mut Run) {
    if !active(run) {
        return;
    }
    let ended: Vec<(u16, String)> =
        run.stages
            .iter()
            .filter(|s| red_at_head(s) && lacks_green(run, s) && s.full.note.is_none())
            .filter(|s| s.bisect.is_none() && bisect::fix_open(run, s.n).is_none())
            .filter(|s| !super::spent(run, s.n))
            .filter_map(|s| {
                let fix = run.tasks.iter().rev().find(
                    |t| matches!(&t.fixes, Some(FixOf::Bisect { stage, .. }) if *stage == s.n),
                )?;
                (fix.state.is_finished() && fix.state != TaskState::Merged).then(|| {
                    let text = format!(
                        "fix task {} ended without merging; tier 3 is red at {}",
                        fix.id(),
                        sha7(&s.head)
                    );
                    (s.n, text)
                })
            })
            .collect();
    for (n, text) in ended {
        if let Some(s) = stage_mut(run, n) {
            s.full.note = Some(text.clone());
        }
        wake::note(run, text);
    }
}
