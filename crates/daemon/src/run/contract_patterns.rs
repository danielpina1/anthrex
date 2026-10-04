//! Milestone 9.5 decisions 25 and 26: the test writer's contract and prompt, and the
//! texts that hand its red commit to the implementer and the reviewer (Interfaces,
//! "Contracts and texts (exact)"). Pure — no `std::fs`, `std::process`, `std::thread`,
//! `tokio` or `std::time::SystemTime` (design decision 1).

use super::contract::{handover_tail, sha7};
use super::messages::summary;
use proto::PairPhase;

use super::model::{Run, Task};

/// The test writer's system prompt (decision 25, exact): the worker contract's form and
/// words; lines 1, 6 and 8 to 12 are `WORKER_CONTRACT`'s, line 3 its line 3 with a
/// different first sentence.
pub const TEST_WRITER_CONTRACT: &str = "You are the test writer in an anthrex orchestration run. A different agent writes the implementation after you.
1. Work only in this worktree and only in the paths this task owns. Changing files outside them stops the task.
2. Write only the test named in your task prompt. It must check the behaviour the task asks for, and fail now because that behaviour does not exist yet. Do not implement the behaviour.
3. Commit the test: that commit is the red commit, and it must be your last commit. Its HEAD is detached: commit on it, and the engine records your commits on the task's branch. Never create, switch or push branches, and never rewrite commits already there. Commit new files: untracked files are not part of your work.
4. Run the single-test command from your task prompt once, and confirm the test fails.
5. Call the anthrex tool task_done (in Claude: mcp__anthrex__task_done) with a summary, test set to the test's name, and red set to your commit. Then stop.
6. If you cannot continue, call task_blocked (in Claude: mcp__anthrex__task_blocked): kind question if you need an answer, mis_sized if the task is bigger than one task, environment if a tool or setup is broken. Then stop.
7. Do not write the implementation, even to check your test: the next agent does.
8. Messages that start with [anthrex] come from the orchestration engine. Do what they say, commit, and call task_done again.
9. Nobody can answer a permission prompt. If a tool is denied, work without it or call task_blocked with kind environment.
10. If you learn something that affects other tasks or the plan, such as another place that must change, a wrong assumption in the brief, or a risk, report it with task_note (in Claude: mcp__anthrex__task_note) and keep working. Use task_blocked only when you cannot continue.
11. A message of kind change means the plan or the code around this task changed: in your next task_done summary, start with Changes applied: and say how you applied it. A message of kind stop_and_wait means finish your current step, commit anything worth keeping, and end your turn without calling task_done; wait for the next message.
12. After a commit, git may print Unable to create '.../packed-refs.lock': Operation not permitted. That is expected: the commit succeeded, and it needs no action. Do not try to fix it or change git settings.";

/// The first turn of a test writer's session (decision 25, Interfaces
/// "`test_writer_prompt`"): the task header as `worker_prompt` has it, the test and the
/// single-test command, the job, what the task owns and must meet, then the brief last.
pub fn test_writer_prompt(run: &Run, task: &Task) -> String {
    let spec = &task.spec;
    let start = task.start_commit.as_deref().unwrap_or(run.head_for(task));
    let test = spec.test_to_write.as_deref().unwrap_or_default();
    let single = run.profile.single_test.as_deref().unwrap_or_default();
    let mut lines = vec![
        format!("[anthrex] Test for task {}: {}", spec.id, spec.title),
        format!("Run goal: {}", run.goal),
        format!("Worktree: {}", task.worktree.display()),
        format!("Branch: {}", task.branch),
        format!("Start commit: {}", sha7(start)),
        format!("Test to write: {test}"),
        format!("Single-test command: {single}"),
        String::new(),
        format!(
            "Your job: write and commit only the failing test {test}. A different agent implements the behaviour after you."
        ),
        String::new(),
        "This task owns:".to_string(),
    ];
    lines.extend(spec.owns.iter().map(|glob| format!("- {glob}")));
    lines.push("Acceptance criteria:".to_string());
    lines.extend(spec.acceptance.iter().map(|item| format!("- {item}")));
    lines.push(String::new());
    lines.push(spec.brief.clone());
    lines.join("\n")
}

/// A fresh test writer's first turn (rung 2, `run retry`, a lost resume): the test
/// writer's prompt, then decision 30's hand-over part as a worker's has it.
pub fn test_writer_handover(
    run: &Run,
    task: &Task,
    reason: &str,
    stat: &str,
    patch: &str,
) -> String {
    let mut out = test_writer_prompt(run, task);
    out.push_str(&handover_tail(run, task, reason, stat, patch));
    out
}

/// Decision 26: what the implementer is told before its brief (exact).
pub fn pair_implementer_note(test: &str, red: &str) -> String {
    format!(
        "The failing test {test} was committed in {} by a separate test writer. Make it pass without weakening it; you may add more tests. Call task_done with test {test} and red {}, or leave both out.",
        sha7(red),
        sha7(red)
    )
}

/// Decision 26 (ruling RP-2): the reviewer prompt's one sentence on the signals, which
/// are measured from the red commit (exact).
pub fn pair_reviewer_note(test: &str, red: &str) -> String {
    format!(
        "A separate test writer committed the test {test} at {}; the weakening signals above are measured from that commit, so a W-signal on its files means the implementer changed the test.",
        sha7(red)
    )
}

/// Decision 25: the red-only proof saw the test pass at the red commit (exact).
pub fn red_check_failed_message(red: &str, test: &str, command: &str, tail: &str) -> String {
    format!(
        "[anthrex] The red check failed: at your red commit {} the test {test} passed, so it does not check missing behaviour. Change the test so it fails without the implementation, commit, then call task_done again.\nCommand: {command}\nLast 40 lines:\n{}",
        sha7(red),
        summary(tail)
    )
}

/// Decision 25: a test writer's claim whose red is not its head (exact; `none` when the
/// claim named no red).
pub fn writer_not_on_red(head: &str, red: Option<&str>) -> String {
    format!(
        "task_done rejected: you are the test writer, so red must be your last commit (HEAD is {}, red is {}). Commit only the failing test, then call task_done again.",
        sha7(head),
        red.map_or("none", sha7)
    )
}

/// Decision 26: an implementer's claim naming another test or red than the writer's
/// (exact).
pub fn implementer_wrong_test(test: &str, red: &str) -> String {
    format!(
        "task_done rejected: this task's test was written by the test writer; name test {test} and red {}, or leave both out",
        sha7(red)
    )
}

/// Decision 25's run log line once the red check confirms the test fails (exact).
pub fn red_confirmed_line(task: &str, red: &str) -> String {
    format!(
        "pair {task}: red {} fails as it should; implementer started",
        sha7(red)
    )
}

/// [`pair_implementer_note`] for task `task`, once its implementer has a red to make pass.
pub(crate) fn implementer_note(task: &Task) -> Option<String> {
    let pair = task
        .pair
        .as_ref()
        .filter(|p| p.phase == PairPhase::Implementing)?;
    Some(pair_implementer_note(
        pair.test.as_deref()?,
        pair.red.as_deref()?,
    ))
}

/// [`pair_reviewer_note`] for task `task`, once its implementer has a red.
pub(crate) fn reviewer_note(task: &Task) -> Option<String> {
    let pair = task
        .pair
        .as_ref()
        .filter(|p| p.phase == PairPhase::Implementing)?;
    Some(pair_reviewer_note(
        pair.test.as_deref()?,
        pair.red.as_deref()?,
    ))
}
