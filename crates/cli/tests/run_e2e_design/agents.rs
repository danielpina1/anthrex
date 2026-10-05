//! The brainstormers when one fails (DF §3.5, ruling T8-7), and `--yes`, which skips
//! none of the three gates (decision 6).

use proto::{DesignAgentStatus, DocGateKind, RunState};
use serde_json::json;

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_design::*;

/// Ruling T8-7: a Codex design agent whose two sessions both end without its submit.
const TWICE: &str = "ended twice without submitting";

#[test]
fn e2e_one_brainstormer_fails() {
    let h = harness("");
    h.brainstormer("claude", 1, Draft::Fixture);
    // The Codex brainstormer ends its turn without submitting, is relaunched fresh once
    // with the same pack and one line more, and ends without submitting again.
    h.brainstormer("codex", 1, Draft::Unsubmitted);
    h.brainstormer("codex", 2, Draft::Unsubmitted);
    let failed = format!(
        "one brainstormer failed (codex: {TWICE}); read the other draft with get_doc and submit the merged report"
    );
    let single = single_report("claude", "codex", TWICE);
    let mut steps = ask(None);
    steps.push(read(Some(&failed)));
    steps.push(call(
        "get_doc",
        json!({"kind": "brainstorm_draft", "from": "claude"}),
    ));
    steps.push(call(
        "submit_doc",
        json!({"kind": "brainstorm", "text": single}),
    ));
    steps.push(approved("brainstorm"));
    steps.extend([marker(), read(None)]);
    let (run, _) = start(&h, &steps, &[]);

    let info = approve(&h, &run, DocGateKind::Brainstorm, 1);
    wait_passed(&h, 1);
    assert_eq!(h.run(&run).unwrap().state, RunState::Specifying);
    // The gate showed the report whose first line names the failure (DF §3.5).
    let report = stored(&h, &run, "brainstorm-v1.md");
    let head = format!("single brainstorm: codex failed: {TWICE}\n");
    assert!(report.starts_with(&head), "{report}");
    let agents: Vec<_> = (info.design_agents.iter())
        .map(|a| (a.label.as_str(), a.state, a.sessions))
        .collect();
    assert_eq!(
        agents,
        [
            ("claude", DesignAgentStatus::Done, 1),
            ("codex", DesignAgentStatus::Failed, 2),
        ]
    );
    // Both Codex sessions ran ephemeral and fresh, never resumed (ruling T8-7), and the
    // relaunch's message is the first's with the one line more.
    let argvs = h.io_lines(&brainstormer_name("codex", 1), "args");
    let relaunch = h.io_lines(&brainstormer_name("codex", 2), "args");
    assert_eq!((argvs.len(), relaunch.len()), (1, 1));
    for argv in argvs.iter().chain(&relaunch) {
        let argv: Vec<String> = serde_json::from_str(argv).unwrap();
        assert_eq!(argv.first().map(String::as_str), Some("exec"), "{argv:?}");
        assert!(argv.iter().any(|a| a == "--ephemeral"), "{argv:?}");
        assert!(!argv.iter().any(|a| a == "resume"), "{argv:?}");
    }
    let first = h.codex_messages(&brainstormer_name("codex", 1));
    let second = h.codex_messages(&brainstormer_name("codex", 2));
    let line = "Your previous attempt ended without submitting; submit it now with submit_doc.";
    assert!(!first[0].contains(line), "{}", first[0]);
    assert_eq!(
        second[0],
        first[0].replacen('\n', &format!("\n{line}\n"), 1)
    );
}

#[test]
fn e2e_yes_still_stops_at_every_gate() {
    let h = harness("");
    drafts(&h);
    h.reviewer("spec", 1, 1, Findings::Submits(vec![]));
    h.reviewer("plan", 1, 1, Findings::Submits(vec![]));
    green(&h, "t1", PLAN_FILE);
    let mut d = OrchDesign::fixture(LABELS);
    d.answer = None;
    let mut steps = orch_design_steps(&d);
    steps.extend([marker(), read(None)]);
    let (run, _) = start(&h, &steps, &["--yes"]);

    // Each gate waits for the user, the plan gate too, which `--yes` skips without the
    // design flow; the start said so once.
    for kind in [
        DocGateKind::Brainstorm,
        DocGateKind::Spec,
        DocGateKind::Plan,
    ] {
        let info = approve(&h, &run, kind, 1);
        assert_eq!(info.state, RunState::AwaitingApproval, "{kind:?}");
        assert_eq!(info.approved_by, None, "{kind:?}");
    }
    wait_passed(&h, 1);
    // The plan gate opened at v1 with no refusal: the fixture plan passes the real
    // coverage and brief checks (task 19's review, minor 1).
    assert!(
        refusals(&h, "edit_plan").is_empty(),
        "{:?}",
        refusals(&h, "edit_plan")
    );
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, DESIGN_RUN_WAIT);
    assert_eq!(info.approved_by.as_deref(), Some("user"));
    let log = crate::support::run_pr::log_lines(&h, &run);
    let yes = "design flow: --yes does not skip the brainstorm, spec or plan gates";
    assert_eq!(log.iter().filter(|l| *l == yes).count(), 1, "{log:#?}");
    // The orchestrator was never told the plan gate is off.
    let first = h.io_lines(ORCH, "stdin");
    let first: serde_json::Value = serde_json::from_str(&first[0]).unwrap();
    let first = first["first_message"].as_str().unwrap();
    assert!(first.contains("Plan gate: the user approves"), "{first}");
    assert!(!first.contains("Plan gate: off"), "{first}");
}
