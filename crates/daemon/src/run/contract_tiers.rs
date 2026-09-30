//! Milestone 9.1's worker-facing texts (task M9.1.13): the tier-0 block of a tiered
//! profile's worker prompt (decision 13, Interfaces "Prompts (exact)") and the tier
//! line of a tier job's bounce. Pure, like `contract.rs`, which calls them. An untiered
//! profile gets neither, so M8a's texts are byte-identical (decision 6).

use proto::ModuleNames;

use crate::run::globs::path_module;
use crate::run::model::{CheckRecord, Run, Task};
use crate::run::tiers::Scope;
use crate::run::tiers::command::{Placeholders, filter_expr, substitute};

/// The modules task `task`'s `owns` fall in (decision 10 on each glob's literal
/// prefix), by name, sorted. Names are known here only with `module_names = "dir"`;
/// `cargo` names come from the module graph, which the engine does not hold.
fn task_modules(run: &Run, task: &Task) -> Vec<String> {
    if run.profile.tiers.module_names != ModuleNames::Dir {
        return Vec::new();
    }
    let mut names: Vec<String> = task
        .spec
        .owns
        .iter()
        .filter_map(|glob| path_module(glob, &run.profile.modules))
        .filter_map(|dir| dir.rsplit('/').next().map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Decision 13's lines after `Check command:`: none for an untiered profile or one
/// with no module command.
pub(super) fn tier0_lines(run: &Run, task: &Task) -> Vec<String> {
    let tiers = &run.profile.tiers;
    if !tiers.is_tiered() {
        return Vec::new();
    }
    let names = task_modules(run, task);
    let filter = filter_expr(tiers, Scope::Gate, false);
    let command = match (&tiers.module_test, &tiers.module_tests) {
        (Some(one), _) => substitute(
            one,
            &Placeholders {
                filter,
                ..Placeholders::default()
            },
        ),
        (None, Some(many)) => substitute(
            many,
            &Placeholders {
                modules: (!names.is_empty()).then(|| names.clone()),
                filter,
                ..Placeholders::default()
            },
        ),
        (None, None) => return Vec::new(),
    };
    let mut lines = vec![format!("Module test command: {command}")];
    if !names.is_empty() {
        lines.push(format!(
            "Your modules: {} (put one in place of {{module}})",
            names.join(", ")
        ));
    }
    lines.push(
        "While you work, run your own test and these module tests only; after task_done the engine runs the wider suites."
            .to_string(),
    );
    lines
}

/// The tier line of a tier job's bounce (task M9.1.13): `\ntier <n>: <affected>`, put
/// after the bounce's first line; empty for M8a's check.
pub(super) fn tier_line(c: &CheckRecord) -> String {
    c.tier
        .as_ref()
        .map(|t| format!("\ntier {}: {}", t.tier, t.affected))
        .unwrap_or_default()
}
