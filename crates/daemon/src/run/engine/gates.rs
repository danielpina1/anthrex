//! The gates after `done` (M8a.13): decision 33's fail-to-pass test proof, decision
//! 34's check, the order the gates run in (proof, check, review, then the merge
//! queue), and decision 35's `run override`. The review gate itself is `review.rs`.
//! A gate's op is awaited by its id (`Task.gate_op`, ruling T12-N's correlation): a
//! result for any other op, or for a task that has left the gate, is dropped. A gate
//! failure goes to decision 38's ladder (`ladder::gate_failure`); a gate that cannot
//! run at all (setup or git failed, the command could not start) blocks the task on
//! its environment and counts nothing. Pure (design decision 2).

use proto::{BlockReason, GateKind, RunState, TaskState, TestMode};

use super::dispatch::{block, history};
use super::{
    Effect, EngineState, OpId, OpKind, OpResult, OverrideCount, ReplyId, ScratchAt, emit_op,
    ladder, next_op, review,
};
use crate::run::contract::{check_failed_message, proof_failed_message};
use crate::run::env::profile_env;
use crate::run::model::{CheckRecord, ProofRecord, Run};
use crate::run::proof::{proof_command, proof_pattern};

/// The pattern a proof's output must match when the profile has no `test_passed`
/// (invented): the test's own name, escaped, on some line of the head run's output.
const NAME_ONLY: &str = "{test}";

/// The state a task enters after `passed` (`None`: its done claim was accepted), in
/// decision 32's order: `proof` for tdd, `check` when the profile has one, `review` when
/// the task is reviewed, else the merge queue.
pub(super) fn next_gate(run: &Run, i: usize, passed: Option<TaskState>) -> TaskState {
    const ORDER: [TaskState; 4] = [
        TaskState::Proof,
        TaskState::Check,
        TaskState::Review,
        TaskState::MergeQueue,
    ];
    let task = &run.tasks[i];
    let wanted = |state: TaskState| match state {
        TaskState::Proof => task.test_mode == TestMode::Tdd,
        TaskState::Check => run.profile.check.is_some(),
        TaskState::Review => task.review_level.is_some(),
        _ => true,
    };
    let from = passed.map_or(0, |p| {
        ORDER.iter().position(|&s| s == p).map_or(3, |k| k + 1)
    });
    ORDER[from..]
        .iter()
        .copied()
        .find(|&s| wanted(s))
        .unwrap_or(TaskState::MergeQueue)
}

/// Task `i` enters `state`; the merge queue is FIFO in arrival order (decision 36).
pub(super) fn enter(run: &mut Run, i: usize, state: TaskState) {
    run.tasks[i].state = state;
    run.tasks[i].gate_op = None;
    let id = run.tasks[i].id().to_string();
    if state == TaskState::MergeQueue && !run.merge_queue.contains(&id) {
        run.merge_queue.push(id);
    }
}

/// Task `i` passed `gate`: on to the next one.
fn passed(run: &mut Run, i: usize, gate: TaskState, now: u64) {
    let next = next_gate(run, i, Some(gate));
    enter(run, i, next);
    history(
        run,
        i,
        now,
        format!("{} passed; next: {}", gate.label(), next.label()),
    );
}

/// Every scheduler pass of a running run: a task in `proof` or `check` with nothing
/// awaited gets its op. A task that is in no gate awaits nothing, so a late result of
/// an earlier visit is dropped even if the task comes back to that gate.
pub(super) fn start_gates(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let state = run.tasks[i].state;
        // Ruling T13-minors (m4): the verdict-less count is review's alone.
        if state != TaskState::Review {
            run.tasks[i].review_misses = 0;
        }
        if !matches!(
            state,
            TaskState::Proof | TaskState::Check | TaskState::Review
        ) {
            run.tasks[i].gate_op = None;
            continue;
        }
        if run.tasks[i].gate_op.is_some() {
            continue;
        }
        match state {
            TaskState::Proof => start_proof(run, i, now, fx),
            TaskState::Check => start_check(run, i, now, fx),
            _ => {}
        }
    }
}

/// Decision 33: `Op Proof` for the claim's `test` and `red` at the task's head. A claim
/// that named neither (the turn-end fallback's) fails the proof at once, with the
/// message that names what is missing (decision 32).
fn start_proof(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    let head = task.head.clone().unwrap_or_default();
    let claim = task.done.as_ref();
    let named = claim.and_then(|c| c.test.clone().zip(c.red.clone()));
    let single = run.profile.single_test.clone().unwrap_or_default();
    let Some((test, red)) = named else {
        let record = ProofRecord {
            at: now,
            test: String::new(),
            red: String::new(),
            head,
            red_failed: false,
            head_passed: false,
            matched: false,
            red_tail: String::new(),
            head_tail: String::new(),
        };
        let text = proof_failed_message(&single, &record, NAME_ONLY);
        run.tasks[i].proofs.push(record);
        ladder::gate_failure(run, i, GateKind::Proof, text, false, now, fx);
        return;
    };
    let passed = run.profile.test_passed.as_deref().unwrap_or(NAME_ONLY);
    let id = task.id().to_string();
    let path = run.proof_path(&id);
    let kind = OpKind::Proof {
        root: run.root.clone(),
        path: path.clone(),
        red,
        head,
        command: proof_command(&single, &test),
        passed: proof_pattern(passed, &test),
        timeout_secs: run.profile.check_timeout_secs,
        setup: run.profile.setup.clone(),
        env: profile_env(&run.profile, &path),
    };
    let op = next_op(run);
    run.tasks[i].gate_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
    history(run, i, now, "running the test proof");
}

/// Decision 34: `Op Check` in the task's worktree.
fn start_check(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let Some(command) = run.profile.check.clone() else {
        // No check: the gate is skipped (the run is `unverified` from its start).
        return passed(run, i, TaskState::Check, now);
    };
    // Ruling T13-I3: on the claimed commit, in the task's scratch worktree, never on
    // whatever the branch tip has become since the claim.
    let task = &run.tasks[i];
    let id = task.id().to_string();
    let dir = run.proof_path(&id);
    let kind = OpKind::Check {
        env: profile_env(&run.profile, &dir),
        dir,
        command,
        timeout_secs: run.profile.check_timeout_secs,
        scratch: Some(ScratchAt {
            root: run.root.clone(),
            commit: task.head.clone().unwrap_or_default(),
            setup: run.profile.setup.clone(),
        }),
    };
    let op = next_op(run);
    run.tasks[i].gate_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
    history(run, i, now, "running the check");
}

/// Whether task `i` awaits `op` in `state`; either way, it awaits it no longer.
fn awaited(run: &mut Run, i: usize, op: OpId, state: TaskState) -> bool {
    let task = &mut run.tasks[i];
    if task.gate_op != Some(op) {
        return false;
    }
    task.gate_op = None;
    task.state == state
}

/// `Proof`'s result (decision 33): all three runs as required passes the gate;
/// anything else is a gate failure of `proof` with `proof_failed_message`.
pub(super) fn proof_done(
    run: &mut Run,
    i: usize,
    op: OpId,
    kind: &OpKind,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let OpKind::Proof {
        red,
        head,
        command,
        passed: pattern,
        ..
    } = kind
    else {
        return;
    };
    if !awaited(run, i, op, TaskState::Proof) {
        return;
    }
    match result {
        OpResult::Proof {
            red_failed,
            head_passed,
            matched,
            red_tail,
            head_tail,
        } => {
            let test = run.tasks[i]
                .done
                .as_ref()
                .and_then(|c| c.test.clone())
                .unwrap_or_default();
            let record = ProofRecord {
                at: now,
                test,
                red: red.clone(),
                head: head.clone(),
                red_failed,
                head_passed,
                matched,
                red_tail,
                head_tail,
            };
            let ok = red_failed && head_passed && matched;
            let text = proof_failed_message(command, &record, pattern);
            run.tasks[i].proofs.push(record);
            if ok {
                passed(run, i, TaskState::Proof, now);
            } else {
                ladder::gate_failure(run, i, GateKind::Proof, text, false, now, fx);
            }
        }
        OpResult::SetupFailed { output } => {
            let text = format!("setup failed in the proof worktree:\n{output}");
            block(run, i, BlockReason::Environment, text, now);
        }
        OpResult::Failed { message } => {
            let text = format!("could not run the test proof: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// `Check`'s result (decision 34): green passes the gate; red is a gate failure of
/// `check` with `check_failed_message` (the last 40 lines, deterministically).
pub(super) fn check_done(
    run: &mut Run,
    i: usize,
    op: OpId,
    kind: &OpKind,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let OpKind::Check { command, .. } = kind else {
        return;
    };
    if !awaited(run, i, op, TaskState::Check) {
        return;
    }
    match result {
        OpResult::Check {
            ok,
            code,
            timed_out,
            tail,
            secs,
        } => {
            let record = CheckRecord {
                at: now,
                ok,
                code,
                timed_out,
                tail,
                secs,
                on_candidate: false,
            };
            let text = check_failed_message(command, &record);
            run.tasks[i].checks.push(record);
            if ok {
                passed(run, i, TaskState::Check, now);
            } else {
                ladder::gate_failure(run, i, GateKind::Check, text, false, now, fx);
            }
        }
        OpResult::SetupFailed { output } => {
            let text = format!("setup failed in the check worktree:\n{output}");
            block(run, i, BlockReason::Environment, text, now);
        }
        OpResult::Failed { message } => {
            let text = format!("could not run the check: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// The refusal's tail when a task can go to the merge queue by no override.
const OVERRIDE_APPLIES: &str = "override applies only to a task in review, or blocked with commits";

/// Decision 35's override: a task in `review`, or `blocked` with at least one commit,
/// goes to the merge queue without review, marked for the report and exempt from the
/// spill checks from then on; it still passes the candidate check. A live reviewer is
/// stopped; the worker's session is left alone (ruling T13-minors m5). A held task
/// (M8a.6 ruling N5) or a `dep_cancelled` one is refused: its dependencies come first. A
/// blocked task whose commits no accepted claim recorded (M8a.15, the M8a.13 carry) has
/// them counted first (`CountCommits`): with none it is refused, else its branch's head
/// is what merges, and the reply waits for the count. A task that never started has no
/// commits.
pub(super) fn override_task(
    state: &mut EngineState,
    reply: ReplyId,
    run_id: &str,
    task_id: &str,
    reason: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let answer = |fx: &mut Vec<Effect>, result| fx.push(Effect::Reply { reply, result });
    let Some(run) = state.runs.get_mut(run_id) else {
        return answer(fx, Err(format!("unknown run {run_id}")));
    };
    if !matches!(run.state, RunState::Running | RunState::Paused) {
        return answer(fx, Err(format!("run {run_id} is {}", run.state.label())));
    }
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return answer(fx, Err(format!("unknown task {task_id}")));
    };
    if let Some(text) = override_refusal(run, i) {
        return answer(fx, Err(text));
    }
    let task = &run.tasks[i];
    if task.override_count.is_some() {
        let text = format!(
            "task {task_id}'s commits are being counted for an override; wait for its reply"
        );
        return answer(fx, Err(text));
    }
    // A blocked task's branch is counted even with an accepted claim: only a claim that
    // is still the branch's tip merges (T15-minors).
    if task.state == TaskState::Review {
        let text = send_to_queue(run, i, reason, now, fx);
        return answer(fx, Ok(text));
    }
    let Some(start) = task.start_commit.clone() else {
        return answer(
            fx,
            Err(format!("task {task_id} has no commits; {OVERRIDE_APPLIES}")),
        );
    };
    let kind = OpKind::CountCommits {
        worktree: task.worktree.clone(),
        start,
        run_head: run.run_head.clone(),
    };
    let op = next_op(run);
    run.tasks[i].override_count = Some(OverrideCount {
        op,
        reply,
        reason: reason.to_string(),
    });
    emit_op(run, op, Some(task_id), kind, fx);
    history(run, i, now, "counting its commits for an override");
}

/// Why task `i` cannot be overridden now, if it cannot: a held or `dep_cancelled` task
/// waits for its dependencies; any task but one in `review` or `blocked` is refused.
fn override_refusal(run: &Run, i: usize) -> Option<String> {
    let task = &run.tasks[i];
    let id = task.id();
    let dep_cancelled = task
        .block
        .as_ref()
        .is_some_and(|b| b.reason == BlockReason::DepCancelled);
    if task.awaiting_deps || dep_cancelled {
        return Some(format!(
            "task {id} waits for its dependencies; override it once they are merged"
        ));
    }
    if !matches!(task.state, TaskState::Review | TaskState::Blocked) {
        return Some(format!(
            "task {id} is {}; {OVERRIDE_APPLIES}",
            task.state.label()
        ));
    }
    None
}

/// Task `i` goes to the merge queue without review; the reply's text.
fn send_to_queue(run: &mut Run, i: usize, reason: &str, now: u64, fx: &mut Vec<Effect>) -> String {
    review::stop_reviewers(run, i, now, fx);
    let task = &mut run.tasks[i];
    task.merged_without_approval = Some(reason.to_string());
    task.block = None;
    enter(run, i, TaskState::MergeQueue);
    history(
        run,
        i,
        now,
        format!("overridden: to the merge queue without review ({reason})"),
    );
    let id = run.tasks[i].id();
    format!("task {id} goes to the merge queue without review: {reason}")
}

/// Whether task `i`'s override awaits `op`'s count.
pub(super) fn awaits_override(run: &Run, i: usize, op: OpId) -> bool {
    run.tasks[i]
        .override_count
        .as_ref()
        .is_some_and(|c| c.op == op)
}

/// The override's `CountCommits` result: at least one commit sends the task to the
/// merge queue at its branch's head, if it can still be overridden and any accepted
/// claim is that head. A hand-back due goes first (`merge::start_merge`).
pub(super) fn override_counted(
    run: &mut Run,
    i: usize,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(OverrideCount { reply, reason, .. }) = run.tasks[i].override_count.take() else {
        return;
    };
    let id = run.tasks[i].id().to_string();
    let result = match result {
        _ if override_refusal(run, i).is_some() => {
            Err(override_refusal(run, i).unwrap_or_default())
        }
        OpResult::Commits { count: 0, .. } => {
            Err(format!("task {id} has no commits; {OVERRIDE_APPLIES}"))
        }
        OpResult::Commits { head, .. }
            if run.tasks[i]
                .head
                .as_ref()
                .is_some_and(|claimed| *claimed != head) =>
        {
            Err(format!(
                "task {id} has commits after its accepted claim; retry it to have them checked"
            ))
        }
        OpResult::Commits { head, .. } => {
            run.tasks[i].head = Some(head);
            Ok(send_to_queue(run, i, &reason, now, fx))
        }
        OpResult::Failed { message } => {
            Err(format!("could not count task {id}'s commits: {message}"))
        }
        other => Err(format!("could not count task {id}'s commits: {other:?}")),
    };
    fx.push(Effect::Reply { reply, result });
}
