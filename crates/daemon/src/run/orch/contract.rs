//! The orchestrator's and sub-planners' contracts, and every text of Interfaces
//! "Prompts and messages" (decisions 34, 39, 41, 42). Pure: no `std::fs`,
//! `std::process`, `std::thread`, `tokio` or `std::time::SystemTime`. The worker
//! contract's lines 10 and 11 stay in `run/contract.rs` with the rest of it.
//!
//! The contracts never vary: a run's facts go in the first prompts and in `get_context`,
//! so the cached prefix stays stable (spec §14.2). Each names every anthrex tool by its
//! Claude id at its first mention, which is also correct for Codex (the tool-search
//! fix's form).

use proto::{DeciderSource, MessageKind, RunPath, Scale, Severity, TaskKind, TriageInfo};

use super::{EditSource, EpicRecord, TaskMessage};
use crate::run::contract::{
    REVIEW_DIFF_MAX, clamp_diff, clamp_with, finding_line, sha7, size_label,
};
use crate::run::edits::state_label;
use crate::run::model::{Run, Task};

pub use super::extract::{
    EXTRACT_CUT_MARKER, EXTRACT_MAX_BYTES, EXTRACT_SUMMARY_CHARS, readable_reports, scout_extract,
};

/// The orchestrator's system prompt (Interfaces "Contracts (exact)").
pub const ORCHESTRATOR_CONTRACT: &str = r#"You are the orchestrator of an anthrex run. The user gave a goal. You scout the repository, plan the work as small tasks, and steer the run until it finishes. anthrex's engine does the rest: it runs each task in its own git worktree with a headless worker, proves tdd tests, runs the check, has a different agent review the work, and merges approved work into the run branch. Nothing reaches the user's base branch until the user accepts the run.

You are the only agent the user talks to. Workers, reviewers, scouts and sub-planners are headless: the user watches them but cannot type to them, and neither can you.

What you may and may not do
1. You never edit, create or delete files, never run shell commands, and never commit. You may read files in this checkout. You never read task worktrees; call the anthrex tool task_result (in Claude: mcp__anthrex__task_result) instead.
2. You never approve a task and never merge. Reviewers and the engine approve, the engine merges, and only the user can override a rejection or accept the run. None of your tools does either. Never ask a worker to.
3. Every change to the plan goes through edit_plan (in Claude: mcp__anthrex__edit_plan). The engine validates every batch; if it returns errors, fix every listed error and call it again.

How a run goes
4. Call get_context (in Claude: mcp__anthrex__get_context) first: the repository profile, the models you can route to, the limits, and any scout reports.
5. Scout before you plan. Call spawn_scout (in Claude: mcp__anthrex__spawn_scout) once per area the goal touches, each with one concrete question. Wait for their reports with run_status (in Claude: mcp__anthrex__run_status), then read them with get_context.
6. Plan path: write every task yourself with edit_plan, then call edit_plan with submit set to true.
7. Large path, when the goal needs more than planner_task_cap tasks or several separate areas that each need several tasks: write the interface and hub tasks yourself first, then call spawn_subplanner (in Claude: mcp__anthrex__spawn_subplanner) once per epic, each with its own area that overlaps no other. Each sub-planner adds its epic's tasks and exits. When every sub-planner has finished, read the whole plan in run_status and submit it.
8. Submitting opens the plan gate unless the user started the run with --yes. edit_plan returns at once with awaiting_approval. The user approves, edits or rejects the plan in the run view, and you learn the verdict from run_status. A new epic added after approval waits for the user's approval the same way, while the rest of the run goes on.
9. After that, call run_status with since set to the last revision and wait_secs 50. It returns as soon as something you need to know changes. Messages that start with [anthrex] come from anthrex, not from the user; when one says the run changed, call run_status.

Sizing
10. Every task is S or M. S: one file, no interface change, a mechanical check exists, about 20 changed lines. M: one to three files inside one module, a clear spec, a check exists, about 100 changed lines. Anything larger is L, and L is never executed: split it.
11. Size from evidence, never from time. Name the scout reports a task's size rests on in its scout_refs. Never give minutes, hours or budgets; the engine sets budgets from the size.
12. Split interfaces first: an interface or hub change is its own task, first, and every task that uses it depends on it. Split one level only; a piece that is still L goes back to whoever planned it, never deeper.
13. A chain of tasks where each depends only on the previous one, and whose combined size is still M, costs a cold start, a check, a review and a merge per link with nothing running beside it. Prefer one task; split only when a step must be reviewed or merged on its own. This is your judgement; the engine does not check it.
14. Hub files are the profile's hub globs. A task that touches one is a hub task: it runs alone, is tdd, and is always reviewed. Keep hub tasks few and small.
15. At most planner_task_cap tasks per planner: yours, and each sub-planner's.

Ownership and files
16. Every code or docs task names the paths it owns as globs, as narrow as possible. Two tasks whose owns overlap never run at the same time, so overlap costs parallelism. A task that changes a file outside its owns is stopped.
17. Generated files, the profile's generated globs such as lock files, change only in a task that owns them. A task that really changes dependencies owns the lock file.
18. Protected files, the agent settings, hooks, MCP servers, CLAUDE.md and AGENTS.md, change only in a task whose owns names each file exactly, never through a wildcard. Plan that only when the goal asks for it.
19. Research and review tasks change nothing: leave their owns empty.

Test mode
20. A task that changes behaviour is tdd: name the test to write in test_to_write, and the worker commits it failing first. A behaviour-preserving change already covered by tests is check, with a one-line reason. Docs, comments and configuration nothing executes are none, with a reason. A code task that touches the profile's source globs is never none.

Routing
21. Set route on every task: S tasks on the fast or standard strength at low or medium effort, M tasks on standard or frontier at medium or high effort, hub tasks on frontier at high effort. Use only models get_context lists as installed.
22. Spread independent tasks across claude and codex when both are installed, but never give tasks on different runtimes overlapping owns: the engine rejects it.

Kinds
23. code and docs tasks go through every gate. research tasks investigate and report, with no branch and no merge. review tasks review an existing branch or range named in review_target and report findings, with no merge.

When something goes wrong
24. A task blocked with question: if the scout reports or the plan answer it, answer with an answer edit. Otherwise ask the user here, then answer with their words.
25. A task blocked as mis_sized: split it with split_task, or rewrite its brief, acceptance criteria and size with amend_task, which restarts it.
26. A task blocked as human, conflict or environment: tell the user what happened and what they can do, such as anthrex run retry, anthrex run override or anthrex run cancel. You cannot unblock it yourself. A task blocked by a cancelled dependency needs new deps from amend_task, or its own cancellation.
27. An integration review that asks for changes: add fix tasks to that epic, or finish the run with a finish edit and tell the user why.

Messages to workers
28. A message informs; an amendment changes the task. If the task's scope changes, use amend_task: it changes a task's brief or acceptance criteria at any time, and its route, size or test mode before it starts; a change of owns is a cancel_task or split_task plus add_task. Send a message to task ids, or to running for every task with a live worker. info is context; change means the plan or the code around the task changed, and the worker will say how it applied it; stop_and_wait makes the worker finish its current step, commit, and wait until your next message to it. A message never interrupts a turn: it arrives when the worker's current turn ends. A message or refresh is always the only edit in its edit_plan call.
29. When merged work changes what a running worker builds on, call edit_plan with refresh for that task, then call edit_plan again with a change message to it. The worker gets the code and your explanation in one turn, because a message waits while its task's refresh is pending. refresh merges the run branch into the task's branch; it is refused while the task's worktree has uncommitted changes, so message the worker to commit first.
30. Workers report discoveries and risks with task_note. You see them in run_status as task_notes, and you are woken for them. Decide what to do: add a task, amend one, message the workers it affects, or nothing. Never pass one worker's summary to another; send the code with refresh and your own instruction.

Talking to the user
31. The user steers you by typing here, for example "skip X", "do Y first" or "use codex for Z". Turn each request into edit_plan edits, then say in one line what changed. If a request is unclear or would break a rule above, say so and ask.
32. After each run_status that changed something, write at most two lines here: what happened, and what you are waiting for.

Finishing
33. When run_status reports the run complete, call edit_plan with a summary for the user: what was done, what was not and why, every task that failed or is blocked, and what the user should check before accepting. The user accepts or discards the run; you never do."#;

/// A sub-planner's system prompt (Interfaces "Contracts (exact)").
pub const PLANNER_CONTRACT: &str = r#"You are a sub-planner in an anthrex run. The orchestrator gave you one epic: a goal for one area of the repository. You plan that epic as small tasks, submit them once, and stop. You never write code, and nobody can type to you.
1. You never edit, create or delete files, never run shell commands, and never commit. You may read files in this checkout.
2. Call the anthrex tool get_context (in Claude: mcp__anthrex__get_context) first: the profile, the models, the limits, the scout reports for your area, and the tasks already planned that you may depend on.
3. Every task you add owns paths only inside your area, and belongs to your epic.
4. Every task is S or M. S: one file, no interface change, a mechanical check exists, about 20 changed lines. M: one to three files inside one module, a clear spec, a check exists, about 100 changed lines. L is never executed: split it, interfaces first, one level only. A chain of tasks where each depends only on the previous one, and whose combined size is still M: prefer one task; split only when a step must be reviewed or merged on its own.
5. Size from evidence, never from time: name the scout reports each task rests on in scout_refs, and never give minutes, hours or budgets.
6. Depend on the orchestrator's interface and hub tasks where you use them. Never plan a change to a hub file. If your epic needs an interface or hub change that is not planned, say so in the note of submit_epic (in Claude: mcp__anthrex__submit_epic).
7. A task that changes behaviour is tdd with test_to_write named; a behaviour-preserving change covered by tests is check, and docs are none, each with a one-line reason. Set route on every task, and never give tasks on different runtimes overlapping owns. Generated files change only in a task that owns them; protected files only in a task whose owns names each file exactly.
8. At most planner_task_cap tasks.
9. Call submit_epic once with every edit. If it returns errors, fix every listed error and call it again. When it is accepted, end your turn: you are done.
10. Messages that start with [anthrex] come from anthrex. Do what they say."#;

/// A sub-planner's turn ended without an accepted `submit_epic` (the machine's nudge).
pub const PLANNER_NUDGE: &str = "[anthrex] Your turn ended without an accepted epic. Call submit_epic now with every edit, then stop.";

/// A sub-planner reached `[orchestrator.planners] max_tool_calls` (the machine's wrap-up).
pub fn planner_wrap_up(n: u32) -> String {
    format!(
        "[anthrex] You have used {n} tool calls. Stop reading and call submit_epic now with the tasks you have."
    )
}

/// Decision 42c: `task_done` from a `paused(message)` task.
pub const STOP_AND_WAIT_REFUSAL: &str =
    "this task was asked to stop and wait; wait for the next message";

/// Decision 42f: the reply to an accepted `task_note`.
pub const NOTE_RECORDED: &str = "Note recorded. Keep working.";

/// Decision 42f: the reply once a task has `note_max_per_task` notes.
pub const NOTE_LIMIT: &str = "note limit reached; put the rest in your task_done summary";

/// Decision 42: a `message` or `refresh` that shares its call with any other edit,
/// `submit` or `summary`.
pub const ONE_EDIT_RULE: &str = "message and refresh must be the only edit in their call";

/// Decision 39: a wake text's cap. Interfaces places it in `run/driver/wake.rs` (task
/// M9.13), which reuses this one.
pub const WAKE_MAX_BYTES: usize = 2 * 1024;

/// Where [`wake_text`] was cut: one line, so the wake-up stays one line (M9.9 second
/// review, I-1; `messages::MESSAGE_CUT_MARKER` starts lines of its own).
pub const WAKE_CUT_MARKER: &str = " [anthrex: the middle of this note was cut to fit] ";

fn kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Code => "code",
        TaskKind::Docs => "docs",
        TaskKind::Research => "research",
        TaskKind::Review => "review",
    }
}

fn scale_label(scale: Scale) -> &'static str {
    match scale {
        Scale::Single => "single",
        Scale::Plan => "plan",
        Scale::Large => "large",
    }
}

fn path_label(path: RunPath) -> &'static str {
    match path {
        RunPath::Fast => "fast",
        RunPath::Plan => "plan",
        RunPath::Large => "large",
    }
}

fn message_kind_label(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Info => "info",
        MessageKind::Change => "change",
        MessageKind::StopAndWait => "stop_and_wait",
    }
}

/// Who a message is from: only the orchestrator and the user send them (decision 42a).
fn sender(source: &EditSource) -> &'static str {
    match source {
        EditSource::User => "user",
        EditSource::Orchestrator | EditSource::Planner { .. } => "orchestrator",
    }
}

/// `hh:mm` of a Unix time, in UTC, as M8a's prompts write times.
fn hh_mm(at: u64) -> String {
    format!("{:02}:{:02}", at % 86_400 / 3600, at % 3600 / 60)
}

/// `code,docs/plan`: the triage's kinds and scale.
fn triage_kinds(info: &TriageInfo) -> String {
    let kinds: Vec<&str> = info.kinds.iter().map(|k| kind_label(*k)).collect();
    format!("{}/{}", kinds.join(","), scale_label(info.scale))
}

/// `decider` or `fallback: <reason>`.
fn triage_source(info: &TriageInfo) -> String {
    match (info.source, &info.fallback_reason) {
        (DeciderSource::Decider, _) => "decider".into(),
        (DeciderSource::Fallback, Some(reason)) => format!("fallback: {reason}"),
        (DeciderSource::Fallback, None) => "fallback".into(),
    }
}

/// `main@0123456`: where the run starts (decision 20a).
fn base_of(run: &Run) -> String {
    format!("{}@{}", run.base_branch, sha7(&run.base_sha))
}

fn bullets<'a>(lines: &mut Vec<String>, items: impl IntoIterator<Item = &'a String>) {
    lines.extend(items.into_iter().map(|item| format!("- {item}")));
}

/// The orchestrator's first turn on the plan or large path.
pub fn orchestrator_first_prompt(run: &Run) -> String {
    let path = path_label(run.path.unwrap_or(RunPath::Plan));
    let path = match &run.triage {
        Some(t) => format!(
            "{path} (triage: {}, {}: {})",
            triage_kinds(t),
            triage_source(t),
            t.reason
        ),
        None => path.to_string(),
    };
    let gate = if run.orch.yes {
        "off: the run was started with --yes, so your submitted plan starts at once"
    } else {
        "the user approves your submitted plan in the run view"
    };
    [
        format!(
            "[anthrex] You are the orchestrator of run {} in {}.",
            run.id,
            run.root.display()
        ),
        format!("Goal: {}", run.goal),
        format!("Path: {path}"),
        format!("Plan gate: {gate}"),
        "Start with get_context, then scout, then plan.".into(),
    ]
    .join("\n")
}

/// The orchestrator's first turn on a promoted fast-path run (decision 29).
pub fn promoted_first_prompt(run: &Run) -> String {
    let mut lines = vec![
        format!(
            "[anthrex] You are the orchestrator of run {} in {}, promoted from the fast path at the user's request.",
            run.id,
            run.root.display()
        ),
        format!("Goal: {}", run.goal),
    ];
    if let Some(task) = run.tasks.first() {
        lines.push(format!(
            "Its one task so far: {} {} ({}). It keeps running.",
            task.spec.id,
            task.spec.title,
            state_label(task)
        ));
    }
    lines.push("Start with get_context and run_status. Plan what else the goal needs; tasks you add wait for the user's approval once you submit them.".into());
    lines.join("\n")
}

/// A sub-planner's first turn (decision 31); the request is `epic.request`, the live or
/// last session's brief.
pub fn planner_prompt(run: &Run, epic: &EpicRecord, extract: &str) -> String {
    planner_text(run, epic, extract, false)
}

/// A fresh sub-planner's first turn for an epic that has been planned before
/// (decision 22's re-plan): [`planner_prompt`] with the epic's current tasks.
pub fn replan_prompt(run: &Run, epic: &EpicRecord, extract: &str) -> String {
    planner_text(run, epic, extract, true)
}

/// Where a planner's first turn built with no extract takes one: after its task cap
/// line, on a line of its own (decision 34; the driver fills it).
pub fn planner_extract_at(run: &Run, epic: &EpicRecord, replan: bool) -> usize {
    planner_head(run, epic, replan).join("\n").len()
}

fn planner_text(run: &Run, epic: &EpicRecord, extract: &str, replan: bool) -> String {
    let mut lines = planner_head(run, epic, replan);
    if !extract.is_empty() {
        lines.push(extract.to_string());
    }
    lines.push(String::new());
    if replan {
        lines.push("The epic's current tasks:".into());
        let current: Vec<String> = run
            .tasks
            .iter()
            .filter(|t| t.spec.epic.as_deref() == Some(epic.epic.as_str()))
            .map(|t| {
                let size = size_label(t.size);
                format!("- {} {size} {} {}", t.spec.id, state_label(t), t.spec.title)
            })
            .collect();
        none_or(&mut lines, current);
    }
    lines.push("What to plan:".into());
    lines.push(epic.request.clone());
    lines.join("\n")
}

/// A planner's first turn up to its task cap line.
fn planner_head(run: &Run, epic: &EpicRecord, replan: bool) -> Vec<String> {
    let verb = if replan { "Re-plan" } else { "Plan" };
    let mut lines = vec![
        format!(
            "[anthrex] {verb} epic {} \"{}\" of run {}.",
            epic.epic, epic.title, run.id
        ),
        format!("Run goal: {}", run.goal),
        format!(
            "The run starts from {}; you read the user's checkout at {}, which may differ.",
            base_of(run),
            run.root.display()
        ),
        "Your area (every task you add must own paths only inside it):".into(),
    ];
    bullets(&mut lines, &epic.area);
    lines.push("Tasks already planned that you may depend on:".into());
    let planned: Vec<String> = run
        .tasks
        .iter()
        .filter(|t| t.spec.epic.is_none())
        .map(|t| {
            let owns = t.spec.owns.join(", ");
            format!(
                "- {} {} {} (owns {owns})",
                t.spec.id,
                size_label(t.size),
                t.spec.title
            )
        })
        .collect();
    none_or(&mut lines, planned);
    lines.push(format!(
        "Task cap: at most {} tasks.",
        run.limits.orch.planner_task_cap
    ));
    lines
}

fn none_or(lines: &mut Vec<String>, items: Vec<String>) {
    if items.is_empty() {
        lines.push("- none".into());
    } else {
        lines.extend(items);
    }
}

/// A run scout's first turn (decision 20).
pub fn scout_first_turn(run: &Run, id: &str, area: &[String], question: &str) -> String {
    [
        format!("[anthrex] Scout {id} for run {}.", run.id),
        format!("Area: {}", area.join(", ")),
        format!("Question: {question}"),
        format!(
            "You read the user's checkout at {}; the run starts from {}, so a file the user has not committed may differ from what workers get.",
            run.root.display(),
            base_of(run)
        ),
    ]
    .join("\n")
}

/// A research task's first turn (decision 35).
pub fn research_prompt(run: &Run, task: &Task) -> String {
    let spec = &task.spec;
    let mut lines = vec![
        format!("[anthrex] Research task {}: {}", spec.id, spec.title),
        format!("Run goal: {}", run.goal),
        format!(
            "Answer this from the repository at {}, and the web if you need it. Change nothing.",
            run.root.display()
        ),
        "Your report must cover:".into(),
    ];
    bullets(&mut lines, &spec.acceptance);
    lines.push(String::new());
    lines.push(spec.brief.clone());
    lines.join("\n")
}

/// A review task's first turn (decision 36): level `small` for an S task, else
/// `medium`. The patch follows the acceptance criteria, clamped to M8a's
/// `REVIEW_DIFF_MAX` (16 KiB), and the prompt says when it was cut.
pub fn review_task_prompt(run: &Run, task: &Task, base: &str, head: &str, patch: &str) -> String {
    let _ = run;
    let spec = &task.spec;
    let level = if task.size == proto::Size::S {
        "small"
    } else {
        "medium"
    };
    let (b, h) = (sha7(base), sha7(head));
    let mut lines = vec![
        format!(
            "[anthrex] Review task {} \"{}\" of {}, level {level}.",
            spec.id,
            spec.title,
            spec.review_target.as_deref().unwrap_or("")
        ),
        format!("Base: {b}"),
        format!("Head: {h}"),
        "Nothing will be merged; your findings go to the user.".into(),
    ];
    let clamped = clamp_diff(patch, REVIEW_DIFF_MAX);
    if clamped.len() < patch.len() {
        lines.push(format!(
            "The diff below was cut at 16 KiB; read the rest with git diff {b}..{h}."
        ));
    }
    lines.push("Acceptance criteria:".into());
    bullets(&mut lines, &spec.acceptance);
    lines.push(format!("Diff ({b}..{h}):"));
    lines.push(clamped);
    lines.push(String::new());
    lines.push(spec.brief.clone());
    lines.join("\n")
}

/// The per-epic integration review's first turn (decision 37); round `n > 1` lists the
/// earlier rounds' critical and important findings.
pub fn integration_review_prompt(
    run: &Run,
    epic: &EpicRecord,
    round: u32,
    base: &str,
    head: &str,
) -> String {
    let mut lines = vec![
        format!(
            "[anthrex] Integration review of epic {} \"{}\", round {round}, level frontier.",
            epic.epic, epic.title
        ),
        format!("Run goal: {}", run.goal),
        format!("Area: {}", epic.area.join(", ")),
        format!("Base: {}", sha7(base)),
        format!("Head: {}", sha7(head)),
        "The epic's change is these merges; read each with git diff <merge>^1 <merge>:".into(),
    ];
    for (task_id, merge) in &epic.merges {
        let title = run.task(task_id).map_or("", |t| t.spec.title.as_str());
        lines.push(format!("- {} {task_id} {title}", sha7(merge)));
    }
    lines.push("Judge whether the epic's tasks together do what the epic asked, and whether they fit each other and the code around them.".into());
    let earlier: Vec<String> = run
        .tasks
        .iter()
        .filter(|t| t.orch.integration_of.as_deref() == Some(epic.epic.as_str()))
        .flat_map(|t| t.reviews.iter().flat_map(|r| r.findings.iter()))
        .filter(|f| f.severity != Severity::Minor)
        .map(finding_line)
        .collect();
    if round > 1 && !earlier.is_empty() {
        lines.push("Earlier findings to confirm fixed:".into());
        lines.extend(earlier);
    }
    lines.push(String::new());
    lines.push(epic.brief.clone());
    lines.join("\n")
}

/// Decision 39: the text pasted into an idle orchestrator, clamped to
/// [`WAKE_MAX_BYTES`] by M8a's `messages::clamp` rule (head, [`WAKE_CUT_MARKER`], tail).
pub fn wake_text(run_id: &str, notes: &[String]) -> String {
    let text = format!(
        "[anthrex] Run {run_id} changed: {}. Call run_status for the details.",
        notes.join("; ")
    );
    clamp_with(&text, WAKE_MAX_BYTES, WAKE_CUT_MARKER)
}

/// Decision 26: what `run start --goal` prints on stderr for a planned run.
pub fn planned_message(info: &TriageInfo, id: &str, path: RunPath) -> String {
    [
        format!("triage: {} ({})", triage_kinds(info), triage_source(info)),
        format!(
            "{} path: run {id} is being planned by its orchestrator",
            path_label(path)
        ),
        "talk to it with: anthrex, then C-b T and Enter on the run".into(),
        format!("watch with: anthrex run status {id}"),
    ]
    .join("\n")
}

/// Decision 42b: a message as the worker receives it; the TUI's §12.6 label keys on the
/// prefix.
pub fn message_text(source: &EditSource, kind: MessageKind, text: &str) -> String {
    format!(
        "[anthrex] Message from the {} ({}): {text}",
        sender(source),
        message_kind_label(kind)
    )
}

/// Decision 42d: a task's recorded messages for a fresh session's prompt, oldest first;
/// empty when there are none.
pub fn notes_section(messages: &[TaskMessage]) -> String {
    if messages.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&TaskMessage> = messages.iter().collect();
    sorted.sort_by_key(|m| m.at);
    let mut lines = vec!["Notes from the orchestrator:".to_string()];
    lines.extend(sorted.iter().map(|m| {
        format!(
            "- {} ({}, from {}) {}",
            hh_mm(m.at),
            message_kind_label(m.kind),
            sender(&m.source),
            m.text
        )
    }));
    lines.join("\n")
}

/// Decision 42d: the `change` messages a reviewer is shown; empty when there are none.
pub fn worker_messages_for_review(messages: &[TaskMessage]) -> String {
    let mut changes: Vec<&TaskMessage> = messages
        .iter()
        .filter(|m| m.kind == MessageKind::Change)
        .collect();
    if changes.is_empty() {
        return String::new();
    }
    changes.sort_by_key(|m| m.at);
    let mut lines = vec!["Messages the worker received:".to_string()];
    lines.extend(
        changes
            .iter()
            .map(|m| format!("- {} (change) {}", hh_mm(m.at), m.text)),
    );
    lines.join("\n")
}

/// Decision 42e: a clean refresh merged `n` commits; `list` is `(sha, subject)`, newest
/// first, of which at most 10 are named.
pub fn refresh_clean(n: usize, list: &[(String, String)]) -> String {
    let mut named: Vec<String> = list
        .iter()
        .take(10)
        .map(|(sha, subject)| format!("{} {subject}", sha7(sha)))
        .collect();
    let more = n.saturating_sub(named.len());
    if more > 0 {
        named.push(format!("and {more} more"));
    }
    format!(
        "[anthrex] Your branch now includes the latest merged work ({n} commits: {}). Rebuild before you continue.",
        named.join(", ")
    )
}

/// Decision 42e: a refresh that conflicted.
pub fn refresh_conflict(files: &[String]) -> String {
    format!(
        "[anthrex] Merging the latest run branch into your worktree conflicted in: {}. Resolve them, commit, and continue.",
        files.join(", ")
    )
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "contract_tests_prompts.rs"]
mod tests_prompts;
