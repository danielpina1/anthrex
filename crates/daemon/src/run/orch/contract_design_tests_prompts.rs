//! Milestone 9.6 task M9.6.14 (DF §8.4, §9; rulings T11-1 and T14-1): a design run's
//! first prompt names its phases and carries rules 47 to 53; every session of it is told
//! where the run is; a run without the design flow gets 9.5's contract and prompts byte
//! for byte. Pure.

use proto::{DesignMode, DocAuthor, DocGateKind, DocKind, RunPath, RunState};

use super::*;
use crate::run::design::state::{DesignState, DocGate, NewDoc, sha256_hex, store};
use crate::run::orch::contract::{
    ORCHESTRATOR_CONTRACT, fresh_first_prompt, orchestrator_first_prompt,
};
use crate::run::orch::contract_rounds::handoff_prompt;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

/// Rules 47 to 52, exactly as the brief's "Contracts (exact)" has them, with the
/// default `max_questions` (5).
const RULES_47_TO_52: &str = "\
47. This run uses the design flow: brainstorming, then specifying, then planning. Each ends at a gate only the user opens; read verdicts with run_status, never assume one.
48. In brainstorming, ask the user at most 5 short questions in your window, one at a time; when they answer or say skip, call start_brainstorm with their answers.
49. When both brainstorm drafts are in, read them with get_doc and submit one merged report with submit_doc kind \"brainstorm\": where they agree, where they disagree (each side, then your judgment), the approaches tagged [<label>] or [both], one recommendation naming a listed approach, and questions for the user. Never paste a draft wholesale.
50. In specifying, write the spec in the template you were given; every requirement is a line \"R<n> …\" with its acceptance check. Submit with ready false for review; answer every finding (\"fixed\" or \"kept: <reason>\") in the submit with ready true.
51. In planning, every requirement must be covered by a task's covers, and every brief has the headings Files:, Tests first:, Steps:, Acceptance:, Verify:. Answer the plan review's findings in edit_plan's responses when you submit again.
52. When the user asks for changes, revise and submit the next version; the user's note is in run_status. Never approve, and never call a gate approved.";

/// Ruling T11-1: a design run's sub-planner needs `covers`.
const RULE_53: &str = "53. In planning, spawn_subplanner needs covers: the requirement ids its epic owns. Its sub-planner is given those requirements, and the whole plan's coverage is still checked when you submit.";

const RETRY: &str = "If an anthrex tool is reported missing, call get_context again before anything else: the server may still be connecting.";

/// The where line's fixed head.
const WHERE: &str = "Where the run is now (run_status has anything newer):";

fn plain_run() -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/src/x.rs\"]", "")],
    ));
    run.path = Some(RunPath::Plan);
    run
}

/// A design run, in brainstorming, nothing stored yet.
fn design_run() -> Run {
    let mut run = plain_run();
    run.design_mode = DesignMode::Full;
    run.orch.design = Some(DesignState::default());
    run.state = RunState::Brainstorming;
    run
}

/// Stores a version of `kind`: its number.
fn stored(run: &mut Run, kind: DocKind) -> u32 {
    let doc = NewDoc::new(kind, DocAuthor::Orchestrator, "submitted", "text");
    store(run, doc, 1_000).unwrap().0.n
}

/// `run` waits at the `kind` gate's version `n`.
fn at_gate(run: &mut Run, kind: DocGateKind, n: u32, revising: Option<&str>) {
    run.state = RunState::AwaitingApproval;
    let design = run.orch.design.as_mut().unwrap();
    design.gate = Some(DocGate {
        kind,
        version: n,
        opened_at: 1_000,
        revising: revising.map(String::from),
        review: false,
        cause: Default::default(),
    });
}

/// A document's path in the run's design folder.
fn path(file: &str) -> String {
    format!("/tmp/data/runs/add-password-reset-3f9a/design/{file}")
}

/// The handoff of a fresh session on chain `chain-1` after run `0b1c`.
fn handoff(run: &Run) -> String {
    handoff_prompt(
        &fresh_first_prompt(run),
        ("chain-1", "0b1c", "accepted"),
        Some("Done."),
        None,
    )
}

/// The brief's rules 47 to 52 and ruling T11-1's 53 ride in a design run's first prompt
/// and so in its handoff (its first prompt is the handoff's head), with the run's own
/// questions limit; never in the contract every run shares, nor in a run without the
/// design flow.
#[test]
fn rules_47_to_52_are_exact_and_only_in_design_runs() {
    let mut run = design_run();
    let rules = format!("{RULES_47_TO_52}\n{RULE_53}");
    assert_eq!(design_rules(&run).as_deref(), Some(rules.as_str()));
    let first = orchestrator_first_prompt(&run);
    assert!(
        first.ends_with(&format!(
            "\n\nYour contract's rules continue, for this run:\n{rules}"
        )),
        "{first}"
    );
    assert!(handoff(&run).contains(&rules), "{}", handoff(&run));
    run.limits.orch.design.max_questions = 2;
    let rule48 = "48. In brainstorming, ask the user at most 2 short questions in your window,";
    assert!(orchestrator_first_prompt(&run).contains(rule48));
    assert_eq!(
        DESIGN_RULES.replace("{max_questions}", "5"),
        rules,
        "design_rules! is the brief's text with the limit's place held"
    );

    let plain = plain_run();
    assert_eq!(design_rules(&plain), None);
    for text in [
        ORCHESTRATOR_CONTRACT.to_string(),
        orchestrator_first_prompt(&plain),
        handoff(&plain),
    ] {
        for n in 47..=53 {
            let line = format!("\n{n}. ");
            assert!(!text.contains(&line), "{n} in {text}");
        }
        assert!(!text.contains("start_brainstorm"), "{text}");
    }
}

/// The first prompt names the phases, the design tools by their Claude ids, and where to
/// start, then rules 47 to 53. `--yes` leaves the plan gate's line as it is (decision
/// 6), and the line names both cases (final fix wave FW-52, ruling T15-8): every design
/// gate stops, and only a round the user starts in off mode with --yes starts its plan
/// at once. Ruling T14-3: the step after scouting is the session's own
/// (`session_prompt`), never stored, so a later session is not sent back to rule 48.
#[test]
fn the_first_prompt_names_the_phases() {
    let mut run = design_run();
    let expected = format!(
        "[anthrex] You are the orchestrator of run add-password-reset-3f9a in /tmp/x.\n\
         Goal: Test goal\n\
         Path: plan\n\
         Plan gate: the user approves each design document and your submitted plan in the run view, even if the run was started with --yes; only a round the user starts in off mode with --yes starts its plan at once\n\
         Design flow: brainstorming, then specifying, then planning, under rules 47 to 53 below. Its tools: start_brainstorm (in Claude: mcp__anthrex__start_brainstorm), submit_doc (in Claude: mcp__anthrex__submit_doc) and get_doc (in Claude: mcp__anthrex__get_doc).\n\
         Start with get_context, then scout. {RETRY}\n\
         \n\
         Your contract's rules continue, for this run:\n\
         {RULES_47_TO_52}\n\
         {RULE_53}"
    );
    assert_eq!(orchestrator_first_prompt(&run), expected);
    run.orch.yes = true;
    assert_eq!(orchestrator_first_prompt(&run), expected);
}

/// Ruling T14-1 and DF §8.4: a fresh session's handoff, as it is sent, ends with where
/// the run is: its phase, its open gate (kind and version) and each approved document's
/// path. The stored handoff itself never carries it, so a later session's is never
/// stale nor doubled.
#[test]
fn the_handoff_names_the_phase_the_gate_and_the_documents() {
    let mut run = design_run();
    let b = stored(&mut run, DocKind::Brainstorm);
    stored(&mut run, DocKind::Spec);
    let s = stored(&mut run, DocKind::Spec);
    assert_eq!((b, s), (1, 2));
    at_gate(&mut run, DocGateKind::Spec, 2, None);
    let handoff = handoff(&run);
    assert!(!handoff.contains(WHERE), "{handoff}");
    assert_eq!(
        session_prompt(&run, &handoff),
        format!(
            "{handoff}\n{WHERE} phase specifying; open gate: spec v2, waiting for the user; \
             approved documents: brainstorm v1 ({}).",
            path("brainstorm-v1.md")
        )
    );
    at_gate(&mut run, DocGateKind::Spec, 2, Some("Name the store."));
    assert_eq!(
        where_the_run_is(&run).unwrap(),
        format!(
            "{WHERE} phase specifying; open gate: spec v2, which you are revising; \
             approved documents: brainstorm v1 ({}).",
            path("brainstorm-v1.md")
        )
    );
}

/// The where line through a design run's life: nothing approved in brainstorming; the
/// approved spec's own version in planning; the open plan gate, also while paused; all
/// three once the plan is approved; and a back to the brainstorm approves nothing again.
#[test]
fn the_where_line_follows_the_run() {
    let mut run = design_run();
    assert_eq!(
        where_the_run_is(&run).unwrap(),
        format!("{WHERE} phase brainstorming; open gate: none; approved documents: none.")
    );
    stored(&mut run, DocKind::Brainstorm);
    stored(&mut run, DocKind::Spec);
    stored(&mut run, DocKind::Spec);
    let design = run.orch.design.as_mut().unwrap();
    design.approved_spec = Some(1);
    run.state = RunState::Planning;
    let two = format!(
        "brainstorm v1 ({}), spec v1 ({})",
        path("brainstorm-v1.md"),
        path("spec-v1.md")
    );
    assert_eq!(
        where_the_run_is(&run).unwrap(),
        format!("{WHERE} phase planning; open gate: none; approved documents: {two}.")
    );
    stored(&mut run, DocKind::Plan);
    at_gate(&mut run, DocGateKind::Plan, 1, None);
    run.paused_from = Some(RunState::AwaitingApproval);
    run.state = RunState::Paused;
    assert_eq!(
        where_the_run_is(&run).unwrap(),
        format!(
            "{WHERE} phase planning (paused); open gate: plan v1, waiting for the user; \
             approved documents: {two}."
        )
    );
    run.orch.design.as_mut().unwrap().gate = None;
    run.paused_from = None;
    run.state = RunState::Running;
    run.approved_at = Some(2_000);
    assert_eq!(
        where_the_run_is(&run).unwrap(),
        format!(
            "{WHERE} phase running; open gate: none; approved documents: {two}, plan v1 ({}).",
            path("plan-v1.md")
        )
    );
    run.approved_at = None;
    run.orch.design.as_mut().unwrap().approved_spec = None;
    at_gate(&mut run, DocGateKind::Brainstorm, 1, Some("Rethink R2."));
    assert_eq!(
        where_the_run_is(&run).unwrap(),
        format!(
            "{WHERE} phase brainstorming; open gate: brainstorm v1, which you are revising; \
             approved documents: none."
        )
    );
    run.orch.design.as_mut().unwrap().halted_from = Some(RunState::Brainstorming);
    run.orch.design.as_mut().unwrap().gate = None;
    run.state = RunState::Halted;
    assert!(
        where_the_run_is(&run)
            .unwrap()
            .contains(" phase brainstorming (halted); open gate: none; "),
        "{:?}",
        where_the_run_is(&run)
    );
}

/// Ruling T14-3: a session asking the user's questions (brainstorming, no answers
/// recorded) is sent rule 48's step after where the run is; with a questions limit of 0
/// it is told to start the brainstorm with empty answers, rule 48 kept exact; once the
/// answers are recorded, or in a later phase, no step follows.
#[test]
fn the_session_step_follows_the_questions_limit_and_the_phase() {
    let mut run = design_run();
    let first = orchestrator_first_prompt(&run);
    let line = where_the_run_is(&run).unwrap();
    assert_eq!(
        session_prompt(&run, &first),
        format!("{first}\n{line}\nThen follow rule 48.")
    );
    run.limits.orch.design.max_questions = 0;
    let first = orchestrator_first_prompt(&run);
    let rule48 = "48. In brainstorming, ask the user at most 0 short questions in your window, one at a time; when they answer or say skip, call start_brainstorm with their answers.";
    assert!(first.contains(rule48), "{first}");
    let sent = session_prompt(&run, &first);
    assert_eq!(
        sent,
        format!(
            "{first}\n{line}\nThen call start_brainstorm with empty answers: this run asks no questions."
        )
    );
    assert!(!sent.contains("follow rule 48"), "{sent}");
    run.limits.orch.design.max_questions = 5;
    run.orch.design.as_mut().unwrap().answers = Some("skip".into());
    assert_eq!(session_prompt(&run, &first), format!("{first}\n{line}"));
    run.orch.design.as_mut().unwrap().answers = None;
    run.state = RunState::Specifying;
    let line = where_the_run_is(&run).unwrap();
    assert_eq!(session_prompt(&run, &first), format!("{first}\n{line}"));
}

/// A run without the design flow: 9.5's contract (its SHA-256 at 9.5's head `f4df79c4`)
/// and first prompt byte for byte, no where line, and a handoff sent as built.
#[test]
fn a_non_design_contract_is_unchanged() {
    assert_eq!(
        sha256_hex(ORCHESTRATOR_CONTRACT.as_bytes()),
        NINE_FIVE_CONTRACT_SHA256
    );
    let run = plain_run();
    assert_eq!(
        orchestrator_first_prompt(&run),
        format!(
            "[anthrex] You are the orchestrator of run add-password-reset-3f9a in /tmp/x.\n\
             Goal: Test goal\n\
             Path: plan\n\
             Plan gate: the user approves your submitted plan in the run view\n\
             Start with get_context, then scout, then plan. {RETRY}"
        )
    );
    assert_eq!(where_the_run_is(&run), None);
    let handoff = handoff(&run);
    assert_eq!(session_prompt(&run, &handoff), handoff);
    // A run with the design flow's state but `Off` (none is built so) adds nothing.
    let mut off = design_run();
    off.design_mode = DesignMode::Off;
    assert_eq!(where_the_run_is(&off), None);
    assert_eq!(design_rules(&off), None);
    // Final fix wave FW-66 (T14 m2): nor a step.
    assert_eq!(session_step(&off), None);
    assert_eq!(session_prompt(&off, &handoff), handoff);
}

/// Final fix wave FW-67 (T14 m3): a run paused while it asks its questions is still
/// asking (its `paused_from`), so its session gets rule 48's step; paused after the
/// answers, none.
#[test]
fn a_paused_run_keeps_the_step_of_the_phase_it_paused_in() {
    let mut run = design_run();
    run.paused_from = Some(RunState::Brainstorming);
    run.state = RunState::Paused;
    assert_eq!(session_step(&run), Some(ASK_STEP));
    run.orch.design.as_mut().unwrap().answers = Some("skip".into());
    assert_eq!(session_step(&run), None);
    run.orch.design.as_mut().unwrap().answers = None;
    run.paused_from = Some(RunState::Specifying);
    assert_eq!(session_step(&run), None);
}

/// `ORCHESTRATOR_CONTRACT`'s SHA-256 at 9.5's head (`f4df79c4`), whose text the design
/// flow has not changed (`git diff f4df79c4 -- crates/daemon/src/run/orch/contract.rs`
/// touches only `first_prompt`'s body, never the constant), with milestone 9.8's rules
/// 4, 21, 22 and 28 (task M9.8.9, decision 31; 9.5's was `e1cd7e40…54ed`), and milestone
/// 9.9's rules 2, 24, 26, 37, 40, 41 and 42 (task M9.9.10; 9.8's was `b888a5a5…ffaf`),
/// and rule 26's promotion-hold sentence (final review I-2; T10's was `c6e792fd…ef86`).
const NINE_FIVE_CONTRACT_SHA256: &str =
    "f82c973c0f0578d723ddf418f7b894d53151e52325299b946f68e8aa16329c2e";
