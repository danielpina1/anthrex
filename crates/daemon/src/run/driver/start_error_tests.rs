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

/// Fix round 1 (M3): only a spawn marked as finding no program names the role; any
/// other `ENOENT` (a vanished worktree) is the bare error.
#[test]
fn only_a_missing_program_names_the_role() {
    let gone = std::io::Error::from(std::io::ErrorKind::NotFound);
    let bare = anyhow::Error::new(gone).context("could not start claude");
    assert_eq!(
        named_as("research", Runtime::Claude, &bare),
        "could not start claude"
    );
    let program = crate::headless::session::ProgramNotFound::new(
        "claude".into(),
        std::io::Error::from(std::io::ErrorKind::NotFound),
    );
    let error = anyhow::Error::new(program).context("could not start the session");
    assert_eq!(
        named_as("research", Runtime::Claude, &error),
        "research: claude not found; choose another model in C-b S"
    );
}
