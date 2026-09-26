//! A scout's lifecycle (milestone 8b decision 14): a pure machine that `ScoutService`
//! drives with the scout's session events and a clock, and whose effects it executes.

use std::time::Duration;

use proto::{ScoutState, TokenUsage};

use super::contract::{SCOUT_NUDGE, scout_wrap_up};
use crate::run::driver::{INTERRUPT_GRACE, RETIRE_AFTER};

/// `[orchestrator.scouts]`'s limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoutLimits {
    pub timeout_secs: u64,
    pub max_tool_calls: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoutMachine {
    pub state: ScoutState,
    pub started_at: u64,
    pub turns_without_report: u8,
    pub tool_calls: u32,
    pub wrap_up_sent: bool,
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
    let effects = match event {
        ScoutEvent::Start { now } => {
            machine.state = ScoutState::Working;
            machine.started_at = now;
            Vec::new()
        }
        ScoutEvent::TurnEnded { .. } => {
            machine.turns_without_report = machine.turns_without_report.saturating_add(1);
            if machine.turns_without_report == 1 {
                vec![ScoutEffect::Send(SCOUT_NUDGE.to_string())]
            } else {
                fail(
                    &mut machine,
                    "the scout ended two turns without a report".into(),
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
                    format!("the scout used {n} tool calls without a report"),
                    true,
                )
            } else if n >= limits.max_tool_calls && !machine.wrap_up_sent {
                machine.wrap_up_sent = true;
                vec![ScoutEffect::Send(scout_wrap_up(n))]
            } else {
                Vec::new()
            }
        }
        ScoutEvent::Exited { code } => {
            let code = code.map_or("unknown".to_string(), |c| c.to_string());
            fail(
                &mut machine,
                format!("the scout's process exited without a report (code {code})"),
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
                    format!("the scout ran longer than {limit} s"),
                    true,
                )
            } else {
                Vec::new()
            }
        }
        ScoutEvent::Stop => fail(&mut machine, "stopped by the user".into(), true),
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
