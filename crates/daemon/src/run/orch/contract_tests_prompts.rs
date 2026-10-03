//! M9.5: every prompt of Interfaces "Prompts and messages" on a fixed run, the scout
//! extract and the notes sections. Pure.

use proto::{
    DeciderSource, Finding, MessageKind, Route, RunPath, Scale, Severity, TaskKind, TaskState,
    TriageInfo,
};

use super::*;
use crate::run::model::{ReviewRecord, Run};
use crate::run::orch::{EditSource, EpicRecord, PlannerPhase, TaskMessage};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

const BASE: &str = "cccccccccccccccccccccccccccccccccccccccc";
const HEAD: &str = "dddddddddddddddddddddddddddddddddddddddd";

/// A planned run: `api` (no epic, not a hub task), `a1` and `a2` (epic `auth`), `r1` and `v1` for the
/// research and review prompts, and the epic `auth` itself.
fn planned_run() -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml(
                "api",
                "S",
                "[\"crates/core/src/api.rs\", \"crates/core/src/lib.rs\"]",
                "",
            ),
            task_toml("a1", "M", "[\"crates/auth/src/a.rs\"]", "epic = \"auth\""),
            task_toml("a2", "S", "[\"crates/auth/src/b.rs\"]", "epic = \"auth\""),
        ],
    ));
    run.limits.orch.planner_task_cap = 8;
    run.path = Some(RunPath::Plan);
    run.triage = Some(TriageInfo {
        kinds: vec![TaskKind::Code, TaskKind::Docs],
        scale: Scale::Plan,
        path: RunPath::Plan,
        reason: "two modules change".into(),
        source: DeciderSource::Decider,
        fallback_reason: None,
        at: 0,
    });
    let mut epic = EpicRecord::new("auth", PlannerPhase::Planning);
    epic.title = "Auth".into();
    epic.area = vec!["crates/auth/**".into(), "docs/auth.md".into()];
    epic.brief = "Build the auth epic.".into();
    epic.request = "Plan the auth epic.".into();
    epic.merges = vec![("a1".into(), "e".repeat(40)), ("a2".into(), "f".repeat(40))];
    run.orch.epics.push(epic);
    run
}

fn epic(run: &Run) -> &EpicRecord {
    &run.orch.epics[0]
}

/// Milestone 9.3 (task M9.3.7, decision 12, KG §3.3): the `--yes` gate line says that a
/// round or a goal the orchestrator starts itself still stops at the gate.
#[test]
fn orchestrator_first_prompt_is_exact() {
    let mut run = planned_run();
    assert_eq!(
        orchestrator_first_prompt(&run),
        "[anthrex] You are the orchestrator of run add-password-reset-3f9a in /tmp/x.\n\
         Goal: Test goal\n\
         Path: plan (triage: code,docs/plan, decider: two modules change)\n\
         Plan gate: the user approves your submitted plan in the run view\n\
         Start with get_context, then scout, then plan."
    );
    run.path = Some(RunPath::Large);
    run.orch.yes = true;
    let triage = run.triage.as_mut().unwrap();
    triage.kinds = vec![TaskKind::Code];
    triage.scale = Scale::Large;
    triage.source = DeciderSource::Fallback;
    triage.fallback_reason = Some("the decider timed out".into());
    assert_eq!(
        orchestrator_first_prompt(&run),
        "[anthrex] You are the orchestrator of run add-password-reset-3f9a in /tmp/x.\n\
         Goal: Test goal\n\
         Path: large (triage: code/large, fallback: the decider timed out: two modules change)\n\
         Plan gate: off: the run was started with --yes, so your submitted plan starts at once; a round you start with iterate and a goal you start with start_goal still stop at the gate for the user (rules 43 and 45)\n\
         Start with get_context, then scout, then plan."
    );
}

#[test]
fn promoted_first_prompt_is_exact() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"a.rs\"]", "")],
    ));
    run.tasks[0].state = TaskState::Working;
    assert_eq!(
        promoted_first_prompt(&run),
        "[anthrex] You are the orchestrator of run add-password-reset-3f9a in /tmp/x, promoted from the fast path at the user's request.\n\
         Goal: Test goal\n\
         Its one task so far: t1 Title t1 (working). It keeps running.\n\
         Start with get_context and run_status. Plan what else the goal needs; tasks you add wait for the user's approval once you submit them."
    );
}

const EXTRACT: &str = "Scout report s1:\n  Summary one\nFiles: crates/auth/src/a.rs";

#[test]
fn planner_prompt_is_exact() {
    let run = planned_run();
    assert_eq!(
        planner_prompt(&run, epic(&run), EXTRACT),
        "[anthrex] Plan epic auth \"Auth\" of run add-password-reset-3f9a.\n\
         Run goal: Test goal\n\
         The run starts from main@bbbbbbb; you read the user's checkout at /tmp/x, which may differ.\n\
         Your area (every task you add must own paths only inside it):\n\
         - crates/auth/**\n\
         - docs/auth.md\n\
         Tasks already planned that you may depend on:\n\
         - api S Title api (owns crates/core/src/api.rs, crates/core/src/lib.rs)\n\
         Task cap: at most 8 tasks.\n\
         Scout report s1:\n\
         \x20 Summary one\n\
         Files: crates/auth/src/a.rs\n\
         \n\
         What to plan:\n\
         Plan the auth epic."
    );
    // No extract, and no task outside an epic.
    let mut run = planned_run();
    run.tasks.retain(|t| t.spec.epic.is_some());
    let prompt = planner_prompt(&run, epic(&run), "");
    assert!(
        prompt.contains(
            "\nTasks already planned that you may depend on:\n- none\nTask cap: at most 8 tasks.\n\nWhat to plan:\n"
        ),
        "{prompt}"
    );
}

#[test]
fn replan_prompt_is_exact() {
    let mut run = planned_run();
    run.orch.epics[0].request = "Split the login flow.".into();
    run.tasks[2].state = TaskState::Merged;
    assert_eq!(
        replan_prompt(&run, epic(&run), ""),
        "[anthrex] Re-plan epic auth \"Auth\" of run add-password-reset-3f9a.\n\
         Run goal: Test goal\n\
         The run starts from main@bbbbbbb; you read the user's checkout at /tmp/x, which may differ.\n\
         Your area (every task you add must own paths only inside it):\n\
         - crates/auth/**\n\
         - docs/auth.md\n\
         Tasks already planned that you may depend on:\n\
         - api S Title api (owns crates/core/src/api.rs, crates/core/src/lib.rs)\n\
         Task cap: at most 8 tasks.\n\
         \n\
         The epic's current tasks:\n\
         - a1 M pending Title a1\n\
         - a2 S merged Title a2\n\
         What to plan:\n\
         Split the login flow."
    );
}

#[test]
fn scout_first_turn_is_exact() {
    let run = planned_run();
    let area = ["crates/auth/**".to_string(), "docs/**".to_string()];
    assert_eq!(
        scout_first_turn(&run, "s1", &area, "Where is the token checked?"),
        "[anthrex] Scout s1 for run add-password-reset-3f9a.\n\
         Area: crates/auth/**, docs/**\n\
         Question: Where is the token checked?\n\
         You read the user's checkout at /tmp/x; the run starts from main@bbbbbbb, so a file the user has not committed may differ from what workers get."
    );
}

#[test]
fn research_prompt_is_exact() {
    let mut run = planned_run();
    run.tasks[0].spec.kind = TaskKind::Research;
    run.tasks[0].spec.acceptance = vec!["Which callers".into(), "Which tests".into()];
    assert_eq!(
        research_prompt(&run, &run.tasks[0]),
        "[anthrex] Research task api: Title api\n\
         Run goal: Test goal\n\
         Answer this from the repository at /tmp/x, and the web if you need it. Change nothing.\n\
         Your report must cover:\n\
         - Which callers\n\
         - Which tests\n\
         \n\
         Brief api"
    );
}

#[test]
fn review_task_prompt_is_exact() {
    let mut run = planned_run();
    run.tasks[0].spec.kind = TaskKind::Review;
    run.tasks[0].spec.review_target = Some("feature/x".into());
    assert_eq!(
        review_task_prompt(&run, &run.tasks[0], BASE, HEAD, "diff --git a b\n+x"),
        "[anthrex] Review task api \"Title api\" of feature/x, level small.\n\
         Base: ccccccc\n\
         Head: ddddddd\n\
         Nothing will be merged; your findings go to the user.\n\
         Acceptance criteria:\n\
         - Accept api\n\
         Diff (ccccccc..ddddddd):\n\
         diff --git a b\n\
         +x\n\
         \n\
         Brief api"
    );
    // An M task is reviewed at medium; a clamped patch is cut, and the prompt says so.
    run.tasks[0].size = proto::Size::M;
    let patch = "+line\n".repeat(4000);
    let prompt = review_task_prompt(&run, &run.tasks[0], BASE, HEAD, &patch);
    assert!(
        prompt.starts_with("[anthrex] Review task api \"Title api\" of feature/x, level medium.\n"),
        "{prompt}"
    );
    assert!(
        prompt.contains(
            "\nNothing will be merged; your findings go to the user.\nThe diff below was cut at 16 KiB; read the rest with git diff ccccccc..ddddddd.\nAcceptance criteria:\n"
        ),
        "{prompt}"
    );
    assert!(
        prompt.contains(crate::run::contract::DIFF_CUT_MARKER),
        "{prompt}"
    );
    assert!(prompt.ends_with("\n\nBrief api"), "{prompt}");
}

fn finding(severity: Severity, text: &str) -> Finding {
    Finding {
        severity,
        file: Some("crates/auth/src/a.rs".into()),
        line: Some(7),
        input: None,
        text: text.into(),
    }
}

#[test]
fn integration_review_prompt_is_exact() {
    let run = planned_run();
    let head = "[anthrex] Integration review of epic auth \"Auth\", round 1, level frontier.\n\
                Run goal: Test goal\n\
                Area: crates/auth/**, docs/auth.md\n\
                Base: ccccccc\n\
                Head: ddddddd\n\
                The epic's change is these merges; read each with git diff <merge>^1 <merge>:\n\
                - eeeeeee a1 Title a1\n\
                - fffffff a2 Title a2\n\
                Judge whether the epic's tasks together do what the epic asked, and whether they fit each other and the code around them.";
    assert_eq!(
        integration_review_prompt(&run, epic(&run), 1, BASE, HEAD),
        format!("{head}\n\nBuild the auth epic.")
    );
    // Round 2 lists the earlier round's blocking findings.
    let mut run = planned_run();
    let mut review = run.tasks[0].clone();
    review.spec.id = "auth-int1".into();
    review.orch.integration_of = Some("auth".into());
    review.reviews.push(ReviewRecord {
        round: 1,
        route: Route {
            runtime: proto::Runtime::Codex,
            model: "gpt-5".into(),
            strength: proto::Strength::Frontier,
            effort: proto::Effort::High,
        },
        base: BASE.into(),
        head: HEAD.into(),
        verdict: None,
        summary: String::new(),
        findings: vec![
            finding(Severity::Critical, "the token never expires"),
            finding(Severity::Minor, "a name"),
        ],
    });
    run.tasks.push(review);
    let prompt = integration_review_prompt(&run, epic(&run), 2, BASE, HEAD);
    assert!(
        prompt.starts_with(
            "[anthrex] Integration review of epic auth \"Auth\", round 2, level frontier.\n"
        ),
        "{prompt}"
    );
    assert!(
        prompt.ends_with(
            "the code around them.\nEarlier findings to confirm fixed:\n- [critical] crates/auth/src/a.rs:7 the token never expires\n\nBuild the auth epic."
        ),
        "{prompt}"
    );
}

fn message(at: u64, source: EditSource, kind: MessageKind, text: &str) -> TaskMessage {
    TaskMessage {
        at,
        source,
        kind,
        text: text.into(),
        delivered: false,
        outbox: None,
    }
}

/// 13:07 and 13:09 UTC on some day.
fn messages() -> Vec<TaskMessage> {
    vec![
        message(
            86_400 * 3 + 47_340,
            EditSource::User,
            MessageKind::Info,
            "the CI is slow today",
        ),
        message(
            86_400 * 3 + 47_220,
            EditSource::Orchestrator,
            MessageKind::Change,
            "use the v2 token API",
        ),
    ]
}

#[test]
fn notes_section_is_exact() {
    assert_eq!(notes_section(&[]), "");
    assert_eq!(
        notes_section(&messages()),
        "Notes from the orchestrator:\n\
         - 13:07 (change, from orchestrator) use the v2 token API\n\
         - 13:09 (info, from user) the CI is slow today"
    );
}

#[test]
fn worker_messages_for_review_is_exact() {
    assert_eq!(worker_messages_for_review(&[]), "");
    let info_only = &messages()[..1];
    assert_eq!(worker_messages_for_review(info_only), "");
    assert_eq!(
        worker_messages_for_review(&messages()),
        "Messages the worker received:\n- 13:07 (change) use the v2 token API"
    );
}
