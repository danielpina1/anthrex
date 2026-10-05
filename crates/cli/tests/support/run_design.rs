//! Milestone 9.6 task M9.6.19: the design flow's scripted agents and fixture documents.
//!
//! - **Brainstormers** run `brainstormer-<label>-<n>.jsonl` and **document reviewers**
//!   `doc_reviewer-<doc>-r<k>-<n>.jsonl`, keyed by their `anthrex mcp --agent-label`
//!   (ruling T1-O3); `k` is the run-wide review number (rulings T10-1, T15-9), `n` the
//!   session. A Codex design agent is never resumed: its one relaunch after a turn that
//!   ended without submitting (ruling T8-7) is a new session, which claims `n + 1`. A
//!   Claude one's nudge arrives on the same session's stdin (ruling T8-4).
//! - **The orchestrator** runs [`orch_design_steps`] (`run_design_orch.rs`, re-exported
//!   here), gate by gate, each step waiting on the wake note that follows the user's
//!   decision.
//! - **The fixtures** pass the real template, numbering and brief checks of task M9.6.4
//!   (`run_design_fixtures.rs` holds the test that says so).
//!
//! Every agent here is `fake-agent`: the run harness pins every `*_BIN` and puts the
//! config, socket and data dir under its temp dir, so nothing reaches a real `claude`,
//! `codex` or `gh`.

use serde_json::{Value, json};

use super::orch_script::{add, call, plan_task, read};
use daemon::scout::design_spec::{BRAINSTORMER_NUDGE, DOC_REVIEWER_NUDGE};

// The scripted orchestrator's steps and the harness's design helpers, kept in their own
// file for the 400-line rule; every test reaches them through this module.
#[allow(unused_imports)] // Only the design flow's test binaries use them.
pub use super::run_design_orch::*;

/// `[orchestrator]` lines that put the design flow on for every planned goal.
pub const DESIGN_LINES: &str = "design.default = \"full\"\n";

/// The question the scripted orchestrator asks in its window (rule 48).
pub const QUESTION: &str = "Should a.txt end with a newline?";

/// The user's answer, typed into the orchestrator's window and passed to
/// `start_brainstorm`.
pub const ANSWER: &str = "yes, one line ending in a newline";

/// The wake note that both drafts are in (Messages, exact).
pub const DRAFTS_IN: &str =
    "both brainstorm drafts are in; read them with get_doc and submit the merged report";

/// The file the fixture plan's task writes.
pub const PLAN_FILE: &str = "a.txt";

/// A brainstormer's draft in the six-section template (DF §3.3), naming its `label`.
pub fn draft(label: &str) -> String {
    format!(
        "## Understanding
Add a.txt holding one line ({label}'s reading). Success: a.txt is on the base branch.

## Assumptions
- (assumed) a.txt does not exist yet.

## Constraints found
- README:1 is the only tracked file.

## Approaches
### 1. Write the file directly
One task writes a.txt. Files: a.txt. Trade-offs: none. Risks: none. Size S.

### 2. Generate it from a script
A script writes a.txt. Files: gen.sh, a.txt. Trade-offs: more to review. Size M.

## Recommendation
Write the file directly: it is the smallest change.

## Questions for you
None.
"
    )
}

/// The merged brainstorm report (DF §3.4) over the drafts of `labels`: one approach
/// both drafts share, one only the second proposed, and a recommendation naming the
/// first.
pub fn report(labels: [&str; 2]) -> String {
    let [a, b] = labels;
    format!(
        "## Where they agree
Both write a.txt in one small task.

## Where they disagree
{a} writes the file directly; {b} also weighs a generator script. Judgment: write it directly.

## Approaches
### 1. Write the file directly [both]
One task writes a.txt. Size S.

### 2. Generate it from a script [{b}]
A script writes a.txt. Size M.

## Recommendation
Write the file directly: it is the smallest change.

## Questions for you
None.
"
    )
}

/// The merged report when brainstormer `failed` failed with `reason` and only
/// `survivor`'s draft came in (DF §3.5): its first line names the failure.
pub fn single_report(survivor: &str, failed: &str, reason: &str) -> String {
    let body = report([survivor, survivor]);
    format!("single brainstorm: {failed} failed: {reason}\n\n{body}")
}

/// The spec (DF §4.1) with two requirements, each with its acceptance check, and no
/// open question: it passes with `ready` true or false.
pub const SPEC: &str = "# Add a.txt

## Goal and success criteria
a.txt exists on the base branch with one line.

## Non-goals
Nothing else changes.

## Approach
Write the file directly, as the brainstorm recommends.

## Design
One task writes a.txt in the repository root.

## Requirements
R1 a.txt exists at the repository root. Acceptance: `test -f a.txt` succeeds.
R2 a.txt holds exactly one line. Acceptance: `wc -l < a.txt` prints 1.

## Interfaces
None.

## Errors and edge cases
An existing a.txt is replaced.

## Testing
A check-mode task; its reviewer reads the file.

## Risks
None.

## Open questions
";

/// The spec's requirement ids.
pub const REQUIREMENTS: [&str; 2] = ["R1", "R2"];

/// A round-2 amendment of [`SPEC`] (decision 14): it adds R3, continuing the numbering.
pub const AMENDMENT: &str = "# Add b.txt too

## Goal and success criteria
b.txt exists on the base branch beside a.txt.

## Non-goals
a.txt does not change.

## Approach
Write b.txt directly, as round 1 wrote a.txt.

## Design
One task writes b.txt in the repository root.

## Requirements
R3 b.txt exists at the repository root. Acceptance: `test -f b.txt` succeeds.

## Interfaces
None.

## Errors and edge cases
An existing b.txt is replaced.

## Testing
A check-mode task; its reviewer reads the file.

## Risks
None.

## Open questions
";

/// A design run's task brief (decision 19): each heading on its own line.
pub fn brief(file: &str) -> String {
    format!(
        "Write {file}.\nFiles:\n- {file}\nTests first:\n- none; a check-mode task\nSteps:\n- write one line to {file}\nAcceptance:\n- {file} holds one line\nVerify:\n- wc -l < {file}\n"
    )
}

/// An S `check` task `id` writing `file` that covers `covers`, with the extra keys of
/// `more` (`stage`), as an `add_task` edit.
pub fn covered(id: &str, file: &str, covers: &[&str], more: Value) -> Value {
    let mut extra = json!({"brief": brief(file), "covers": covers});
    for (key, value) in more.as_object().cloned().unwrap_or_default() {
        extra[key] = value;
    }
    add(plan_task(id, &[file], extra))
}

/// The fixture plan: `t1` writes [`PLAN_FILE`] and covers every requirement.
pub fn plan_edits() -> Vec<Value> {
    vec![covered("t1", PLAN_FILE, &REQUIREMENTS, json!({}))]
}

/// `spawn_subplanner` of epic `epic` over `area`, owning `covers` (ruling T11-1: a
/// design run's spawn must name them).
pub fn subplanner(epic: &str, area: &[&str], covers: &[&str]) -> Value {
    call(
        "spawn_subplanner",
        json!({"epic": epic, "title": format!("Epic {epic}"), "area": area,
            "brief": format!("Plan epic {epic}."), "covers": covers}),
    )
}

/// A review finding.
pub fn finding(id: &str, severity: &str, place: &str, text: &str) -> Value {
    json!({"id": id, "severity": severity, "place": place, "text": text})
}

/// The orchestrator's answer `fixed` to finding `id`.
pub fn fixed(id: &str) -> Value {
    json!({"id": id, "answer": "fixed"})
}

/// The orchestrator's answer `kept: <reason>` to finding `id`: a disputed finding.
pub fn kept(id: &str, reason: &str) -> Value {
    json!({"id": id, "answer": format!("kept: {reason}")})
}

/// How a brainstormer's session goes.
pub enum Draft<'a> {
    /// Submits the fixture [`draft`] for its label.
    Fixture,
    /// Submits this text; a refusal ends the script (exit 3).
    Text(&'a str),
    /// Ends its turn without submitting. A Codex brainstormer is then relaunched once
    /// as a new session (ruling T8-7), which takes the next script.
    Unsubmitted,
    /// Ends its turn without submitting, reads the nudge on the same session's stdin,
    /// then submits the fixture draft: a Claude brainstormer (ruling T8-4).
    AfterNudge,
    /// Fails its turn with this error.
    Fails(&'a str),
}

/// The steps of brainstormer `label`'s session.
pub fn brainstormer_script(label: &str, draft: Draft<'_>) -> Vec<Value> {
    let submit = |text: &str| {
        call(
            "submit_doc",
            json!({"kind": "brainstorm_draft", "text": text}),
        )
    };
    match draft {
        Draft::Fixture => vec![submit(&self::draft(label))],
        Draft::Text(text) => vec![submit(text)],
        Draft::Unsubmitted => vec![json!({"end_turn": {}})],
        Draft::AfterNudge => vec![
            json!({"end_turn": {}}),
            read(Some(BRAINSTORMER_NUDGE)),
            submit(&self::draft(label)),
        ],
        Draft::Fails(error) => vec![json!({"fail_turn": {"error": error}})],
    }
}

/// `brainstormer-<label>-<n>`: brainstormer `label`'s `n`th session's script.
pub fn brainstormer_name(label: &str, n: u32) -> String {
    format!("brainstormer-{label}-{n}")
}

/// How a document reviewer's session goes.
pub enum Findings {
    /// Reads its document, then submits these findings (none is a valid review).
    Submits(Vec<Value>),
    /// Reads its document and ends its turn without submitting (ruling T8-7).
    Unsubmitted,
    /// Reads its document, ends its turn without submitting, reads the nudge on the
    /// same session's stdin, then submits these: a Claude reviewer (ruling T8-4).
    AfterNudge(Vec<Value>),
    /// Fails its turn with this error.
    Fails(String),
}

/// The steps of the reviewer of review `n` of `kind` (`spec` or `plan`). A spec
/// reviewer reads its review draft by number (`get_doc { draft: n }`, ruling T5-1); a
/// plan reviewer reads the plan sent to review, then the approved spec.
pub fn reviewer_script(kind: &str, n: u32, findings: Findings) -> Vec<Value> {
    let mut reads = match kind {
        "spec" => vec![call("get_doc", json!({"kind": "spec", "draft": n}))],
        _ => vec![
            call("get_doc", json!({"kind": kind})),
            call("get_doc", json!({"kind": "spec"})),
        ],
    };
    let submit = |findings: Vec<Value>| call("submit_findings", json!({"findings": findings}));
    match findings {
        Findings::Submits(list) => reads.push(submit(list)),
        Findings::Unsubmitted => reads.push(json!({"end_turn": {}})),
        Findings::AfterNudge(list) => reads.extend([
            json!({"end_turn": {}}),
            read(Some(DOC_REVIEWER_NUDGE)),
            submit(list),
        ]),
        Findings::Fails(error) => return vec![json!({"fail_turn": {"error": error}})],
    }
    reads
}

/// `doc_reviewer-<kind>-r<k>-<n>`: the `n`th session of review `k` of `kind`.
pub fn reviewer_name(kind: &str, k: u32, n: u32) -> String {
    format!("doc_reviewer-{kind}-r{k}-{n}")
}
