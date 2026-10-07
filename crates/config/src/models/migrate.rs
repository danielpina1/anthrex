//! Milestone 9.8 decisions 14–18 (MR §6.2): the old model keys migrated into the role
//! table, once, at load. A pure port of the resolution the daemon applied to those keys
//! before the role table: private copies of the roster helpers it needs live here
//! because the daemon's go in M9.8.13 and old configs must keep migrating. The golden
//! test (`daemon/src/run/model_roles_golden.rs`) proves the port.
//!
//! A row is written where one of the old keys that feeds it is present in the file, or
//! where its model differs from the built-in's (decision 14), except a class
//! `default_runtime` has no model for, which keeps its built-in (decision 16, preflight
//! ruling F3). Each old key present gives one note (decision 18), in the order the
//! parsed `[orchestrator]` table lists its keys.

use proto::{DeciderMode, Effort, ModelEntry, Runtime, Strength};

use super::{BrainstormChoice, ModelRef, ModelTable, Role, RoleChoice};
use super::{builtin_brainstorm, builtin_choice};
use crate::{Orchestrator, RouteList, RouteLists};

/// What the `[[orchestrator.models]]` roster and `builtin_models` are replaced by.
const CLIS: &str = "the models your CLIs report (C-b S)";

/// Each sub-table's keys that fed a row, and the row.
const SCALARS: &[(&str, &[&str], &str)] = &[
    ("agent", &["runtime", "model", "effort"], "orchestrator"),
    ("planners", &["runtime", "strength", "effort"], "planner"),
    ("scouts", &["runtime", "strength", "effort"], "research"),
    // `mode` stays a live key (decision 17).
    ("deciders", &["strength", "effort"], "helpers"),
];

/// Each `[orchestrator.routes.<name>]` table and the row it is replaced by.
const ROUTES: &[(&str, &str)] = &[
    ("s", "implementer.small"),
    ("m", "implementer.medium"),
    ("hub", "implementer.hub"),
    ("review", "reviewer"),
    ("scout", "research"),
    ("decider", "helpers"),
    ("planner", "planner"),
    ("orchestrator", "orchestrator"),
    ("brainstorm", "brainstorm"),
];

/// A route as the old code resolved it: a roster entry (or a runtime's default) at an
/// effort.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Old {
    runtime: Runtime,
    /// Empty: the CLI's default.
    model: String,
    strength: Strength,
    effort: Effort,
}

impl Old {
    fn of(entry: &ModelEntry, effort: Effort) -> Old {
        Old {
            runtime: entry.runtime,
            model: entry.model.clone(),
            strength: entry.strength,
            effort,
        }
    }

    fn model_ref(&self) -> ModelRef {
        ModelRef {
            runtime: self.runtime,
            id: (!self.model.is_empty()).then(|| self.model.clone()),
        }
    }

    fn choice(&self, fallback: Option<&Old>) -> RoleChoice {
        RoleChoice {
            model: self.model_ref(),
            effort: Some(effort_name(self.effort)),
            fallback: fallback.map(Old::model_ref),
        }
    }
}

/// M9.8.4 replaces the match with `e.as_str().to_string()`.
fn effort_name(e: Effort) -> String {
    match e {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
    .to_string()
}

fn strength_name(s: Strength) -> &'static str {
    match s {
        Strength::Fast => "fast",
        Strength::Standard => "standard",
        Strength::Frontier => "frontier",
    }
}

// ---- private ports of the daemon's roster helpers (`run/roster.rs`, `scout/spec.rs`) ----

fn peer(runtime: Runtime) -> Runtime {
    match runtime {
        Runtime::Claude => Runtime::Codex,
        Runtime::Codex => Runtime::Claude,
        Runtime::Shell => Runtime::Shell,
    }
}

fn first_at(roster: &[ModelEntry], runtime: Runtime, strength: Strength) -> Option<&ModelEntry> {
    (roster.iter()).find(|e| e.runtime == runtime && e.strength == strength)
}

/// The entry on `runtime` with the lowest strength at or above `min`, first in roster
/// order among ties, skipping `exclude_model`.
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

/// The highest-strength entry on `runtime`, first in roster order among ties.
fn strongest_of(roster: &[ModelEntry], runtime: Runtime) -> Option<&ModelEntry> {
    let mut best: Option<&ModelEntry> = None;
    for entry in roster.iter().filter(|e| e.runtime == runtime) {
        best = match best {
            Some(current) if current.strength >= entry.strength => Some(current),
            _ => Some(entry),
        };
    }
    best
}

/// `scout::spec::route_within` with the peer allowed (every runtime counts as
/// installed at load).
fn route_within(
    roster: &[ModelEntry],
    runtime: Runtime,
    strength: Strength,
    effort: Effort,
) -> Old {
    let entry = lowest_at_or_above(roster, runtime, strength, None)
        .or_else(|| lowest_at_or_above(roster, peer(runtime), strength, None))
        .or_else(|| roster.iter().find(|e| e.runtime == runtime));
    match entry {
        Some(entry) => Old::of(entry, effort),
        None => Old {
            runtime,
            model: String::new(),
            strength,
            effort,
        },
    }
}

/// `roster::pick_reviewer` at the `Medium` level: the other runtime's lowest entry at
/// or above the author's strength; else the same runtime's, another model preferred;
/// else its strongest; else the author's route. At `medium`.
fn pick_reviewer(roster: &[ModelEntry], author: &Old) -> Old {
    let (required, effort) = (author.strength, Effort::Medium);
    lowest_at_or_above(roster, peer(author.runtime), required, None)
        .or_else(|| lowest_at_or_above(roster, author.runtime, required, Some(&author.model)))
        .or_else(|| strongest_of(roster, author.runtime))
        .map_or_else(
            || Old {
                effort,
                ..author.clone()
            },
            |e| Old::of(e, effort),
        )
}

/// A list's candidates that are in the roster, at their own effort or else `effort`.
fn candidates(list: &RouteList, roster: &[ModelEntry], effort: Effort) -> Vec<Old> {
    (list.candidates.iter())
        .filter_map(|c| {
            let entry = roster
                .iter()
                .find(|e| e.runtime == c.runtime && e.model == c.model)?;
            Some(Old::of(entry, c.effort.unwrap_or(effort)))
        })
        .collect()
}

/// MR §6.2: a list's first candidate in the roster as the model, the next as the
/// fallback. `None` with no usable candidate.
fn listed(list: &RouteList, roster: &[ModelEntry], effort: Effort) -> Option<(Old, Option<Old>)> {
    let mut found = candidates(list, roster, effort).into_iter();
    let first = found.next()?;
    Some((first, found.next()))
}

/// Which old keys are present in the raw `[orchestrator]` table.
struct Present<'a>(&'a toml::Table);

impl Present<'_> {
    fn key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    fn any_of(&self, table: &str, keys: &[&str]) -> bool {
        let sub = self.0.get(table).and_then(toml::Value::as_table);
        sub.is_some_and(|t| keys.iter().any(|k| t.contains_key(*k)))
    }

    fn route(&self, name: &str) -> bool {
        self.any_of("routes", &[name])
    }

    fn scalars(&self, table: &str) -> bool {
        let keys = SCALARS.iter().find(|(t, _, _)| *t == table).map(|s| s.1);
        self.any_of(table, keys.unwrap_or_default())
    }
}

/// Decision 14's rule: written when an old key feeds it or its model is not the
/// built-in's.
fn put(table: &mut ModelTable, role: Role, choice: RoleChoice, fed: bool) {
    if fed || choice.model != builtin_choice(role).model {
        table.rows.insert(role, choice);
    }
}

/// Decisions 14–18: the rows the old keys give and one note per old key present.
/// `raw` is the file's `[orchestrator]` table, `None` without one.
pub fn migrate(o: &Orchestrator, raw: Option<&toml::Table>) -> (ModelTable, Vec<String>) {
    let Some(raw) = raw else {
        return (ModelTable::default(), Vec::new());
    };
    let roster = &o.models[..];
    let present = Present(raw);
    // Preflight ruling F4: the lists are read here, from the raw table; their problems
    // were already reported by `tuning::read_tuning`.
    let lists = crate::orchestrator::read_routes(raw, roster, &mut Vec::new());
    let mut table = ModelTable::default();

    let orchestrator = orchestrator_row(o, &lists, roster);
    let fed = present.scalars("agent") || present.route("orchestrator");
    let (route, fallback) = &orchestrator;
    put(
        &mut table,
        Role::Orchestrator,
        route.choice(fallback.as_ref()),
        fed,
    );

    let p = &o.agent.planners;
    let (planner, fallback) = listed(&lists.planner, roster, p.effort).unwrap_or_else(|| {
        let runtime = p.runtime.unwrap_or(orchestrator.0.runtime);
        (route_within(roster, runtime, p.strength, p.effort), None)
    });
    let fed = present.scalars("planners") || present.route("planner");
    put(
        &mut table,
        Role::Planner,
        planner.choice(fallback.as_ref()),
        fed,
    );

    let (medium, unresolved) = implementer_rows(o, &lists, roster, &present, &mut table);
    if let Some(medium) = &medium {
        derived_rows(
            medium,
            &lists.review,
            roster,
            present.route("review"),
            &mut table,
        );
    }

    let s = &o.scouts;
    let (research, fallback) = listed(&lists.scout, roster, s.effort).unwrap_or_else(|| {
        let runtime = s.runtime.unwrap_or(o.default_runtime);
        (route_within(roster, runtime, s.strength, s.effort), None)
    });
    let fed = present.scalars("scouts") || present.route("scout");
    put(
        &mut table,
        Role::Research,
        research.choice(fallback.as_ref()),
        fed,
    );

    let brainstorm = brainstorm_row(&lists.brainstorm, roster, &orchestrator.0);
    let builtin = builtin_brainstorm();
    let differs = brainstorm.first != builtin.first || brainstorm.second != builtin.second;
    if present.route("brainstorm") || differs {
        table.brainstorm = Some(brainstorm);
    }

    let (helpers, fallback) = helpers_row(o, &lists, roster);
    let fed = present.scalars("deciders") || present.route("decider");
    put(
        &mut table,
        Role::Helpers,
        helpers.choice(fallback.as_ref()),
        fed,
    );

    (table, notes(raw, unresolved))
}

/// The orchestrator: its list, else `launch::resolve_orchestrator`'s rule
/// (`agent.runtime`, else `default_runtime`; `agent.model` when set, else the
/// runtime's strongest entry, a `frontier` one first; else the runtime's default).
fn orchestrator_row(
    o: &Orchestrator,
    lists: &RouteLists,
    roster: &[ModelEntry],
) -> (Old, Option<Old>) {
    let agent = &o.agent.agent;
    if let Some(found) = listed(&lists.orchestrator, roster, agent.effort) {
        return found;
    }
    let runtime = agent.runtime.unwrap_or(o.default_runtime);
    let entry = match agent.model.as_str() {
        "" => strongest_of(roster, runtime),
        model => roster
            .iter()
            .find(|e| e.runtime == runtime && e.model == model),
    };
    let route = match entry {
        Some(entry) => Old::of(entry, agent.effort),
        // A named model outside the roster is refused at a start today; it is kept as
        // named, which `run start` then checks against the CLI.
        None => Old {
            runtime,
            model: agent.model.clone(),
            strength: Strength::Standard,
            effort: agent.effort,
        },
    };
    (route, None)
}

/// The three implementer rows: each class's list, else `default_runtime`'s first
/// entry at the class's strength and effort; else no row and decision 16's note.
/// Returns the medium route (for the derived rows) and the notes.
fn implementer_rows(
    o: &Orchestrator,
    lists: &RouteLists,
    roster: &[ModelEntry],
    present: &Present,
    table: &mut ModelTable,
) -> (Option<Old>, Vec<String>) {
    let classes = [
        (
            Role::ImplementerSmall,
            &lists.s,
            "s",
            Strength::Standard,
            Effort::Low,
        ),
        (
            Role::ImplementerMedium,
            &lists.m,
            "m",
            Strength::Standard,
            Effort::Medium,
        ),
        (
            Role::ImplementerHub,
            &lists.hub,
            "hub",
            Strength::Frontier,
            Effort::High,
        ),
    ];
    let mut medium = None;
    let mut unresolved = Vec::new();
    for (role, list, name, strength, effort) in classes {
        let runtime = o.default_runtime;
        let found = listed(list, roster, effort)
            .or_else(|| first_at(roster, runtime, strength).map(|e| (Old::of(e, effort), None)));
        let Some((route, fallback)) = found else {
            let (rt, s) = (runtime.label(), strength_name(strength));
            let keeps = builtin_choice(role).model.label();
            unresolved.push(format!(
                "config: orchestrator.default_runtime = \"{rt}\" has no {rt} model at {s} strength in the roster; {} keeps {keeps}",
                role.key()
            ));
            continue;
        };
        let fed = present.route(name) || present.key("default_runtime");
        put(table, role, route.choice(fallback.as_ref()), fed);
        if role == Role::ImplementerMedium {
            medium = Some(route);
        }
    }
    (medium, unresolved)
}

/// Decision 15: the test writer and the reviewer of a medium task. The writer: the
/// peer runtime's first entry at the medium route's strength and effort, else the
/// medium route. The reviewer: the `review` list's first candidate on the other
/// runtime at or above the author's strength (the next listed one its fallback), else
/// `pick_reviewer` at `Medium`.
fn derived_rows(
    medium: &Old,
    review: &RouteList,
    roster: &[ModelEntry],
    fed: bool,
    table: &mut ModelTable,
) {
    let writer = first_at(roster, peer(medium.runtime), medium.strength)
        .map_or_else(|| medium.clone(), |e| Old::of(e, medium.effort));
    put(table, Role::TestWriter, writer.choice(None), false);

    let listed = candidates(review, roster, Effort::Medium);
    let qualifies = |c: &Old| c.runtime != medium.runtime && c.strength >= medium.strength;
    let (reviewer, fallback) = match listed.iter().position(qualifies) {
        Some(k) => {
            let next = (listed.iter().enumerate())
                .find(|(j, _)| *j != k)
                .map(|(_, c)| c);
            (listed[k].clone(), next.cloned())
        }
        None => (pick_reviewer(roster, medium), None),
    };
    put(
        table,
        Role::Reviewer,
        reviewer.choice(fallback.as_ref()),
        fed,
    );
}

/// The brainstormers (9.6 decision 10): the list's first two on different runtimes,
/// else its first two, one entry twice; else the strongest of Claude then Codex, one
/// twice, none the orchestrator's twice. At the first's effort (the orchestrator's
/// where a candidate names none).
fn brainstorm_row(list: &RouteList, roster: &[ModelEntry], orchestrator: &Old) -> BrainstormChoice {
    let effort = orchestrator.effort;
    let listed = candidates(list, roster, effort);
    let (first, second) = match listed.first() {
        Some(first) => {
            let other = listed.iter().skip(1).find(|r| r.runtime != first.runtime);
            (
                first.clone(),
                other.or(listed.get(1)).unwrap_or(first).clone(),
            )
        }
        None => {
            let strongest: Vec<Old> = [Runtime::Claude, Runtime::Codex]
                .into_iter()
                .filter_map(|runtime| strongest_of(roster, runtime))
                .map(|e| Old::of(e, effort))
                .collect();
            match &strongest[..] {
                [a, b, ..] => (a.clone(), b.clone()),
                [a] => (a.clone(), a.clone()),
                [] => (orchestrator.clone(), orchestrator.clone()),
            }
        }
    };
    BrainstormChoice {
        first: first.model_ref(),
        second: second.model_ref(),
        effort: Some(effort_name(first.effort)),
    }
}

/// The helpers: the `decider` list, else the mode's runtime's lowest entry at or above
/// `deciders.strength`, else that runtime's first entry, else its default
/// (`decider::call::ladder_route`).
fn helpers_row(o: &Orchestrator, lists: &RouteLists, roster: &[ModelEntry]) -> (Old, Option<Old>) {
    let d = &o.deciders;
    if let Some(found) = listed(&lists.decider, roster, d.effort) {
        return found;
    }
    let runtime = match d.mode {
        DeciderMode::Codex => Runtime::Codex,
        _ => Runtime::Claude,
    };
    let entry = lowest_at_or_above(roster, runtime, d.strength, None)
        .or_else(|| roster.iter().find(|e| e.runtime == runtime));
    let route = entry.map_or_else(
        || Old {
            runtime,
            model: String::new(),
            strength: d.strength,
            effort: d.effort,
        },
        |e| Old::of(e, d.effort),
    );
    (route, None)
}

/// Decision 18's notes, in the parsed table's key order; decision 16's after
/// `default_runtime`'s (or last, without one).
fn notes(raw: &toml::Table, mut unresolved: Vec<String>) -> Vec<String> {
    let note = |key: &str, row: &str| {
        format!("config: {key} is replaced by {row}; it is migrated until you save in C-b S")
    };
    let models = |row: &str| format!("[models.{row}]");
    let mut out = Vec::new();
    for (key, value) in raw {
        match key.as_str() {
            "models" => out.push(note("[[orchestrator.models]]", CLIS)),
            "builtin_models" => out.push(note("orchestrator.builtin_models", CLIS)),
            "default_runtime" => {
                let rows = "[models.implementer.small], [models.implementer.medium] and [models.implementer.hub]";
                out.push(note("orchestrator.default_runtime", rows));
                out.append(&mut unresolved);
            }
            "routes" => {
                for name in value.as_table().into_iter().flat_map(|t| t.keys()) {
                    if let Some((_, row)) = ROUTES.iter().find(|(n, _)| n == name) {
                        out.push(note(&format!("[orchestrator.routes.{name}]"), &models(row)));
                    }
                }
            }
            table => {
                let Some((_, keys, row)) = SCALARS.iter().find(|(t, _, _)| *t == table) else {
                    continue;
                };
                for sub in value.as_table().into_iter().flat_map(|t| t.keys()) {
                    if keys.contains(&sub.as_str()) {
                        out.push(note(&format!("orchestrator.{table}.{sub}"), &models(row)));
                    }
                }
            }
        }
    }
    out.append(&mut unresolved);
    out
}
