//! Milestone 9.0.6 decision 11: each action's label and effect line, naming exact
//! things from the run alone (no git), as Interfaces "Exact user-visible text" has
//! them. `available` sanitises and caps what these return. Pure (design decision 2).

use proto::{ActionKind, AgentRole, Effort, HoldState};

use super::ActionNode;
use crate::run::contract::sha7;
use crate::run::engine::full;
use crate::run::model::{Run, Task};
use crate::run::roster::escalate;

/// The menu's label for `kind`.
pub(crate) fn label(kind: &ActionKind) -> String {
    use ActionKind::*;
    match kind {
        ReviewPlan => "review plan".into(),
        Approve => "approve".into(),
        Reject => "reject".into(),
        Submit => "submit".into(),
        ApproveHold { hold } => format!("approve hold {hold}"),
        RejectHold { hold } => format!("reject hold {hold}"),
        Pause => "pause".into(),
        Unpause => "resume".into(),
        Resume => "resume halted run".into(),
        Cancel => "cancel run".into(),
        Promote => "promote".into(),
        Accept => "accept".into(),
        Discard => "discard".into(),
        Stats => "stats".into(),
        MessageStage { stage } => format!("message stage {stage}"),
        Answer => "answer".into(),
        Message => "message".into(),
        Refresh => "refresh".into(),
        Retry => "retry".into(),
        Override => "override".into(),
        CancelTask => "cancel task".into(),
        OpenConversation => "open conversation".into(),
    }
}

/// `1 task` / `<n> tasks`.
fn tasks(n: usize) -> String {
    plural(n, "task")
}

fn plural(n: usize, word: &str) -> String {
    match n {
        1 => format!("1 {word}"),
        n => format!("{n} {word}s"),
    }
}

fn unfinished(run: &Run) -> impl Iterator<Item = &Task> {
    run.tasks.iter().filter(|t| !t.state.is_finished())
}

/// What `kind` on `node` does, in one line.
pub(super) fn effect(run: &Run, node: &ActionNode, kind: &ActionKind) -> String {
    use ActionKind::*;
    let (id, base) = (&run.id, &run.base_branch);
    let task = match node {
        ActionNode::Task(t) => run.task(t),
        _ => None,
    };
    let t = task.map_or("", |t| t.id());
    match kind {
        Approve => {
            let n = unfinished(run).count();
            let starts = if n == 1 { "starts" } else { "start" };
            let base7 = sha7(&run.base_sha);
            format!(
                "approve: {} {starts}, each in its own worktree off {base}@{base7}",
                tasks(n)
            )
        }
        Reject => format!(
            "reject: discard run {id}: its worktrees and anthrex/{id}/* branches go; {base} is unchanged"
        ),
        Submit => format!(
            "submit: the plan of {} goes to the plan gate",
            tasks(unfinished(run).count())
        ),
        ApproveHold { hold } => {
            format!("approve hold {hold}: {} may start", tasks(hold_tasks(run, hold)))
        }
        RejectHold { hold } => {
            let n = hold_tasks(run, hold);
            let are = if n == 1 { "is" } else { "are" };
            format!(
                "reject hold {hold}: its {} {are} cancelled; none has started",
                tasks(n)
            )
        }
        Pause => "pause: no new task, gate or delivery starts; open turns finish".into(),
        Unpause => "resume: dispatch, gates and deliveries start again".into(),
        Resume => resume(run),
        Cancel => {
            let workers = run.tasks.iter().filter(|t| has_worker(t)).count();
            let tasks = plural(unfinished(run).count(), "unmerged task");
            format!(
                "cancel: stop {} and cancel {tasks}; the run then completes",
                plural(workers, "worker")
            )
        }
        Promote => "promote: give this fast-path run an orchestrator; what it adds waits under hold promotion".into(),
        Accept => {
            let merged = run
                .tasks
                .iter()
                .filter(|t| t.state == proto::TaskState::Merged)
                .count();
            let onto = match &run.base_moved {
                Some(m) => format!("{}@{} (moved from {})", base, sha7(&m.to), sha7(&m.from)),
                None => format!("{base}@{}", sha7(&run.base_sha)),
            };
            format!("accept: merge {} into {onto}", tasks(merged))
        }
        // Preflight F1: a salvage ref keeps only a dirty worktree's work.
        Discard => {
            // Milestone 9.2 ruling R-1: a `pr`-mode discard is local only.
            let (local, never) = match crate::run::engine::delivery::pr(run) {
                true => ("local ", ", never a PR or a remote branch"),
                false => ("", ""),
            };
            format!(
                "discard: remove the run's {local}worktrees and anthrex/{id}/* branches{never} (uncommitted work is kept under refs/anthrex/salvage/{id}/); {base} is unchanged"
            )
        }
        MessageStage { stage } => {
            let k = unfinished(run).filter(|t| t.stage() == *stage).count();
            match k {
                1 => format!("message stage {stage}: 1 unfinished task gets it at its next turn"),
                k => format!(
                    "message stage {stage}: {k} unfinished tasks get it at their next turn"
                ),
            }
        }
        Answer => format!("answer {t}: its worker gets the answer in the same session"),
        Message => format!("message {t}: its worker gets it at its next turn"),
        Refresh => {
            let head = task.map_or(run.run_head.as_str(), |task| run.head_for(task));
            format!(
                "refresh {t}: merge the run's latest merged work ({}) into its branch at its next turn boundary",
                sha7(head)
            )
        }
        Retry => {
            let route = task.map(|task| escalate(&run.roster, &task.route));
            let (runtime, model, effort) = route.map_or_else(Default::default, |r| {
                let model = if r.model.is_empty() { "default".to_string() } else { r.model };
                (r.runtime.label(), model, effort_label(r.effort))
            });
            format!("retry {t}: a fresh session at rung 2 on {runtime} {model} ({effort} effort)")
        }
        Override => format!("override {t}: to the merge queue without review"),
        CancelTask => {
            let k = unfinished(run)
                .filter(|d| d.id() != t && d.spec.deps.iter().any(|dep| dep == t))
                .count();
            match k {
                0 => format!("cancel {t}: its worker stops; nothing depends on it"),
                1 => format!(
                    "cancel {t}: its worker stops; 1 dependent becomes blocked(dep_cancelled)"
                ),
                k => format!(
                    "cancel {t}: its worker stops; {k} dependents become blocked(dep_cancelled)"
                ),
            }
        }
        ReviewPlan => "review plan: read every task before approving".into(),
        Stats => "stats: this project's run history".into(),
        OpenConversation => format!("open conversation: {t}'s conversation, read-only"),
    }
}

/// Resume: the halt's first line, or (preflight F32) the held stage's tier 3.
fn resume(run: &Run) -> String {
    if run.state == proto::RunState::Running && full::held(run) {
        let stage = run
            .stages
            .iter()
            .find(|s| full::infra_held(s))
            .map_or(1, |s| s.n);
        return format!("resume: tier 3 retries on stage {stage}");
    }
    let reason = run
        .halted_reason
        .as_deref()
        .and_then(|r| r.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("run halted");
    format!(
        "resume: {reason}; rebaseline reads {} and the run head again",
        run.base_branch
    )
}

/// The live tasks hold `hold` keeps.
fn hold_tasks(run: &Run, hold: &str) -> usize {
    run.orch
        .gate_holds
        .iter()
        .find(|h| h.id == hold && h.state == HoldState::Awaiting)
        .map_or(0, |h| {
            h.tasks
                .iter()
                .filter(|t| run.task(t).is_some_and(|t| !t.state.is_finished()))
                .count()
        })
}

fn has_worker(task: &Task) -> bool {
    task.rounds
        .iter()
        .any(|r| r.role == AgentRole::Worker && !r.ended)
}

fn effort_label(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}
