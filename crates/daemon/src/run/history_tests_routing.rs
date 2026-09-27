//! Whole-branch review m2: routing decisions (decision 33a) are recorded only for a run
//! with history, the same gate as its records, so a run restored from milestone 8a
//! gains no `initial` decision for a session it launched before the upgrade.

use proto::{Effort, Runtime, Strength};

use super::fixtures::*;
use crate::run::model::ReviewLevel;
use crate::run::roster::{escalate, pick_reviewer};
use crate::run::routing::{record_reviewer, record_worker};

#[test]
fn a_run_without_history_records_no_routing_decisions() {
    let sonnet = route(
        Runtime::Claude,
        "claude-sonnet-5",
        Strength::Standard,
        Effort::Medium,
    );
    for history in [false, true] {
        let mut run = run_of(&["t1"]);
        run.history = history;
        let up = escalate(&run.roster, &sonnet);
        let task = &mut run.tasks[0];
        task.session = 2;
        task.escalated_from = Some(std::mem::replace(&mut task.route, up.clone()));
        record_worker(&mut run, 0, 100);
        assert_eq!(run.tasks[0].escalated_from, None, "history {history}");
        let chosen = pick_reviewer(&run.roster, &up, ReviewLevel::Medium);
        record_reviewer(&mut run, 0, (&up, ReviewLevel::Medium), &chosen, 1, 200);
        let recorded = run.tasks[0].routing_decisions.len();
        assert_eq!(recorded, if history { 2 } else { 0 }, "history {history}");
    }
}
