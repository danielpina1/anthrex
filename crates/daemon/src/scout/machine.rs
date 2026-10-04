//! A scout's lifecycle (milestone 8b decision 14): a pure machine that `ScoutService`
//! drives with the scout's session events and a clock, and whose effects it executes.

use std::collections::HashSet;
use std::time::Duration;

use proto::{Runtime, ScoutState, TokenUsage};

use super::contract::{SCOUT_NUDGE, scout_wrap_up};
use super::spec::SUBMIT_TOOL;
use crate::headless::SessionEvent;
use crate::run::driver::{INTERRUPT_GRACE, RETIRE_AFTER};
use crate::run::orch::contract::{PLANNER_NUDGE, planner_wrap_up};

/// `[orchestrator.scouts]`'s limits, or `[orchestrator.planners]`' for a sub-planner
/// (milestone 9 decision 31), with the texts of the session's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoutLimits {
    pub timeout_secs: u64,
    pub max_tool_calls: u32,
    /// Whether the runtime takes a message while a turn runs: Claude reads it from
    /// stdin, Codex's `headless_send` refuses an open turn. Without it the wrap-up waits
    /// for the turn's end (ruling I1).
    pub send_mid_turn: bool,
    /// Whether a turn that ends without the submission gets the nudge, a resumed turn.
    /// A Codex design agent's does not (milestone 9.6 ruling T8-7): its session is never
    /// resumed, so the turn's end fails it as [`unsubmitted`], for the engine to relaunch.
    pub nudges: bool,
    pub texts: MachineTexts,
}

impl ScoutLimits {
    /// `[orchestrator.scouts]`'s limits for a scout on `runtime`.
    pub fn new(scouts: &config::Scouts, runtime: Runtime) -> Self {
        ScoutLimits {
            timeout_secs: scouts.timeout_secs,
            max_tool_calls: scouts.max_tool_calls,
            send_mid_turn: runtime == Runtime::Claude,
            nudges: true,
            texts: SCOUT_TEXTS,
        }
    }
}

/// What the machine says, and which tool is the session's submission (milestone 9
/// decision 31): a scout's (M8b's texts) or a sub-planner's. `missing` names what a
/// failed session never delivered; the brief's four fields gain it, since the failure
/// texts differ in it (`a report`, `an accepted epic`).
#[derive(Debug, Clone, Copy)]
pub struct MachineTexts {
    pub nudge: &'static str,
    pub wrap_up: fn(u32) -> String,
    /// The submission tool as Claude names it: it does not count toward the budget.
    pub submit_tool: &'static str,
    /// `the scout`, `the sub-planner`.
    pub noun: &'static str,
    pub missing: &'static str,
}

/// Two texts are the same role's: compared by their strings, since a function
/// pointer's address is not a stable identity.
impl PartialEq for MachineTexts {
    fn eq(&self, other: &Self) -> bool {
        (self.nudge, self.submit_tool, self.noun, self.missing)
            == (other.nudge, other.submit_tool, other.noun, other.missing)
    }
}

impl Eq for MachineTexts {}

/// M8b's scout texts.
pub const SCOUT_TEXTS: MachineTexts = MachineTexts {
    nudge: SCOUT_NUDGE,
    wrap_up: scout_wrap_up,
    submit_tool: SUBMIT_TOOL,
    noun: "the scout",
    missing: "a report",
};

/// A sub-planner's texts (decision 31).
pub const PLANNER_TEXTS: MachineTexts = MachineTexts {
    nudge: PLANNER_NUDGE,
    wrap_up: planner_wrap_up,
    submit_tool: "mcp__anthrex__submit_epic",
    noun: "the sub-planner",
    missing: "an accepted epic",
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoutMachine {
    pub state: ScoutState,
    pub started_at: u64,
    pub turns_without_report: u8,
    pub tool_calls: u32,
    pub wrap_up_sent: bool,
    /// The wrap-up is owed at the next turn end (a runtime with no mid-turn message).
    pub wrap_up_pending: bool,
    pub usage: TokenUsage,
    pub failure: Option<String>,
}

impl Default for ScoutMachine {
    fn default() -> Self {
        ScoutMachine {
            state: ScoutState::Starting,
            started_at: 0,
            turns_without_report: 0,
            tool_calls: 0,
            wrap_up_sent: false,
            wrap_up_pending: false,
            usage: TokenUsage::default(),
            failure: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoutEvent {
    Start {
        now: u64,
    },
    TurnEnded {
        usage: Option<TokenUsage>,
    },
    ToolUse,
    Exited {
        code: Option<i32>,
    },
    ReportAccepted,
    Tick {
        now: u64,
    },
    /// `ScoutService::stop`: the user stopped the scout.
    Stop,
    /// `ScoutService::stop_planner`: the engine stopped a sub-planner, for `reason`
    /// (milestone 9 decision 22).
    Halt {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoutEffect {
    /// A new turn with this text (`headless_send`).
    Send(String),
    /// `headless_kill`.
    Kill,
    /// `headless_retire`.
    CloseStdin,
    /// `headless_kill` after this long, if the process still runs.
    KillAfter(Duration),
    /// `WindowManager::remove` after this long.
    RemoveAfter(Duration),
    Finished(Result<(), String>),
}

/// Ruling T8-7: the failure of a session that ends a turn without its submission and
/// is not nudged (`ScoutLimits.nudges` false).
pub fn unsubmitted(t: &MachineTexts) -> String {
    format!("{} ended its turn without {}", t.noun, t.missing)
}

/// One step of scout `machine` on `event`.
pub fn step(
    mut machine: ScoutMachine,
    event: ScoutEvent,
    limits: &ScoutLimits,
) -> (ScoutMachine, Vec<ScoutEffect>) {
    if let ScoutEvent::TurnEnded { usage: Some(usage) } = &event {
        add(&mut machine.usage, usage);
    }
    let live = matches!(machine.state, ScoutState::Starting | ScoutState::Working);
    if !live {
        return (machine, Vec::new());
    }
    let t = limits.texts;
    let effects = match event {
        ScoutEvent::Start { now } => {
            machine.state = ScoutState::Working;
            machine.started_at = now;
            Vec::new()
        }
        ScoutEvent::TurnEnded { .. } if !limits.nudges => fail(&mut machine, unsubmitted(&t), true),
        ScoutEvent::TurnEnded { .. } => {
            machine.turns_without_report = machine.turns_without_report.saturating_add(1);
            if machine.turns_without_report == 1 {
                // An owed wrap-up goes in the nudge's place (ruling I1).
                let text = if std::mem::take(&mut machine.wrap_up_pending) {
                    (t.wrap_up)(machine.tool_calls)
                } else {
                    t.nudge.to_string()
                };
                vec![ScoutEffect::Send(text)]
            } else {
                fail(
                    &mut machine,
                    format!("{} ended two turns without {}", t.noun, t.missing),
                    true,
                )
            }
        }
        ScoutEvent::ToolUse => {
            machine.tool_calls += 1;
            let n = machine.tool_calls;
            if u64::from(n) * 2 >= u64::from(limits.max_tool_calls) * 3 {
                fail(
                    &mut machine,
                    format!("{} used {n} tool calls without {}", t.noun, t.missing),
                    true,
                )
            } else if n >= limits.max_tool_calls && !machine.wrap_up_sent {
                machine.wrap_up_sent = true;
                if limits.send_mid_turn {
                    vec![ScoutEffect::Send((t.wrap_up)(n))]
                } else {
                    machine.wrap_up_pending = true;
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        }
        ScoutEvent::Exited { code } => {
            let code = code.map_or("unknown".to_string(), |c| c.to_string());
            fail(
                &mut machine,
                format!(
                    "{}'s process exited without {} (code {code})",
                    t.noun, t.missing
                ),
                false,
            )
        }
        ScoutEvent::ReportAccepted => {
            machine.state = ScoutState::Reported;
            vec![
                ScoutEffect::Finished(Ok(())),
                ScoutEffect::CloseStdin,
                ScoutEffect::KillAfter(INTERRUPT_GRACE),
                ScoutEffect::RemoveAfter(RETIRE_AFTER),
            ]
        }
        ScoutEvent::Tick { now } => {
            let limit = limits.timeout_secs;
            if machine.state == ScoutState::Working && now >= machine.started_at + limit {
                fail(
                    &mut machine,
                    format!("{} ran longer than {limit} s", t.noun),
                    true,
                )
            } else {
                Vec::new()
            }
        }
        ScoutEvent::Stop => fail(&mut machine, "stopped by the user".into(), true),
        ScoutEvent::Halt { reason } => fail(&mut machine, reason, true),
    };
    (machine, effects)
}

/// Fails the scout: a kill when its process may still run, the outcome, and the
/// window's removal after decision 52's `RETIRE_AFTER`.
fn fail(machine: &mut ScoutMachine, reason: String, kill: bool) -> Vec<ScoutEffect> {
    machine.state = ScoutState::Failed;
    machine.failure = Some(reason.clone());
    let mut effects = Vec::new();
    if kill {
        effects.push(ScoutEffect::Kill);
    }
    effects.push(ScoutEffect::Finished(Err(reason)));
    effects.push(ScoutEffect::RemoveAfter(RETIRE_AFTER));
    effects
}

fn add(total: &mut TokenUsage, usage: &TokenUsage) {
    total.input += usage.input;
    total.output += usage.output;
    total.cache_read += usage.cache_read;
    total.cache_write += usage.cache_write;
}

/// The machine's event for one session event of a scout's process `pid`, or `None`
/// for one it does not act on. `turn_ended` records the processes in which a turn
/// ended: a Codex process exits after each turn, so its exit then is the turn's end, not
/// the session's (M8a's T17-I1). The submission call itself (`texts.submit_tool`) does
/// not count toward the tool budget (ruling M2).
pub fn scout_event(
    event: &SessionEvent,
    pid: Option<u32>,
    runtime: Runtime,
    turn_ended: &mut HashSet<u32>,
    texts: &MachineTexts,
) -> Option<ScoutEvent> {
    match event {
        SessionEvent::TurnEnded { usage, .. } => {
            turn_ended.extend(pid);
            Some(ScoutEvent::TurnEnded { usage: *usage })
        }
        SessionEvent::ToolUse { name, .. } if name == texts.submit_tool => None,
        SessionEvent::ToolUse { .. } => Some(ScoutEvent::ToolUse),
        SessionEvent::ProcessExited { code, .. } => {
            let turn_over = pid.is_some_and(|pid| turn_ended.contains(&pid));
            if runtime == Runtime::Codex && turn_over {
                return None;
            }
            Some(ScoutEvent::Exited { code: *code })
        }
        _ => None,
    }
}
