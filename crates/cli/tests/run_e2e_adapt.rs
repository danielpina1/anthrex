//! Milestone 8b, task 18 (I): `anthrex run start --goal` end to end, through a real
//! daemon with `fake-agent` as both runtimes and as the decider (`ANTHREX_DECIDER_BIN`):
//! the fast path, the goals it refuses without creating anything, the plan path without
//! deciders, the missing profile, M8a's start checks and `run promote`. The engine's
//! deciders, the output filter and metering are in `run_e2e_adapt_engine.rs`.

mod support;

use std::process::Output;

use daemon::run::triage::refused_message;
use proto::{
    DaemonMsg, DeciderSource, ProposalOrigin, ProposalState, RunPath, RunReply, RunState, Scale,
    TaskKind, TaskState, TriageInfo,
};
use serde_json::{Value, json};
use support::run_adapt::{ADAPT_FILES, PROFILE_LINES, PROFILE_WAIT, STORED_PROFILE, triage_single};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_plans::*;

/// A harness running deciders in `mode`, with the stored profile (`profile` extra lines)
/// and the base files it needs.
fn harness(mode: &str, profile: &str, env: &[(&str, &str)], files: &[(&str, &str)]) -> RunHarness {
    let mut all: Vec<(&str, &str)> = ADAPT_FILES.to_vec();
    all.extend_from_slice(files);
    let h = RunHarness::adapt(mode, "", env, &all);
    h.stored_profile(&format!("{STORED_PROFILE}{profile}"));
    h
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `run start --goal` started a fast-path run: its id.
fn started(h: &RunHarness, out: &Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}\n{}",
        out.status.code(),
        stdout(out),
        stderr(out),
        h.log_tail()
    );
    let id = stdout(out).trim().to_string();
    assert!(!id.is_empty() && !id.contains('\n'), "{:?}", stdout(out));
    id
}

/// `run start --goal` failed (exit 1) with exactly `message`, and created nothing: no
/// run branch, no run directory, no window.
fn refused_without_side_effects(h: &RunHarness, out: &Output, message: &str) {
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        stdout(out),
        stderr(out)
    );
    assert_eq!(stderr(out).trim_end(), message);
    assert!(stdout(out).is_empty(), "{}", stdout(out));
    assert!(no_run_branches(&h.repo), "a run branch was created");
    let runs = h.data().join("runs");
    let entries: Vec<_> = std::fs::read_dir(&runs)
        .map(|d| d.map(|e| e.unwrap().file_name()).collect())
        .unwrap_or_default();
    assert!(entries.is_empty(), "{}: {entries:?}", runs.display());
    assert!(h.windows().is_empty(), "{:?}", h.windows());
}

/// The planned path's refusal for a triage of `kinds`/`scale` by `source`.
fn planned(scale: Scale, source: DeciderSource, fallback: Option<&str>, reason: &str) -> String {
    refused_message(&TriageInfo {
        kinds: vec![TaskKind::Code],
        scale,
        path: RunPath::Plan,
        reason: reason.to_string(),
        source,
        fallback_reason: fallback.map(str::to_string),
        at: 0,
    })
}

fn triage_calls(h: &RunHarness) -> usize {
    h.decider_calls()
        .iter()
        .filter(|c| c["kind"] == "triage")
        .count()
}

#[test]
fn e2e_green_s_task_on_the_fast_path() {
    let h = harness("claude", "", &[], &[]);
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    let watcher = h.subscribe();
    let out = h.start_goal("add a", &[]);
    let id = started(&h, &out);
    assert!(
        stderr(&out).contains("fast path: one task, no plan gate"),
        "{}",
        stderr(&out)
    );
    assert_eq!(triage_calls(&h), 1);

    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(run.path, Some(RunPath::Fast));
    assert_eq!(run.approved_by.as_deref(), Some("fast path"));
    let triage = run.triage.as_ref().expect("the triage is recorded");
    assert_eq!(triage.source, DeciderSource::Decider);
    assert_eq!(triage.scale, Scale::Single);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);

    watcher.wait_for(REQUEST_WAIT, |m| {
        matches!(m, DaemonMsg::Run(RunReply::Snapshot(s))
            if s.runs.iter().any(|r| r.run_id == id && r.state == RunState::Complete))
    });
    let states: Vec<RunState> = watcher
        .snapshots()
        .iter()
        .flat_map(|s| s.runs.iter().filter(|r| r.run_id == id).map(|r| r.state))
        .collect();
    assert!(!states.is_empty());
    assert!(
        !states.contains(&RunState::AwaitingApproval),
        "a fast-path run showed the plan gate: {states:?}"
    );

    let repo = h.repo.display().to_string();
    let accept = h.anthrex_input(&["run", "accept", &id, "--yes", "--dir", &repo], "");
    assert!(
        accept.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&accept),
        stderr(&accept)
    );
    assert_eq!(h.git(&["show", "main:a.txt"]), "a");
}

#[test]
fn e2e_goal_needing_a_plan_is_refused_without_side_effects() {
    let h = harness("claude", "", &[], &[]);
    let reason = "it needs a new module and a migration";
    h.decider(
        "triage",
        1,
        json!({"answer": {"kinds": ["code"], "scale": "plan", "reason": reason, "task": null}}),
    );
    let out = h.start_goal("rework storage", &[]);
    let message = planned(Scale::Plan, DeciderSource::Decider, None, reason);
    assert!(
        message.contains("this goal needs a planned run"),
        "{message}"
    );
    refused_without_side_effects(&h, &out, &message);
    assert_eq!(triage_calls(&h), 1);
}

#[test]
fn e2e_goal_touching_a_hub_file_is_refused_without_side_effects() {
    let h = harness("claude", "hub = [\"core/**\"]\n", &[], &[]);
    h.decider("triage", 1, triage_single(&["core/x.txt"]));
    let out = h.start_goal("change the core", &[]);
    let reason = "the fast path does not apply: task t1 touches a hub file";
    let message = planned(Scale::Single, DeciderSource::Decider, None, reason);
    refused_without_side_effects(&h, &out, &message);
}

/// Whole-branch review I1: a fast-path task that owns a protected agent-config file
/// would get decision 56's grant with no plan the user approves, so the goal takes the
/// planned path, which M8b refuses, creating nothing.
#[test]
fn e2e_goal_owning_a_protected_file_takes_the_plan_path() {
    let h = harness("claude", "", &[], &[]);
    h.decider("triage", 1, triage_single(&["AGENTS.md"]));
    let out = h.start_goal("rewrite the agent instructions", &[]);
    let reason = "the fast path does not apply: task t1 owns a protected file (AGENTS.md)";
    let message = planned(Scale::Single, DeciderSource::Decider, None, reason);
    refused_without_side_effects(&h, &out, &message);
}

#[test]
fn e2e_goal_without_deciders_takes_the_plan_path() {
    let h = harness("off", "", &[], &[]);
    h.decider("triage", 1, triage_single(&["a.txt"]));
    let out = h.start_goal("add a", &[]);
    let reason = "triage fell back (deciders are off); without a decider the path is plan";
    let message = planned(
        Scale::Plan,
        DeciderSource::Fallback,
        Some("deciders are off"),
        reason,
    );
    assert!(message.contains("deciders are off"), "{message}");
    refused_without_side_effects(&h, &out, &message);
    assert_eq!(triage_calls(&h), 0, "mode off spawns no decider");
}

#[test]
fn e2e_goal_without_a_profile_starts_detection() {
    let files: Vec<(&str, &str)> = ADAPT_FILES.to_vec();
    let h = RunHarness::adapt("claude", PROFILE_LINES, &[], &files);
    h.onboarding_report(1, json!({"check": "sh check.sh"}));
    let out = h.start_goal("add a", &[]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_eq!(stderr(&out).trim_end(), DETECTION_STARTED);
    assert!(no_run_branches(&h.repo));
    assert_eq!(triage_calls(&h), 0, "no triage without a stored profile");
    let status = h.profile_status();
    let record = status.proposal.expect("detection has started");
    assert_eq!(record.origin, ProposalOrigin::Goal);
    // Detection runs to its end, so the test leaves no scout behind.
    let status = h.wait_profile(
        "detection to finish",
        |s| {
            s.proposal.as_ref().is_some_and(|p| {
                matches!(p.state, ProposalState::Ready | ProposalState::Failed { .. })
            })
        },
        PROFILE_WAIT,
    );
    let state = status.proposal.map(|p| p.state);
    assert_eq!(state, Some(ProposalState::Ready), "{}", h.log_tail());
}

/// Decision 22 step 2's refusal that starts detection (exact).
const DETECTION_STARTED: &str = "this repository has no stored profile; detection has started (anthrex profile status), then confirm it with anthrex profile confirm and start the goal again";

const HOOKED_SETTINGS: &str = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"true"}]}]}}"#;

#[test]
fn e2e_goal_runs_m8a_start_checks() {
    let h = harness(
        "claude",
        "",
        &[("ANTHREX_TEST_NO_SETTING_SOURCES", "1")],
        &[(".claude/settings.json", HOOKED_SETTINGS)],
    );
    h.decider("triage", 1, triage_single(&["a.txt"]));
    h.decider("triage", 2, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    let out = h.start_goal("add a", &[]);
    let settings = "this repository has project settings that headless Claude sessions would run without asking: .claude/settings.json; review them, then start again with --trust-project";
    refused_without_side_effects(&h, &out, settings);

    let out = h.start_goal("add a", &["--trust-project"]);
    let id = started(&h, &out);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(run.path, Some(RunPath::Fast));
    assert_eq!(
        run.trusted_project,
        vec![".claude/settings.json".to_string()]
    );
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
}

/// Waits with `sh` until `path` exists: a deadline loop of at most one `RUN_WAIT`
/// (1500 × 0.2 s = 300 s).
fn wait_for_file(path: &std::path::Path) -> Value {
    sh(&format!(
        "for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1",
        path.display()
    ))
}

#[test]
fn e2e_promote_records_intent_and_the_task_continues() {
    let h = harness("claude", "", &[], &[]);
    h.decider("triage", 1, triage_single(&["a.txt"]));
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[wait_for_file(&go), commit("a.txt", "a\n"), done("added a")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = started(&h, &h.start_goal("add a", &[]));
    h.wait_run(&id, |r| t(r, "t1").state == TaskState::Working, RUN_WAIT);

    let repo = h.repo.display().to_string();
    let promote = |h: &RunHarness| h.anthrex(&["run", "promote", &id, "--dir", &repo]);
    let out = promote(&h);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out).trim_end(),
        format!(
            "recorded: run {id} is marked for promotion to a planned run. Until the orchestrator exists (milestone 9) nothing else changes: the fast-path task continues and the run finishes as a fast-path run."
        )
    );
    let run = h.run(&id).unwrap();
    assert!(run.promote_requested_at.is_some());
    assert!(
        run.attention
            .iter()
            .any(|a| a.starts_with("promotion requested at")
                && a.ends_with("it takes effect when the orchestrator exists (milestone 9)")),
        "{:?}",
        run.attention
    );
    assert_eq!(t(&run, "t1").state, TaskState::Working);
    let again = promote(&h);
    assert!(again.status.success(), "{}", stderr(&again));
    assert!(
        stdout(&again).contains("was already marked for promotion at"),
        "{}",
        stdout(&again)
    );

    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(run.path, Some(RunPath::Fast));
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert!(run.promote_requested_at.is_some());
}
