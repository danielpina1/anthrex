//! What the Settings screen owns in `config.toml` (milestone 9.0.6 decisions 23-32): the
//! document ([`doc_of`]), where each value came from ([`origin_of`]), the rules a save
//! must pass ([`validate`]), the line-based writer that keeps every other line
//! ([`edit_text`]), and the atomic save ([`save`]).
//!
//! Owned keys, decision 23 as milestone 9.8 left it: the role table (`[models]`, written
//! by `write_models.rs`, which also removes the old model keys it replaced, decision
//! 40), the S/M/L budgets' `tool_calls` and `minutes`, `stall_after_secs`,
//! `max_writers`, `max_readers` and `max_bounces`. Everything else in the file is the
//! user's and is never rewritten.

use proto::{BudgetLimit, SettingsDoc, SettingsLimits};

use crate::Orchestrator;

mod lines;
mod origin;
mod save;
mod scan;
mod shipped;
mod validate;
mod write;
mod write_models;
mod write_old;

pub use origin::{load_with_origin, origin_of};
pub use save::{Saved, save};
pub(crate) use save::{temp_path, write_temp};
pub use shipped::{SHIPPED_CLAUDE, SHIPPED_CODEX, ShippedModel};
pub use validate::{cleaned, strip_hidden, validate};
// Milestone 9.2 (M9.2.6 fix round 2's deferral, closed by M9.2.15): the writer takes a
// doc as given, so only [`save`], which cleans first, may call it.
pub use write::UNSUPPORTED_FORM;
pub(crate) use write::edit_text;

/// The settings `o` holds.
pub fn doc_of(o: &Orchestrator) -> SettingsDoc {
    let budget = |b: &proto::Budget| BudgetLimit {
        tool_calls: b.tool_calls,
        minutes: b.minutes,
    };
    SettingsDoc {
        limits: SettingsLimits {
            budget_s: budget(&o.budget_s),
            budget_m: budget(&o.budget_m),
            budget_l: budget(&o.budget_l),
            stall_after_secs: o.stall_after_secs,
            max_writers: o.max_writers,
            max_readers: o.max_readers,
            max_bounces: o.max_bounces,
        },
        // Ruling T18-2: read-only in the settings; a save never writes it.
        design_default: Some(o.design.default),
        // Milestone 9.8 (M9.8.12): the table the settings screen edits and a save writes.
        roles: o.roles.clone(),
    }
}

/// Copies only the settings-owned fields of `from` onto `live` (decision 29): every other
/// `[orchestrator]` key, budget `tokens` included, stays as `live` has it. The roster and
/// the orchestrator default are copied too: a save that removed their old keys
/// (decision 40) changes them in the file, and the live value follows the file.
pub fn apply_owned(live: &mut Orchestrator, from: &Orchestrator) {
    live.builtin_models = from.builtin_models;
    live.models = from.models.clone();
    live.agent.agent.runtime = from.agent.agent.runtime;
    live.agent.agent.model = from.agent.agent.model.clone();
    for (to, from) in [
        (&mut live.budget_s, &from.budget_s),
        (&mut live.budget_m, &from.budget_m),
        (&mut live.budget_l, &from.budget_l),
    ] {
        to.tool_calls = from.tool_calls;
        to.minutes = from.minutes;
    }
    live.stall_after_secs = from.stall_after_secs;
    live.max_writers = from.max_writers;
    live.max_readers = from.max_readers;
    live.max_bounces = from.max_bounces;
    // Milestone 9.8 decision 42 (preflight ruling F14): helpers read the live table.
    live.roles = from.roles.clone();
}

#[cfg(test)]
mod save_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod write_models_tests;
#[cfg(test)]
mod write_tests;
