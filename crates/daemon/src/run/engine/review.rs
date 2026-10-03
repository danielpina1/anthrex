//! Decision 35's review gate (M8a.13): each round a fresh, read-only reviewer session
//! in a fresh review worktree, `submit_review` with severities, the rejection's rung-1
//! message, the nudge and replacement of a reviewer that ends without a verdict, and a
//! reviewer's exits and silence. Reviewer messages (the nudge) go through the run's
//! outbox addressed to the mailbox `<task>.review`: task ids cannot contain `.`
//! (decision 16), so every worker-side rule that matches messages by task id leaves
//! them alone. Pure (design decision 2).

use proto::{
    AgentRole, BlockReason, GateKind, RunState, Runtime, Severity, TaskState, ToolCall, Verdict,
};

use super::dispatch::{block, history, new_round, window_limit_reached};
use super::schedule::{hub_holds_slot, is_reader_task, needs_reviewer, readers_busy};
use super::signals::end_round;
use super::tools::parse_review;
use super::{Effect, OpId, OpKind, OpResult, ReplyId, emit_op, gates, ladder, next_op, weakening};
use crate::run::contract::{
    APPROVE_WITH_BLOCKING, REVIEW_RECORDED, review_changes_message, reviewer_prompt,
};
use crate::run::model::{AgentRound, ReviewLevel, ReviewRecord, Run};
use crate::run::orch::contract::worker_messages_for_review;
use crate::run::role_launch::{jitter_ms, reviewer_spec, session_uuid_of};
use crate::run::roster::pick_reviewer_skipping;
use crate::run::route_pick::{failed_routes, reviewer};
use crate::run::routing;

pub(super) use super::review_session::{exited, resume_failed, turn_ended, watch};

/// The outbox address of task `task`'s reviewer.
pub(super) fn mailbox(task: &str) -> String {
    format!("{task}.review")
}

/// The task whose reviewer `address` is, if it is a reviewer mailbox.
pub(super) fn mailbox_task(address: &str) -> Option<&str> {
    address.strip_suffix(".review")
}

/// Task `i`'s current reviewer round, if it has one.
pub(super) fn reviewer_round(run: &Run, i: usize) -> Option<usize> {
    run.tasks[i]
        .rounds
        .iter()
        .rposition(|r| r.role == AgentRole::Reviewer)
}

/// Reader dispatch: a `PrepareReview` for each task in `review` without a reviewer,
/// while reader slots are free (decision 41). A hub task holding a writer slot blocks
/// every dispatch, reviews included. The task awaits that op (`gate_op`).
pub(super) fn dispatch_reviewers(run: &mut Run, fx: &mut Vec<Effect>) {
    if hub_holds_slot(run) {
        return;
    }
    for i in super::schedule::dispatch_order(run) {
        if readers_busy(run) >= usize::from(run.limits.max_readers) {
            break;
        }
        if !needs_reviewer(run, &run.tasks[i]) || run.tasks[i].gate_op.is_some() {
            continue;
        }
        let task = &run.tasks[i];
        // Milestone 9 decision 36: a review task reviews its resolved range; one not
        // resolved yet is `kinds::dispatch`'s.
        let (head_ref, base_ref) = if is_reader_task(task) {
            match super::kinds::review_refs(task) {
                Some(refs) => refs,
                None => continue,
            }
        } else {
            // Ruling T13-I3: the claimed commit, never the branch tip. Final review
            // A-I5: the run head; the git layer diffs from its merge base with the
            // head (`run_head...head`), the task's net change, as the spill check
            // does. `start_commit` would bring in every hand-back's merged work.
            let head = task.head.clone().unwrap_or_else(|| task.branch.clone());
            // Milestone 9.1 decision 47: the task's stage head.
            (head, run.head_for(task).to_string())
        };
        let id = task.id().to_string();
        // Controller ruling C-21 (6): a sync task's resolution only.
        let base_tree = task.sync.as_ref().map(|s| s.base_tree.clone());
        let op = next_op(run);
        let kind = OpKind::PrepareReview {
            root: run.root.clone(),
            head_ref,
            base_ref,
            path: run.review_path(&id),
            base_tree,
        };
        run.tasks[i].gate_op = Some(op);
        emit_op(run, op, Some(&id), kind, fx);
    }
}

/// The result of the `PrepareReview` the task awaits: a fresh reviewer session
/// (decision 35) on `pick_reviewer` against the author's current route, and the
/// round's `ReviewRecord`, whose verdict `submit_review` fills. Reviewers are
/// read-only and their worktree is never watched.
pub(super) fn review_ready(
    run: &mut Run,
    i: usize,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].gate_op != Some(op) {
        return;
    }
    run.tasks[i].gate_op = None;
    if run.tasks[i].state != TaskState::Review {
        return;
    }
    let (base, head, patch) = match result {
        OpResult::Review { base, head, patch } => (base, head, patch),
        OpResult::Failed { message } => {
            let text = format!("could not prepare the review worktree: {message}");
            return block(run, i, BlockReason::Environment, text, now);
        }
        _ => return,
    };
    // Ruling T14-I2: no session starts while the run is not running. The worktree is
    // ready; the running pass prepares the review again (cheap, and its diff is not
    // kept in the run) and starts the reviewer then.
    if run.state != RunState::Running {
        let text = "review worktree prepared; the reviewer starts once the run runs";
        return history(run, i, now, text);
    }
    if window_limit_reached(run, i, now) {
        return;
    }
    let op = next_op(run);
    let task = &run.tasks[i];
    // Only a task with a review level enters `review` (`next_gate`), and `reach` counts
    // reviewers at the levels a task can have; a future path into review without one
    // would launch a reviewer `reach` never checked (T22 re-review 2, F4), so debug
    // builds refuse it.
    let reader = is_reader_task(task);
    debug_assert!(
        reader || task.review_level.is_some(),
        "task {} is in review with no review level",
        task.id()
    );
    let listed;
    let (author, level, route) = if reader {
        // Milestone 9 decisions 36, 37: a review task's own route and level; milestone
        // 9.5 ruling RL-4: a route the `review` list gave it records that list.
        let picked = task
            .list_pick
            .as_ref()
            .filter(|p| p.chosen_route() == Some(&task.route));
        listed = picked.map(|p| (Some(task.route.clone()), p.candidates.clone()));
        super::kinds::review_route(task)
    } else {
        let level = task.review_level.unwrap_or(ReviewLevel::Medium);
        // The reviewer is picked against the route of the session that wrote the
        // claimed commit, the last worker round's (an escalation to the peer runtime
        // included), so it stays on the other runtime. A route amended while that
        // session lived applies from the next fresh session (final review A-6).
        let author = task
            .rounds
            .iter()
            .rfind(|r| r.role == AgentRole::Worker)
            .map_or(&task.route, |r| &r.route)
            .clone();
        // Milestone 9.5 decision 9a: the `review` list's first qualifying candidate.
        let failed = failed_routes(task);
        let (lists, installed) = (&run.limits.route_lists, &run.orch.installed);
        listed = reviewer(lists, &author, level, installed, &failed);
        let first = listed.as_ref().and_then(|(route, _)| route.clone());
        let route =
            first.unwrap_or_else(|| pick_reviewer_skipping(&run.roster, &author, level, &failed));
        (author, level, route)
    };
    let spec = if reader {
        crate::run::orch::launch::review_task_spec(run, task, &route)
    } else {
        reviewer_spec(run, task, &route)
    };
    let round_no = spec.run_ref.as_ref().map_or(1, |r| r.session);
    let first_turn = if reader {
        super::kinds::review_first_turn(run, i, (&base, &head, &patch))
    } else {
        // Milestone 9 decision 42d: the `change` messages the worker received.
        let messages = worker_messages_for_review(&task.orch.messages);
        reviewer_prompt(run, task, round_no, &base, &head, &patch, &messages)
    };
    let name = format!("{}/{}.r{round_no}", run.short(), task.id());
    let uuid = (route.runtime == Runtime::Claude).then(|| session_uuid_of(run, op));
    let jitter = jitter_ms(&run.id, &format!("{}.r", task.id()), round_no);
    let mut round = new_round(
        AgentRole::Reviewer,
        round_no,
        route.clone(),
        op,
        uuid.clone(),
        now,
    );
    round.round = round_no;
    let id = task.id().to_string();
    let worktree = run.review_path(&id);
    // M8b decision 33a: decided before the session-start op.
    match listed {
        Some((_, list)) => routing::record_listed_reviewer(run, i, list, &route, round_no, now),
        None => routing::record_reviewer(run, i, (&author, level), &route, round_no, now),
    }
    let task = &mut run.tasks[i];
    task.review_route = Some(route.clone());
    weakening::new_review(task);
    task.rounds.push(round);
    task.reviews.push(ReviewRecord {
        round: round_no,
        route,
        base,
        head,
        verdict: None,
        summary: String::new(),
        findings: Vec::new(),
        lane: None,
    });
    run.windows_created += 1;
    history(run, i, now, format!("review round {round_no} starting"));
    let kind = OpKind::CreateWindow {
        name,
        spec: Box::new(spec),
        session_uuid: uuid,
        first_turn,
        project: run.project.clone(),
        worktree,
        jitter_ms: jitter,
        extract: None,
    };
    emit_op(run, op, Some(&id), kind, fx);
}

/// Stops task `i`'s live reviewer (a round given up, or a task overridden): the
/// engine's kill, and the mailbox and any resume it awaited dropped.
pub(super) fn stop_reviewers(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    for round in run.tasks[i]
        .rounds
        .iter_mut()
        .filter(|r| r.role == AgentRole::Reviewer && !r.retiring)
    {
        give_up(round, now, fx);
    }
    drop_mail(run, i);
}

/// The engine gives up a reviewer round: its process is killed, and the round holds
/// its reader slot until that exit (ruling T13-I2). A Codex reviewer whose last
/// process has exited (one per turn) has nothing to kill, so its round ends at once.
pub(super) fn give_up(round: &mut AgentRound, now: u64, fx: &mut Vec<Effect>) {
    round.retiring = true;
    round.resume_op = None;
    if round.ended {
        return;
    }
    // Ruling T13-R2 (N1): Codex emits `turn.completed` before its process exits, so a
    // closed turn is not enough: only a round whose last process has exited (no pid)
    // has nothing to wait for.
    if round.route.runtime == Runtime::Codex && !round.turn_open && round.pid.is_none() {
        return end_round(round, now);
    }
    if let Some(window_id) = round.window_id {
        fx.push(Effect::KillWindow { window_id });
    }
}

pub(super) fn drop_mail(run: &mut Run, i: usize) {
    let address = mailbox(run.tasks[i].id());
    run.outbox.retain(|m| m.task_id != address);
}

fn reply(fx: &mut Vec<Effect>, reply: ReplyId, result: Result<String, String>) {
    fx.push(Effect::Reply { reply, result });
}

/// `submit_review` (decision 35 and the MCP section's acceptance), in order: the
/// caller is a reviewer round of the task, its round has no verdict yet, it is the
/// task's live reviewer, the task is under review, the arguments are valid, and an
/// `approve` carries no blocking finding. The verdict is recorded and the reviewer
/// retired; any critical or important finding is a gate failure of `review`, else
/// the task goes to the merge queue (minor findings stay in the record).
pub(super) fn submit(run: &mut Run, id: ReplyId, call: &ToolCall, now: u64, fx: &mut Vec<Effect>) {
    let task_id = call.task_id.clone().unwrap_or_default();
    let not_reviewer = format!("this window is not the reviewer of task {task_id}");
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return reply(fx, id, Err(not_reviewer));
    };
    let found = (call.role == AgentRole::Reviewer)
        .then(|| {
            run.tasks[i]
                .rounds
                .iter()
                .rposition(|r| r.role == AgentRole::Reviewer && r.window_id == Some(call.window_id))
        })
        .flatten();
    let Some(r) = found else {
        return reply(fx, id, Err(not_reviewer));
    };
    let round_no = run.tasks[i].rounds[r].round;
    let decided = |rv: &ReviewRecord| rv.round == round_no && rv.verdict.is_some();
    if run.tasks[i].reviews.iter().any(decided) {
        let text = format!("a review for round {round_no} was already submitted");
        return reply(fx, id, Err(text));
    }
    let round = &run.tasks[i].rounds[r];
    if reviewer_round(run, i) != Some(r) || round.retiring || round.ended {
        return reply(fx, id, Err(not_reviewer));
    }
    let state = run.tasks[i].state;
    if state != TaskState::Review {
        let text = format!(
            "submit_review is accepted only while the task is under review (it is {})",
            state.label()
        );
        return reply(fx, id, Err(text));
    }
    let (mut verdict, summary, mut findings) = match parse_review(&call.args) {
        Ok(parsed) => parsed,
        Err(e) => return reply(fx, id, Err(format!("invalid arguments: {e}"))),
    };
    if verdict == Verdict::Approve && findings.iter().any(|f| f.severity != Severity::Minor) {
        return reply(fx, id, Err(APPROVE_WITH_BLOCKING.to_string()));
    }
    // Milestone 9.1 decision 42: every signal answered, or the engine's findings.
    let checked = weakening::check_review(&mut run.tasks[i], &mut verdict, &mut findings);
    if let Err(text) = checked {
        return reply(fx, id, Err(text));
    }
    let blocking = findings.iter().any(|f| f.severity != Severity::Minor);
    let task = &mut run.tasks[i];
    let route = task.rounds[r].route.clone();
    let k = match task.reviews.iter().position(|rv| rv.round == round_no) {
        Some(k) => k,
        None => {
            task.reviews.push(ReviewRecord {
                round: round_no,
                route,
                base: String::new(),
                head: String::new(),
                verdict: None,
                summary: String::new(),
                findings: Vec::new(),
                lane: None,
            });
            task.reviews.len() - 1
        }
    };
    let record = &mut task.reviews[k];
    record.verdict = Some(verdict);
    record.summary = summary;
    record.findings = findings;
    let record = record.clone();
    task.review_misses = 0;
    let round = &mut task.rounds[r];
    round.retiring = true;
    round.resume_op = None;
    if let Some(window_id) = round.window_id {
        fx.push(Effect::RetireWindow { window_id });
    }
    drop_mail(run, i);
    reply(fx, id, Ok(REVIEW_RECORDED.to_string()));
    // Milestone 9 decision 36: a review task reports whatever the verdict; nothing
    // is sent back and nothing merged.
    if is_reader_task(&run.tasks[i]) {
        return super::kinds::reviewed(run, i, now);
    }
    if blocking {
        history(
            run,
            i,
            now,
            format!("review round {round_no} asked for changes"),
        );
        let text = review_changes_message(&record);
        ladder::gate_failure(run, i, GateKind::Review, text, false, now, fx);
    } else {
        history(run, i, now, format!("review round {round_no} approved"));
        gates::enter(run, i, TaskState::MergeQueue, now);
    }
}

/// Whether round `r` of task `i` is its reviewer still owing a verdict: the current
/// reviewer round of a task under review, not given up, with no verdict recorded.
pub(super) fn owes_verdict(run: &Run, i: usize, r: usize) -> bool {
    let task = &run.tasks[i];
    let round = &task.rounds[r];
    task.state == TaskState::Review
        && reviewer_round(run, i) == Some(r)
        && !round.retiring
        && !task
            .reviews
            .iter()
            .any(|rv| rv.round == round.round && rv.verdict.is_some())
}
