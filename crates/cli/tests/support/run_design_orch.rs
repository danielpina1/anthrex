//! Milestone 9.6 task M9.6.19, split from `run_design.rs` (which re-exports it): the
//! scripted orchestrator's design path, gate by gate, and the run harness's helpers for
//! a design run: its configuration, the agents' scripts, the answer to the
//! orchestrator's question, and the waits on and approvals of each document gate.

use std::time::Duration;

use proto::{DocGateKind, RunInfo};
use serde_json::{Value, json};

use super::orch_script::{call, edit_plan, marker, passed, prompt, read};
use super::run_design::{
    ANSWER, DESIGN_LINES, DRAFTS_IN, Draft, Findings, QUESTION, SPEC, brainstormer_name,
    brainstormer_script, plan_edits, report, reviewer_name, reviewer_script,
};
use super::run_harness::RunHarness;
use super::run_orch::{ORCH_WAIT, framed};
use super::run_rounds::ok;

/// What the scripted orchestrator submits at each gate of a design run.
pub struct OrchDesign {
    /// The answer the test types after [`QUESTION`]; `None` asks nothing and starts
    /// the brainstorm with empty answers.
    pub answer: Option<String>,
    /// The two brainstormers' labels, whose drafts it reads.
    pub labels: [String; 2],
    pub report: String,
    /// Sent to spec review 1 with `ready: false`.
    pub spec_draft: String,
    /// Submitted with `ready: true`, answering review 1 with `spec_responses`.
    pub spec: String,
    pub spec_responses: Vec<Value>,
    /// The `edit_plan` edits sent to the plan review.
    pub plan: Vec<Value>,
    /// The answers to the plan review, in the submit that opens the plan gate.
    pub plan_responses: Vec<Value>,
}

impl OrchDesign {
    /// The fixture path: [`ANSWER`], drafts from `labels`, [`report`], [`SPEC`] for
    /// review and then ready, [`plan_edits`], and no findings to answer.
    pub fn fixture(labels: [&str; 2]) -> Self {
        Self {
            answer: Some(ANSWER.to_string()),
            labels: labels.map(String::from),
            report: report(labels),
            spec_draft: SPEC.to_string(),
            spec: SPEC.to_string(),
            spec_responses: vec![],
            plan: plan_edits(),
            plan_responses: vec![],
        }
    }
}

/// The question and the brainstorm's start (rule 48): the turn ends after the question,
/// and the user's typed answer starts the brainstorm. A [`marker`] comes first, so the
/// test types the answer only after the first message was read
/// ([`RunHarness::answer_question`]).
pub fn ask(answer: Option<&str>) -> Vec<Value> {
    let mut steps = vec![prompt()];
    let answers = match answer {
        Some(answer) => {
            steps.extend([marker(), json!({"print": QUESTION}), read(Some(answer))]);
            answer
        }
        None => "",
    };
    steps.push(call("start_brainstorm", json!({"answers": answers})));
    steps
}

/// Rule 49: once both drafts are in, read each and submit the merged `report`.
pub fn merge(labels: &[String; 2], report: &str) -> Vec<Value> {
    let mut steps = vec![read(Some(DRAFTS_IN))];
    for label in labels {
        let args = json!({"kind": "brainstorm_draft", "from": label});
        steps.push(call("get_doc", args));
    }
    steps.push(call(
        "submit_doc",
        json!({"kind": "brainstorm", "text": report}),
    ));
    steps
}

/// Waits for the wake note that the user approved `kind` v`n`.
pub fn approved(kind: &str, n: u32) -> Value {
    read(Some(&format!("the user approved the {kind} v{n}")))
}

/// Rule 50: the spec sent to review `k` (`ready: false`), then its findings' wake.
pub fn spec_for_review(text: &str, k: u32) -> Vec<Value> {
    vec![
        call(
            "submit_doc",
            json!({"kind": "spec", "text": text, "ready": false}),
        ),
        read(Some(&format!("spec review {k} is in"))),
    ]
}

/// Rule 50: the spec submitted ready, answering the latest review with `responses`.
pub fn spec_ready(text: &str, responses: &[Value]) -> Value {
    call(
        "submit_doc",
        json!({"kind": "spec", "text": text, "ready": true, "responses": responses}),
    )
}

/// Rule 51: the plan's tasks submitted, which sends the plan to its review, then the
/// findings' wake.
pub fn plan_for_review(edits: Vec<Value>) -> Vec<Value> {
    vec![
        edit_plan(edits, json!({"submit": true})),
        read(Some("plan review is in")),
    ]
}

/// Rule 51: the plan submitted again with the review's answers, opening its gate.
pub fn plan_ready(responses: &[Value]) -> Value {
    edit_plan(vec![], json!({"submit": true, "responses": responses}))
}

/// The scripted orchestrator's whole design path: the question, the brainstorm and
/// its report, approved as v1; the spec through one review, approved as v1; the plan
/// through one review, approved as v1. The run then runs its tasks; the caller adds
/// what follows (a `run_status` wait, the summary, a final `read_message`).
pub fn orch_design_steps(d: &OrchDesign) -> Vec<Value> {
    let mut steps = ask(d.answer.as_deref());
    steps.extend(merge(&d.labels, &d.report));
    steps.push(approved("brainstorm", 1));
    steps.extend(spec_for_review(&d.spec_draft, 1));
    steps.push(spec_ready(&d.spec, &d.spec_responses));
    steps.push(approved("spec", 1));
    steps.extend(plan_for_review(d.plan.clone()));
    steps.push(plan_ready(&d.plan_responses));
    steps.push(approved("plan", 1));
    steps
}

impl RunHarness {
    /// A harness for a design run: [`RunHarness::orch`]'s, with the design flow on by
    /// default ([`DESIGN_LINES`]) and `orchestrator` lines, and `files` in the base
    /// commit. Both runtimes are `fake-agent`, so the brainstormers are `claude` and
    /// `codex`.
    pub fn design(orchestrator: &str, files: &[(&str, &str)]) -> RunHarness {
        RunHarness::orch(&format!("{DESIGN_LINES}{orchestrator}"), files)
    }

    /// Writes brainstormer `label`'s `n`th session's script.
    pub fn brainstormer(&self, label: &str, n: u32, draft: Draft<'_>) {
        self.script(
            &brainstormer_name(label, n),
            &brainstormer_script(label, draft),
        );
    }

    /// Writes the `n`th session's script of review `k` of `kind`.
    pub fn reviewer(&self, kind: &str, k: u32, n: u32, findings: Findings) {
        self.script(
            &reviewer_name(kind, k, n),
            &reviewer_script(kind, k, findings),
        );
    }

    /// Rule 48's answer: once orchestrator script `script` has read its first message
    /// and asked [`QUESTION`] (its turn ended at the read), [`ANSWER`] typed into its
    /// window `window`.
    pub fn answer_question(&self, script: &str, window: u32) {
        let asked = |log: &[Value]| passed(log, script) >= 1;
        self.wait_log("the orchestrator's question", asked, ORCH_WAIT);
        self.wait_orchestrator_idle(window);
        self.type_into(window, framed(ANSWER).as_bytes());
    }

    /// Waits, at most `wait`, until run `run` waits at its `kind` gate at v`n`, the
    /// orchestrator not revising it; the run.
    pub fn wait_doc_gate(&self, run: &str, kind: DocGateKind, n: u32, wait: Duration) -> RunInfo {
        let at = |r: &RunInfo| {
            (r.doc_gate.as_ref())
                .is_some_and(|g| g.kind == kind && g.version == n && g.revising.is_none())
        };
        self.wait_run(run, at, wait)
    }

    /// `anthrex run approve <run> --gate <kind>`, once the run waits there at v`n`.
    pub fn approve_doc(&self, run: &str, kind: DocGateKind, n: u32) {
        self.wait_doc_gate(run, kind, n, ORCH_WAIT);
        let repo = self.repo.display().to_string();
        let args = [
            "run",
            "approve",
            run,
            "--gate",
            kind.label(),
            "--dir",
            &repo,
        ];
        ok(&self.anthrex_input(&args, ""));
    }
}
