//! Milestone 9 task M9.9: research and review tasks (decisions 35 and 36). A research
//! task runs a read-only scout session in a reader slot and ends `reported` with its
//! report; a review task resolves its target, runs a reviewer on it, and ends
//! `reported` whatever the verdict. Integration reviews are `kinds_integration.rs`.

use proto::{AgentRole, BlockReason, TaskState};
use serde_json::{Value, json};

use super::dispatch::replies;
use super::fixture::*;
use super::merge::pending_one;
use crate::run::engine::schedule::{readers_busy, writers_busy};
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::orch::contract::{research_prompt, review_task_prompt};
use crate::scout::contract::SCOUT_NUDGE;
use crate::scout::service::REPORT_ACCEPTED;

pub(super) const B1: &str = "b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1";
pub(super) const H1: &str = "e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1e1";

/// A research task with no files.
pub(super) fn research(id: &str, extra: &str) -> String {
    task_toml(id, "S", "[]", &format!("kind = \"research\"\n{extra}"))
}

/// A review task of `main..feature`.
pub(super) fn review(id: &str, size: &str) -> String {
    task_toml(
        id,
        size,
        "[]",
        "kind = \"review\"\nreview_target = \"main..feature\"",
    )
}

/// A running run (`--yes`) of `tasks`, with `limits` spliced into the plan.
pub(super) fn running(limits: &str, tasks: &[String]) -> Fixture {
    let mut fx = Fixture::new(&plan_with(&profile_with(limits), tasks));
    fx.ready(true);
    fx
}

pub(super) fn report_args() -> Value {
    json!({"summary": "The hooks live in crates/daemon/src/hooks.rs.",
           "files": [{"path": "crates/daemon/src/hooks.rs", "why": "the hook table"}],
           "risks": ["the table is order-sensitive"]})
}

/// Research task `id`'s window, once its session was launched.
pub(super) fn research_window(fx: &mut Fixture, id: &str) -> u32 {
    let windows = fx.complete_windows();
    windows
        .into_iter()
        .find(|(t, _)| t == id)
        .map(|(_, w)| w)
        .unwrap_or_else(|| panic!("no window for {id}"))
}

pub(super) fn submit_report(fx: &mut Fixture, window: u32, id: &str, args: Value) -> Vec<Effect> {
    fx.tool_as(AgentRole::Scout, window, id, "submit_scout_report", args)
}

/// Review task `id` resolved to `B1..H1`, its review prepared, and its reviewer's
/// window made; returns the window.
pub(super) fn reviewer_window(fx: &mut Fixture, id: &str, patch: &str) -> u32 {
    let (op, _) = pending_one(fx, "ResolveTarget", Some(id));
    fx.done(
        op,
        OpResult::Target {
            base: B1.into(),
            head: H1.into(),
        },
    );
    let (op, _) = pending_one(fx, "PrepareReview", Some(id));
    fx.done(
        op,
        OpResult::Review {
            base: B1.into(),
            head: H1.into(),
            patch: patch.into(),
        },
    );
    let windows = fx.complete_windows();
    windows
        .into_iter()
        .find(|(t, _)| t == id)
        .map(|(_, w)| w)
        .unwrap_or_else(|| panic!("no reviewer window for {id}"))
}

pub(super) fn verdict(fx: &mut Fixture, window: u32, id: &str, args: Value) -> Vec<Effect> {
    fx.tool_as(AgentRole::Reviewer, window, id, "submit_review", args)
}

pub(super) fn changes() -> Value {
    json!({"verdict": "changes", "summary": "two problems", "findings": [
        {"severity": "critical", "file": "src/a.rs", "line": 3, "text": "drops the error"},
        {"severity": "important", "input": "an empty list", "text": "no test for the empty case"},
        {"severity": "minor", "text": "a typo in a comment"}]})
}

pub(super) fn approve() -> Value {
    json!({"verdict": "approve", "summary": "fine", "findings": [
        {"severity": "minor", "text": "could be shorter"}]})
}

/// The task a `CreateWindow` is for, by its session's run reference (a research
/// session works in the checkout, so `op_task`'s worktree name does not tell).
pub(super) fn window_task(kind: &OpKind) -> String {
    match kind {
        OpKind::CreateWindow { spec, .. } => spec
            .run_ref
            .as_ref()
            .and_then(|r| r.task_id.clone())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn create_window(fx: &Fixture, id: &str) -> OpKind {
    let ops: Vec<OpKind> = fx
        .ops("CreateWindow")
        .into_iter()
        .map(|(_, k)| k)
        .filter(|k| window_task(k) == id)
        .collect();
    ops.last()
        .cloned()
        .unwrap_or_else(|| panic!("no CreateWindow for {id}"))
}

#[test]
fn research_task_takes_a_reader_slot_and_no_worktree() {
    // One reader slot: r1 takes it, r2 waits; no writer slot is taken.
    let mut fx = running("max_readers = 1", &[research("r1", ""), research("r2", "")]);
    fx.tick();
    assert!(
        tasks_of(&fx.log, "PrepareWorktree").is_empty(),
        "{:#?}",
        fx.log
    );
    let OpKind::CreateWindow {
        name,
        spec,
        first_turn,
        worktree,
        extract,
        ..
    } = create_window(&fx, "r1")
    else {
        unreachable!()
    };
    assert_eq!(name, format!("{H4}/r1.s1"));
    assert_eq!(worktree, fx.run().root);
    assert_eq!(extract, None);
    assert_eq!(first_turn, research_prompt(fx.run(), fx.task("r1")));
    let mcp = spec
        .mcp
        .clone()
        .expect("a research session has anthrex tools");
    assert_eq!(mcp.role, AgentRole::Scout);
    assert_eq!(mcp.task_id.as_deref(), Some("r1"));
    assert_eq!(mcp.scout_id, None);
    let run_ref = spec.run_ref.clone().unwrap();
    assert_eq!((run_ref.role, run_ref.session), (AgentRole::Scout, 1));
    assert_eq!(spec.cwd, fx.run().root);
    assert_eq!(fx.task("r1").state, TaskState::Working);
    assert_eq!(fx.task("r1").start_commit, None);
    assert!(!fx.task("r1").worktree_live);
    assert_eq!(fx.task("r2").state, TaskState::Queued);
    assert_eq!(readers_busy(fx.run()), 1);
    assert_eq!(writers_busy(fx.run()), 0);
    assert!(create_window_count(&fx, "r2") == 0);
    // The window comes: still one reader, still no writer.
    research_window(&mut fx, "r1");
    fx.tick();
    assert_eq!(readers_busy(fx.run()), 1);
    assert_eq!(writers_busy(fx.run()), 0);
    assert_eq!(create_window_count(&fx, "r2"), 0);
}

fn create_window_count(fx: &Fixture, id: &str) -> usize {
    fx.ops("CreateWindow")
        .iter()
        .filter(|(_, k)| window_task(k) == id)
        .count()
}

#[test]
fn research_report_marks_the_task_reported() {
    let mut fx = running("max_readers = 1", &[research("r1", ""), research("r2", "")]);
    fx.tick();
    let window = research_window(&mut fx, "r1");
    // Bad arguments are refused and change nothing.
    let effects = submit_report(&mut fx, window, "r1", json!({"summary": ""}));
    let answers = replies(&effects);
    assert!(
        matches!(&answers[..], [Err(e)] if e.starts_with("invalid arguments: ")),
        "{answers:?}"
    );
    assert_eq!(fx.task("r1").state, TaskState::Working);
    // Another window is not r1's session.
    let effects = submit_report(&mut fx, window + 50, "r1", report_args());
    assert_eq!(
        replies(&effects),
        vec![Err(
            "this window is not the research session of task r1".to_string()
        )]
    );
    let effects = submit_report(&mut fx, window, "r1", report_args());
    assert_eq!(replies(&effects), vec![Ok(REPORT_ACCEPTED.to_string())]);
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Reported);
    let report = task.orch.research.clone().expect("the report is kept");
    assert_eq!(
        report.summary,
        "The hooks live in crates/daemon/src/hooks.rs."
    );
    assert_eq!(report.files[0].path, "crates/daemon/src/hooks.rs");
    assert!(
        effects.contains(&Effect::RetireWindow { window_id: window }),
        "{effects:#?}"
    );
    // The slot frees for r2.
    assert_eq!(create_window_count(&fx, "r2"), 1);
    // A second report from the retired session is refused.
    let effects = submit_report(&mut fx, window, "r1", report_args());
    assert!(matches!(&replies(&effects)[..], [Err(_)]), "{effects:#?}");
    assert_eq!(fx.task("r1").state, TaskState::Reported);
}

#[test]
fn research_without_report_is_nudged_then_blocked() {
    let mut fx = running("", &[research("r1", "")]);
    fx.tick();
    let window = research_window(&mut fx, "r1");
    let effects = fx.turn_completed(window);
    let delivered: Vec<&Effect> = effects
        .iter()
        .filter(|e| matches!(e, Effect::Deliver { window_id, text, .. } if *window_id == window && text.contains(SCOUT_NUDGE)))
        .collect();
    assert_eq!(delivered.len(), 1, "{effects:#?}");
    assert_eq!(fx.task("r1").state, TaskState::Working);
    let effects = fx.turn_completed(window);
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Blocked);
    let block = task.block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert_eq!(
        block.text,
        "the research task ended two turns without a report"
    );
    assert!(
        effects.contains(&Effect::KillWindow { window_id: window }),
        "{effects:#?}"
    );
    assert_eq!(readers_busy(fx.run()), 0);
}

#[test]
fn dependent_of_a_reported_task_runs() {
    let mut fx = running(
        "",
        &[
            research("r1", ""),
            task("t2", "S", "auth", "deps = [\"r1\"]"),
        ],
    );
    fx.tick();
    assert_eq!(fx.task("t2").state, TaskState::Pending);
    assert!(tasks_of(&fx.log, "PrepareWorktree").is_empty());
    let window = research_window(&mut fx, "r1");
    submit_report(&mut fx, window, "r1", report_args());
    fx.tick();
    assert_ne!(fx.task("t2").state, TaskState::Pending);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t2".to_string()]);
}

#[test]
fn research_takes_a_reader_slot_after_run_scouts() {
    use crate::run::orch::{RunScout, RunScoutState};
    let text = plan_with(&profile_with("max_readers = 1"), &[research("r1", "")]);
    let mut fx = Fixture::new(&text);
    fx.start(true);
    // A run scout asked for before the first pass: it takes the one slot first.
    let now = fx.now;
    fx.run_mut().orch.run_scouts.push(RunScout {
        id: format!("{H4}-api"),
        question: "Where is the API?".into(),
        area: vec!["crates/api/**".into()],
        web: false,
        state: RunScoutState::Queued,
        queued_at: now,
        started_at: None,
        ended_at: None,
        window_id: None,
    });
    let (op, _) = fx.op("CreateRunBranch");
    let effects = fx.done(op, OpResult::Worktree { head: BASE.into() });
    assert_eq!(ops_in(&effects, "StartScout").len(), 1, "{effects:#?}");
    assert_eq!(create_window_count(&fx, "r1"), 0);
    let (op, _) = fx.op("StartScout");
    fx.done(op, OpResult::ScoutStarted { window_id: 77 });
    fx.tick();
    assert_eq!(create_window_count(&fx, "r1"), 0);
    let effects = fx.next(crate::run::engine::EventKind::Orch(
        crate::run::engine::OrchEvent::ScoutEnded {
            run_id: RUN_ID.into(),
            scout_id: format!("{H4}-api"),
            outcome: crate::run::engine::ScoutEnd::Reported,
            usage: Default::default(),
        },
    ));
    let windows: Vec<String> = ops_in(&effects, "CreateWindow")
        .iter()
        .map(|(_, k)| window_task(k))
        .collect();
    assert_eq!(windows, vec!["r1".to_string()]);
}

#[test]
fn review_task_resolves_its_target_first() {
    let mut fx = running("", &[review("v1", "M")]);
    let effects = fx.tick();
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    let (op, kind) = pending_one(&fx, "ResolveTarget", Some("v1"));
    assert_eq!(
        kind,
        OpKind::ResolveTarget {
            root: fx.run().root.clone(),
            target: "main..feature".into(),
            base_branch: "main".into(),
        }
    );
    assert_eq!(fx.task("v1").state, TaskState::Review);
    assert_eq!(readers_busy(fx.run()), 1);
    assert_eq!(writers_busy(fx.run()), 0);
    let effects = fx.done(
        op,
        OpResult::Target {
            base: B1.into(),
            head: H1.into(),
        },
    );
    assert_eq!(
        fx.task("v1").orch.review_range,
        Some((B1.to_string(), H1.to_string()))
    );
    let prepares = ops_in(&effects, "PrepareReview");
    assert_eq!(prepares.len(), 1, "{effects:#?}");
    let OpKind::PrepareReview {
        head_ref,
        base_ref,
        path,
        ..
    } = &prepares[0].1
    else {
        unreachable!()
    };
    assert_eq!((head_ref.as_str(), base_ref.as_str()), (H1, B1));
    assert_eq!(path, &fx.run().review_path("v1"));
    assert_eq!(readers_busy(fx.run()), 1);
    let (op, _) = pending_one(&fx, "PrepareReview", Some("v1"));
    let patch = "diff --git a/x b/x\n+one\n";
    let effects = fx.done(
        op,
        OpResult::Review {
            base: B1.into(),
            head: H1.into(),
            patch: patch.into(),
        },
    );
    let windows = ops_in(&effects, "CreateWindow");
    assert_eq!(windows.len(), 1, "{effects:#?}");
    let OpKind::CreateWindow {
        spec, first_turn, ..
    } = &windows[0].1
    else {
        unreachable!()
    };
    let task = fx.task("v1");
    assert_eq!(
        first_turn,
        &review_task_prompt(fx.run(), task, B1, H1, patch)
    );
    let mcp = spec.mcp.clone().unwrap();
    assert_eq!(
        (mcp.role, mcp.task_id.as_deref()),
        (AgentRole::Reviewer, Some("v1"))
    );
    // The reviewer runs on the task's own route.
    assert_eq!(task.reviews.last().unwrap().route, task.route);
}

#[test]
fn unresolvable_target_blocks_environment_with_the_text() {
    let mut fx = running("", &[review("v1", "M")]);
    fx.tick();
    let (op, _) = pending_one(&fx, "ResolveTarget", Some("v1"));
    fx.done(
        op,
        OpResult::Failed {
            message: "feature: fatal: bad revision 'feature'".into(),
        },
    );
    let task = fx.task("v1");
    assert_eq!(task.state, TaskState::Blocked);
    let block = task.block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert_eq!(
        block.text,
        "review target main..feature does not resolve: feature: fatal: bad revision 'feature'"
    );
    assert_eq!(readers_busy(fx.run()), 0);
    assert!(fx.ops("PrepareReview").is_empty());
}

#[test]
fn review_verdict_reports_whatever_it_is() {
    for (args, verdict) in [
        (changes(), proto::Verdict::Changes),
        (approve(), proto::Verdict::Approve),
    ] {
        let mut fx = running("", &[review("v1", "S")]);
        fx.tick();
        let window = reviewer_window(&mut fx, "v1", "+x\n");
        let effects = verdict_of(&mut fx, window, args);
        assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
        let task = fx.task("v1");
        assert_eq!(task.state, TaskState::Reported, "{verdict:?}");
        assert_eq!(task.reviews.last().unwrap().verdict, Some(verdict));
        // Nothing is sent back and nothing is merged.
        assert_eq!(task.failures, 0);
        assert!(fx.run().merge_queue.is_empty());
        assert!(fx.ops("MergeCandidate").is_empty());
        assert!(
            effects.contains(&Effect::RetireWindow { window_id: window }),
            "{effects:#?}"
        );
        assert_eq!(readers_busy(fx.run()), 0);
    }
}

fn verdict_of(fx: &mut Fixture, window: u32, args: Value) -> Vec<Effect> {
    verdict(fx, window, "v1", args)
}

#[test]
fn review_findings_go_to_the_report() {
    let mut fx = running("", &[review("v1", "M"), research("r1", "")]);
    fx.tick();
    let window = research_window(&mut fx, "r1");
    submit_report(&mut fx, window, "r1", report_args());
    let window = reviewer_window(&mut fx, "v1", "+x\n");
    verdict_of(&mut fx, window, changes());
    let text = crate::run::report::render(fx.run(), fx.now);
    let findings = text
        .find("## Review findings")
        .unwrap_or_else(|| panic!("{text}"));
    let research = text.find("## Research").unwrap_or_else(|| panic!("{text}"));
    let log = text.find("## Log").unwrap();
    assert!(findings < log && research < log, "{text}");
    let section = &text[findings..log];
    for line in [
        "### v1: Title v1 (main..feature)",
        "Verdict: changes. two problems",
        "- critical src/a.rs:3: drops the error",
        "- important input an empty list: no test for the empty case",
        "- minor: a typo in a comment",
    ] {
        assert!(section.contains(line), "{line:?} in {section}");
    }
    let section = &text[research..];
    for line in [
        "### r1: Title r1",
        "The hooks live in crates/daemon/src/hooks.rs.",
        "- crates/daemon/src/hooks.rs: the hook table",
        "Risks:",
        "- the table is order-sensitive",
    ] {
        assert!(section.contains(line), "{line:?} in {section}");
    }
    // `research.md` holds the same research section.
    let research_md = crate::run::report::research_text(fx.run()).unwrap();
    assert!(
        research_md.starts_with("# Research of run "),
        "{research_md}"
    );
    assert!(research_md.contains("### r1: Title r1"), "{research_md}");
    // A run with no research has none.
    let fx = running("", &[review("v2", "M")]);
    assert_eq!(crate::run::report::research_text(fx.run()), None);
}
