//! Milestone 9.3's rounds, engine side (KG §2). Pure (design decision 2). Task 3 adds
//! [`ensure_first`]; the round's start, settling and widening follow in task 4a.

use crate::run::model::{Round, Run};

/// Decision 18: a run with no round record (one started now, or one from before
/// rounds) gets round 1 ([`Round::first`]).
pub(super) fn ensure_first(run: &mut Run) {
    if run.rounds.is_empty() {
        run.rounds.push(Round::first(run));
    }
}
