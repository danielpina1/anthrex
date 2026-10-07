use proto::{AgentRole, RunRef, Runtime};

use super::*;
use crate::run::orch::test_support::run_of;

fn session(role: AgentRole, task: Option<&str>) -> RunRef {
    RunRef {
        run_id: "r".into(),
        task_id: task.map(str::to_string),
        role,
        session: 1,
        lane: None,
    }
}

/// MR §7's text names the role table's row and the runtime.
#[test]
fn a_missing_program_names_the_role_and_the_way_out() {
    let run = run_of(1);
    let label = |role, task| role_label(Some(&run), &session(role, task));
    assert_eq!(
        not_found(&label(AgentRole::Reviewer, Some("t0")), Runtime::Codex),
        "reviewer: codex not found; choose another model in C-b S"
    );
    assert_eq!(label(AgentRole::Worker, Some("t0")), "implementer · small");
    assert_eq!(label(AgentRole::Scout, None), "research");
    assert_eq!(label(AgentRole::TestWriter, Some("t0")), "test writer");
    assert_eq!(label(AgentRole::Brainstormer, None), "brainstorm");
}
