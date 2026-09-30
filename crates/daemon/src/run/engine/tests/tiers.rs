//! Milestone 9.1 task M9.1.13: tier 1 in place of the check for a tiered profile
//! (decisions 14, 15), tier 2 in the merge candidate (decision 16), what a tier job's
//! outcome does to the run (decisions 9, 11, 21), a lost tier-1 op (decision 29) and
//! the tier-0 block of the worker prompt (decision 13).

use std::collections::BTreeSet;
use std::path::PathBuf;

use proto::{DeciderSource, TaskState};

use super::control::resume;
use super::control_restore::restart;
use super::dispatch::task_path;
use super::fixture::*;
use super::gates::{accepted, only_op, tdd_args};
use super::holds::delivers;
use super::merge::{config, pending_one};
use super::turns_fixes::assert_alive;
use crate::run::contract::{candidate_red_message, check_failed_message, worker_prompt};
use crate::run::engine::{Effect, OpKind, OpResult, ScratchAt};
use crate::run::model::{CheckRecord, OpId, TierRecord};
use crate::run::slots::Priority;
use crate::run::tiers::{Affected, CacheCtx, Scope, StepKind, StepOutcome, TierOutcome, TierSpec};

/// `PROFILE` with tier keys: a build check, a per-module test command that leaves slow
/// tests out, a command graph and directory names.
pub(super) fn tiered() -> String {
    PROFILE.replace(
        "check = \"cargo test\"\n",
        "check = \"cargo test\"\nbuild_check = \"cargo build\"\nmodule_test = \"cargo test -p {module} {filter:-E %}\"\nmodule_graph = \"sh graph.sh\"\nmodule_names = \"dir\"\nslow_tests = \"test(e2e)\"\n",
    )
}

const REPO_DIR: &str = "/tmp/data/repos/r-1";
/// Another stage head, so decision 15's range is seen to follow it.
const MOVED: &str = "4444444444444444444444444444444444444444";

/// A working tdd `t1` owning `crates/a/**` on the tiered profile, its run with a
/// stored profile's `manifests` and a repository data directory; its window.
fn working() -> (Fixture, u32) {
    let plan = plan_with(&tiered(), &[task("t1", "S", "a", "")]);
    let mut fx = Fixture::with_config(&plan, config());
    fx.start_with(true, |run| {
        run.profile.manifests = vec!["deps/*.txt".into()];
        run.profile_hash = crate::run::tiers::profile_hash(&run.profile);
        run.repo_dir = PathBuf::from(REPO_DIR);
    });
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let window = fx.launch_all()[0].1;
    (fx, window)
}

/// `t1`'s accepted tdd claim and passing proof: the effects of the proof's result.
fn proved(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    let effects = accepted(fx, window, tdd_args());
    let (op, _) = only_op(&effects, "Proof");
    fx.done(
        op,
        OpResult::Proof {
            red_failed: true,
            head_passed: true,
            matched: true,
            red_tail: String::new(),
            head_tail: String::new(),
        },
    )
}

fn step(kind: StepKind, command: &str, ok: bool, cached: bool) -> StepOutcome {
    StepOutcome {
        kind,
        command: command.into(),
        ok,
        code: Some(if ok { 0 } else { 101 }),
        timed_out: false,
        secs: if cached { 0 } else { 5 },
        cached,
        retried: !ok,
        failing: if ok { vec![] } else { vec!["a::works".into()] },
        flaky: vec![],
        granted: if cached { 0 } else { 2 },
    }
}

const TESTS: &str = "cargo test -p 'a' -E 'not (test(e2e))'";

/// A tier outcome on module `a`: a build step and a tests step.
fn outcome(tier: u8, ok: bool, cached: bool) -> TierOutcome {
    let steps = vec![
        step(StepKind::Build, "cargo build", true, cached),
        step(StepKind::Tests, TESTS, ok, cached),
    ];
    TierOutcome {
        tier,
        scope: Scope::Gate,
        affected: Affected::Modules(BTreeSet::from(["a".to_string()])),
        tree: "7".repeat(40),
        secs: steps.iter().map(|s| s.secs).sum(),
        steps,
        ok,
        tail: if ok {
            String::new()
        } else {
            "test a::works ... FAILED".into()
        },
        toolchain: None,
        graph_note: None,
    }
}

fn the_tier(kind: &OpKind) -> TierSpec {
    match kind {
        OpKind::Tier(spec) => (**spec).clone(),
        OpKind::MergeCandidate {
            tier: Some(spec), ..
        } => (**spec).clone(),
        other => panic!("no tier job in {other:?}"),
    }
}

#[test]
fn tier1_is_a_tier_op_on_the_task_proof_dir_with_the_affected_range() {
    let (mut fx, window) = working();
    // The task's stage head moved since it started: tier 1 measures from where it is.
    crate::run::engine::stages::set_stage_head(fx.run_mut(), 1, MOVED);
    let effects = proved(&mut fx, window);
    assert_eq!(fx.task("t1").state, TaskState::Check);
    assert!(ops_in(&effects, "Check").is_empty(), "{effects:#?}");
    let (op, kind) = only_op(&effects, "Tier");
    let proof = task_path("t1.proof");
    let spec = the_tier(&kind);
    let run = fx.run();
    let expected = TierSpec {
        tier: 1,
        stage: 1,
        root: "/tmp/x".into(),
        dir: proof.clone(),
        scratch: Some(ScratchAt {
            root: "/tmp/x".into(),
            commit: HEAD.into(),
            setup: Some("make deps".into()),
        }),
        diff_base: MOVED.into(),
        head: HEAD.into(),
        profile: run.profile.tiers.clone(),
        check: Some("cargo test".into()),
        hub: vec!["crates/proto/**".into()],
        source: vec!["crates/*/src/**".into()],
        modules: vec!["crates/*".into()],
        manifests: vec!["deps/*.txt".into()],
        single_test: Some("cargo test -- --exact {test}".into()),
        timeout_secs: 1800,
        env: vec![("TARGET".into(), format!("{}/target", proof.display()))],
        priority: Priority::Gate,
        critical: true,
        cache: Some(CacheCtx {
            profile_hash: run.profile_hash.clone(),
            toolchain: "none".into(),
        }),
        toolchain: None,
        repo_dir: REPO_DIR.into(),
    };
    assert_eq!(spec, expected);
    assert_eq!(fx.task("t1").gate_op, Some(op));
    assert!(run.profile.tiers.is_tiered());
    assert_eq!(run.profile_hash.len(), 16);
    // Nothing more while it is in flight.
    let effects = fx.tick();
    assert!(ops_in(&effects, "Tier").is_empty(), "{effects:#?}");

    // A green tier 1 passes the gate and records the job.
    let effects = fx.done(op, OpResult::Tier(Box::new(outcome(1, true, false))));
    assert_ne!(fx.task("t1").state, TaskState::Check);
    assert_eq!(ops_in(&effects, "MergeCandidate").len(), 1, "{effects:#?}");
    let texts: Vec<&str> = fx
        .task("t1")
        .history
        .iter()
        .map(|e| e.text.as_str())
        .collect();
    assert!(texts.contains(&"tier 1: 1 modules (a)"), "{texts:?}");
    let record = fx.task("t1").checks.last().cloned().unwrap();
    assert!(record.ok && !record.on_candidate);
    assert_eq!(
        record.tier.map(|t| (t.tier, t.steps, t.cached)),
        Some((1, 2, 0))
    );
    assert_alive(&fx);
}

#[test]
fn tier1_red_bounces_like_check_red() {
    let (mut fx, window) = working();
    let effects = proved(&mut fx, window);
    let (op, _) = only_op(&effects, "Tier");
    let effects = fx.done(op, OpResult::Tier(Box::new(outcome(1, false, false))));
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.rung, t1.failures, t1.bounces.check),
        (TaskState::Working, 1, 1, 1)
    );
    let record = CheckRecord {
        at: fx.now,
        ok: false,
        code: Some(101),
        timed_out: false,
        tail: "test a::works ... FAILED".into(),
        secs: 10,
        on_candidate: false,
        // The fixture's deciders are off, so the summary is the fallback (M8b.12).
        summary: None,
        summary_source: Some(DeciderSource::Fallback),
        tier: Some(TierRecord {
            tier: 1,
            affected: "1 modules (a)".into(),
            steps: 2,
            cached: 0,
            ok: false,
            secs: 10,
            flaky: vec![],
            failing: vec!["a::works".into()],
            at: fx.now,
        }),
    };
    assert_eq!(t1.checks, vec![record.clone()]);
    let text = check_failed_message(TESTS, &record);
    assert_eq!(
        text,
        format!(
            "[anthrex] The check failed (exit 101): {TESTS}\ntier 1: 1 modules (a)\nLast 40 lines:\ntest a::works ... FAILED\nFix it, commit, then call task_done again."
        )
    );
    assert_eq!(delivers(&effects), vec![text]);
    assert_alive(&fx);
}

#[test]
fn tier2_is_skipped_when_every_step_is_cached() {
    let (mut fx, window) = working();
    let effects = proved(&mut fx, window);
    let (op, _) = only_op(&effects, "Tier");
    let effects = fx.done(op, OpResult::Tier(Box::new(outcome(1, true, false))));
    let (op, kind) = only_op(&effects, "MergeCandidate");
    let OpKind::MergeCandidate {
        check,
        expected_run_head,
        ..
    } = &kind
    else {
        unreachable!()
    };
    // Decision 16: tier 2 runs where M8a's check did.
    assert_eq!(check, &None);
    let spec = the_tier(&kind);
    let int = task_path("integration");
    assert_eq!(
        (spec.tier, spec.dir.clone(), spec.scratch.clone()),
        (2, int.clone(), None)
    );
    assert_eq!(spec.diff_base, *expected_run_head);
    assert_eq!(spec.priority, Priority::Candidate);
    assert_eq!(spec.manifests, vec!["deps/*.txt".to_string()]);
    assert_eq!(
        spec.env,
        vec![("TARGET".into(), format!("{}/target", int.display()))]
    );

    let cached = outcome(2, true, true);
    fx.done(
        op,
        OpResult::Merged {
            commit: "1".repeat(40),
            tier: Some(Box::new(cached)),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Merged);
    let texts: Vec<&str> = t1.history.iter().map(|e| e.text.as_str()).collect();
    assert!(texts.contains(&"tier 2: cached (2 steps)"), "{texts:?}");
    let record = t1.checks.last().cloned().unwrap();
    assert!(record.ok && record.on_candidate);
    let tier = record.tier.expect("the tier-2 record");
    assert_eq!(
        (tier.tier, tier.steps, tier.cached, tier.secs),
        (2, 2, 2, 0)
    );
}

#[test]
fn tier2_red_is_candidate_red_with_failing_names() {
    let (mut fx, window) = working();
    let effects = proved(&mut fx, window);
    let (op, _) = only_op(&effects, "Tier");
    fx.done(op, OpResult::Tier(Box::new(outcome(1, true, false))));
    let (op, _) = pending_one(&fx, "MergeCandidate", Some("t1"));
    let red = outcome(2, false, false);
    let effects = fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(101),
            timed_out: false,
            tail: red.tail.clone(),
            secs: red.secs,
            tier: Some(Box::new(red)),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!((t1.rung, t1.failures, t1.bounces.merge), (1, 1, 1));
    let record = t1.checks.last().cloned().unwrap();
    assert!(!record.ok && record.on_candidate);
    let tier = record.tier.clone().expect("the tier-2 record");
    assert_eq!(tier.failing, vec!["a::works".to_string()]);
    assert_eq!(tier.affected, "1 modules (a)");
    let text = candidate_red_message(TESTS, &record);
    assert!(
        text.contains(&format!("(exit 101): {TESTS}\ntier 2: 1 modules (a)\n")),
        "{text}"
    );
    assert_eq!(delivers(&effects), vec![text]);
    assert!(fx.run().merge_queue.is_empty());
    assert_alive(&fx);
}

#[test]
fn tier_outcomes_keep_the_toolchain_and_note_an_unknown_graph_once() {
    let (mut fx, window) = working();
    let effects = proved(&mut fx, window);
    let (op, _) = only_op(&effects, "Tier");
    let mut first = outcome(1, true, false);
    first.toolchain = Some("0123456789abcdef".into());
    first.graph_note = Some("module_graph is none".into());
    fx.done(op, OpResult::Tier(Box::new(first)));
    let run = fx.run();
    assert_eq!(run.toolchain.as_deref(), Some("0123456789abcdef"));
    let note = "module graph unknown: module_graph is none; every tier runs check";
    assert_eq!(run.graph_note.as_deref(), Some(note));
    // The next job carries the toolchain and keys the cache with it.
    let (op, kind) = pending_one(&fx, "MergeCandidate", Some("t1"));
    let spec = the_tier(&kind);
    assert_eq!(spec.toolchain.as_deref(), Some("0123456789abcdef"));
    assert_eq!(
        spec.cache.map(|c| c.toolchain),
        Some("0123456789abcdef".to_string())
    );
    let mut second = outcome(2, true, false);
    second.graph_note = Some("module_graph is none".into());
    fx.done(
        op,
        OpResult::Merged {
            commit: "1".repeat(40),
            tier: Some(Box::new(second)),
        },
    );
    let noted = fx.run().log.iter().filter(|e| e.text == note).count();
    assert_eq!(noted, 1);

    // An "unknown" toolchain turns the cache off for every later job.
    let (mut fx, window) = working();
    fx.run_mut().toolchain = Some("unknown".into());
    let effects = proved(&mut fx, window);
    let (_, kind) = only_op(&effects, "Tier");
    assert_eq!(the_tier(&kind).cache, None);
}

#[test]
fn a_lost_tier1_is_issued_again_after_the_restart() {
    let (mut fx, window) = working();
    let effects = proved(&mut fx, window);
    let (lost, _): (OpId, _) = only_op(&effects, "Tier");
    restart(&mut fx, Vec::new());
    assert_eq!(fx.task("t1").gate_op, None);
    let effects = resume(&mut fx);
    let (op, kind) = only_op(&effects, "Tier");
    assert_ne!(op, lost);
    assert_eq!(the_tier(&kind).tier, 1);
    assert_eq!(fx.task("t1").gate_op, Some(op));
}

#[test]
fn tier0_block_is_in_the_worker_prompt_only_when_tiered() {
    let (fx, _) = working();
    let prompt = worker_prompt(fx.run(), fx.task("t1"), "", "");
    let expected = "Check command: cargo test\nModule test command: cargo test -p {module} -E 'not (test(e2e))'\nYour modules: a (put one in place of {module})\nWhile you work, run your own test and these module tests only; after task_done the engine runs the wider suites.\n\nThis task owns:";
    assert!(prompt.contains(expected), "{prompt}");

    // `module_tests` alone: the task's modules are filled in.
    let profile = tiered().replace(
        "module_test = \"cargo test -p {module} {filter:-E %}\"",
        "module_tests = \"cargo test {modules:-p %} {filter:-E %}\"",
    );
    let fx = Fixture::with_config(&plan_with(&profile, &[task("t1", "S", "a", "")]), config());
    let run = build(&fx.plan, &fx.config, true);
    let prompt = worker_prompt(&run, &run.tasks[0], "", "");
    assert!(
        prompt.contains("\nModule test command: cargo test -p 'a' -E 'not (test(e2e))'\nYour modules: a (put one in place of {module})\n"),
        "{prompt}"
    );

    // An untiered profile's prompt has no tier-0 line.
    let run = build(
        &plan_with(PROFILE, &[task("t1", "S", "a", "")]),
        &config(),
        true,
    );
    let prompt = worker_prompt(&run, &run.tasks[0], "", "");
    assert!(
        prompt.contains("Check command: cargo test\n\nThis task owns:"),
        "{prompt}"
    );
    assert!(!prompt.contains("Module test command"), "{prompt}");
}
