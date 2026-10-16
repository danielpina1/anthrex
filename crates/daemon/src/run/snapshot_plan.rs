//! Milestone 9.17's snapshot additions (ruling C1): decision 10's sticky attention line
//! for a `brief` note in a run nobody plans with. Pure: it reads the run, no I/O.

use proto::TaskNoteKind;

use super::model::Run;

/// Decision 10 (`BRIEF_ATTENTION`): for a non-terminal run with no orchestrator record,
/// one line per task with at least one `brief` note, `<t>: the reviewer says the brief
/// is wrong (<n> notes); read them in the task's notes`. It stays while the task has
/// the notes, so later notes never push it out as they do `noted_lines`.
pub(super) fn brief_attention(run: &Run) -> Vec<String> {
    if run.state.is_terminal() || run.orch.orchestrator.is_some() {
        return Vec::new();
    }
    (run.tasks.iter())
        .filter_map(|t| {
            let notes = t.orch.worker_notes.iter();
            let n = notes.filter(|n| n.kind == TaskNoteKind::Brief).count();
            (n > 0).then(|| {
                format!(
                    "{}: the reviewer says the brief is wrong ({n} notes); read them in the task's notes",
                    t.id()
                )
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "snapshot_plan_tests.rs"]
mod tests;
