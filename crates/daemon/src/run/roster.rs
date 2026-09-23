//! Reviewer and escalation roster policy, decisions 23, 35 (reviewer route) and 39
//! (rung-2 route). Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

use proto::{Effort, ModelEntry, Route, Runtime, Strength};

use super::model::ReviewLevel;

/// The roster's entry for one `(runtime, model)`, if any. *(Decision 23.)*
pub fn find<'a>(roster: &'a [ModelEntry], runtime: Runtime, model: &str) -> Option<&'a ModelEntry> {
    roster
        .iter()
        .find(|entry| entry.runtime == runtime && entry.model == model)
}

/// The first roster entry, in roster order, at exactly `strength` on `runtime`.
pub fn first_at(
    roster: &[ModelEntry],
    runtime: Runtime,
    strength: Strength,
) -> Option<&ModelEntry> {
    roster
        .iter()
        .find(|entry| entry.runtime == runtime && entry.strength == strength)
}

/// The other runtime: Claude and Codex swap; anything else (there is no third
/// orchestrated runtime today) maps to itself.
pub fn peer(runtime: Runtime) -> Runtime {
    match runtime {
        Runtime::Claude => Runtime::Codex,
        Runtime::Codex => Runtime::Claude,
        Runtime::Shell => Runtime::Shell,
    }
}

fn strength_one_up(strength: Strength) -> Option<Strength> {
    match strength {
        Strength::Fast => Some(Strength::Standard),
        Strength::Standard => Some(Strength::Frontier),
        Strength::Frontier => None,
    }
}

/// The roster entry on `runtime` with the lowest strength at or above `min`, first in
/// roster order among ties; `exclude_model`, when set, skips an entry with that exact
/// model (decision 35's "preferring a model different from the author's").
fn lowest_at_or_above<'a>(
    roster: &'a [ModelEntry],
    runtime: Runtime,
    min: Strength,
    exclude_model: Option<&str>,
) -> Option<&'a ModelEntry> {
    let mut best: Option<&ModelEntry> = None;
    for entry in roster {
        if entry.runtime != runtime || entry.strength < min {
            continue;
        }
        if exclude_model.is_some_and(|model| entry.model == model) {
            continue;
        }
        best = match best {
            Some(current) if current.strength <= entry.strength => Some(current),
            _ => Some(entry),
        };
    }
    best
}

/// The highest-strength roster entry on `runtime`, first in roster order among ties.
fn highest_strength(roster: &[ModelEntry], runtime: Runtime) -> Option<&ModelEntry> {
    let mut best: Option<&ModelEntry> = None;
    for entry in roster {
        if entry.runtime != runtime {
            continue;
        }
        best = match best {
            Some(current) if current.strength >= entry.strength => Some(current),
            _ => Some(entry),
        };
    }
    best
}

fn route_from(entry: &ModelEntry, effort: Effort) -> Route {
    Route {
        runtime: entry.runtime,
        model: entry.model.clone(),
        strength: entry.strength,
        effort,
    }
}

/// The reviewer's route for an authored round, decision 35. Required strength: `fast`
/// for `Small`, the author's own strength for `Medium`, `frontier` for `Frontier`.
/// Picked, in order: the other runtime's roster entry with the lowest strength at or
/// above the requirement, first in roster order; else the same runtime's, preferring a
/// model different from the author's; else the same runtime's highest-strength entry.
/// Effort follows the level: `low`, `medium`, `high`.
pub fn pick_reviewer(roster: &[ModelEntry], author: &Route, level: ReviewLevel) -> Route {
    let required = match level {
        ReviewLevel::Small => Strength::Fast,
        ReviewLevel::Medium => author.strength,
        ReviewLevel::Frontier => Strength::Frontier,
    };
    let effort = match level {
        ReviewLevel::Small => Effort::Low,
        ReviewLevel::Medium => Effort::Medium,
        ReviewLevel::Frontier => Effort::High,
    };

    let peer_runtime = peer(author.runtime);
    if let Some(entry) = lowest_at_or_above(roster, peer_runtime, required, None) {
        return route_from(entry, effort);
    }
    if let Some(entry) = lowest_at_or_above(roster, author.runtime, required, Some(&author.model)) {
        return route_from(entry, effort);
    }
    if let Some(entry) = highest_strength(roster, author.runtime) {
        return route_from(entry, effort);
    }
    Route {
        runtime: author.runtime,
        model: author.model.clone(),
        strength: author.strength,
        effort,
    }
}

/// The rung-2 route, decision 39: effort below `high` raises effort on the same runtime
/// and model; otherwise the peer runtime's first roster entry at the same strength, at
/// `high` effort; otherwise the same runtime's first entry one strength up, at `high`
/// effort; otherwise the route is unchanged.
pub fn escalate(roster: &[ModelEntry], route: &Route) -> Route {
    if let Some(effort) = route.effort.raised() {
        return Route {
            effort,
            ..route.clone()
        };
    }
    let peer_runtime = peer(route.runtime);
    if let Some(entry) = first_at(roster, peer_runtime, route.strength) {
        return route_from(entry, Effort::High);
    }
    if let Some(up) = strength_one_up(route.strength)
        && let Some(entry) = first_at(roster, route.runtime, up)
    {
        return route_from(entry, Effort::High);
    }
    route.clone()
}

#[cfg(test)]
#[path = "roster_tests.rs"]
mod tests;
