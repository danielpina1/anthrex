//! Milestone 9.3 (keep going), the orchestrator's side of rounds: the round wake (KG
//! §2.4, decision 11) and every refusal and reply text of a round's start (decision 9,
//! 10 and 30; the brief texts of D10). Later tasks add rules 43–46, the next-goal wake
//! and the handoff prompt. Pure (design decision 2).

use proto::GOAL_MAX_CHARS;

use super::contract::WAKE_MAX_BYTES;
use crate::run::delivery::quote::fence;

/// D13: a request wake's own paste cap. A wake carries the fixed text (under 512 bytes
/// with every number at its widest), the request fenced and the notes' `wake_text`
/// (at most [`WAKE_MAX_BYTES`]). A fenced request is at most `4 × GOAL_MAX_CHARS`
/// bytes of text (four-byte characters), or `GOAL_MAX_CHARS` backticks between two
/// fences of `GOAL_MAX_CHARS + 1` (`report_escape::fence_for`), so `6 ×
/// GOAL_MAX_CHARS` bounds both, with room for the fences' line ends.
pub const REQUEST_WAKE_MAX_BYTES: usize = WAKE_MAX_BYTES + 6 * GOAL_MAX_CHARS + 1024;

/// KG §2.4's round wake: round `n` of run `h4`, whose stages so far end at `last`, and
/// the request fenced as user input (D1).
pub fn round_wake(h4: &str, n: u32, last: u16, request: &str) -> String {
    format!(
        "the user asks for round {n} of run {h4}: plan only the new work, in new stages \
         after stage {last}, then submit. Their request:\n{}",
        fence(request)
    )
}

/// Decision 10's reply to an accepted iterate.
pub fn round_started(h4: &str, n: u32) -> String {
    format!("run {h4} round {n} started; its orchestrator plans it")
}

/// Decision 10, step 1: a request over `GOAL_MAX_CHARS` characters.
pub const REQUEST_TOO_LONG: &str = "the request is longer than its 16,384-character limit";

/// Decision 9, step 2: a run with no orchestrator record.
pub fn no_orchestrator(h4: &str) -> String {
    format!("run {h4} has no orchestrator; start a new goal for more work")
}

/// KG §2.1: `ROUNDS_MAX` reached.
pub fn rounds_max(h4: &str) -> String {
    format!(
        "run {h4} has had {} rounds; accept or discard it and start a new goal",
        proto::ROUNDS_MAX
    )
}

/// KG §2.3: a halted run.
pub fn halted(h4: &str) -> String {
    format!("run {h4} is halted; resume or cancel it first")
}

/// Decision 9, step 5: an accepted, discarded or failed run (`label` its state's).
pub fn ended(h4: &str, label: &str) -> String {
    format!("run {h4} is {label}; start a new goal for more work")
}

/// D7: a cancelled `pr` run, complete with its PRs open.
pub fn cancelled(h4: &str) -> String {
    format!("run {h4} was cancelled; start a new goal for more work")
}

/// KG §2.3: any other state.
pub fn not_settled(h4: &str, label: &str) -> String {
    format!("run {h4} is {label}; iterate it when it completes")
}

/// KG §2.4: an edit of stage `s` (or of a task in it), which round `r` holds.
pub fn earlier_round(s: u16, r: u32) -> String {
    format!("stage {s} belongs to round {r}, which is done; put new work in a new stage")
}

/// Decision 30: `edit_plan`'s `iterate` with an edit, `submit` or `summary`.
pub const ITERATE_ALONE: &str = "iterate must be the only thing in its edit_plan call";
/// Decision 30: an `iterate` edit in a sub-planner's `submit_epic`.
pub const ITERATE_BY_PLANNER: &str = "a sub-planner cannot iterate a run";
/// Decision 30: an `iterate` edit in the user's `run edit`.
pub const ITERATE_BY_USER: &str = "use anthrex run iterate to start a round";
/// Decision 30: an `iterate` edit in the orchestrator's `edits` array.
pub const ITERATE_IN_EDITS: &str = "iterate goes in edit_plan's iterate, alone in its call";

/// Decision 32: the action menu's effect line for `iterate`, round `n` being the next.
pub fn iterate_effect(n: u32) -> String {
    format!("plan round {n} of this run with its orchestrator")
}

/// Decision 12: the reply to a rejected round `n`.
pub fn round_rejected(h4: &str, n: u32) -> String {
    format!("run {h4} round {n} rejected; the earlier rounds are unchanged")
}

/// Decision 16: the reply to a cancelled round `n`.
pub fn round_cancelled(h4: &str, n: u32) -> String {
    format!("run {h4} round {n} cancelled; it ends once its sessions have ended")
}

/// Decision 17: the note at the end of round `n` of a `pr` run, which keeps delivering.
pub fn round_done(n: u32) -> String {
    format!("round {n} is done; write its summary with edit_plan summary")
}
