//! Decision 27: the rules a settings save must pass, shared by the daemon (before it
//! writes) and the Settings screen (inline, before it sends). Pure. Milestone 9.8
//! (M9.8.12) replaced the roster's and the orchestrator default's rules with the role
//! table's.
//!
//! A limit's message is exactly the one `config::parse` gives for the same value
//! (`<key>: <message>`), since both read the ranges from `proto::settings`
//! (`the_screen_and_the_parser_agree_on_ranges`).

use std::ops::RangeInclusive;

use proto::models::{ModelRef, ModelTable, Role, valid_effort};
use proto::settings::{
    BUDGET_MIN, MAX_BOUNCES_RANGE, MAX_READERS_RANGE, MAX_WRITERS_RANGE, STALL_AFTER_SECS_RANGE,
    key,
};
use proto::{BudgetLimit, SettingsDoc};

/// Every reason `doc` cannot be saved, in a fixed order: the role table (rows in
/// `Role::all` order, then brainstorm), then the limits in `SETTINGS_KEYS` order. Empty
/// when it can.
pub fn validate(doc: &SettingsDoc) -> Vec<String> {
    let doc = &cleaned(doc);
    let mut problems = Vec::new();
    roles(&doc.roles, &mut problems);
    limits(doc, &mut problems);
    problems
}

/// `doc` as a save writes it (M9.2.6 fix round 2). Since milestone 9.8 the document
/// holds no free text (a `ModelRef` cannot hold a hidden format character, decision
/// 2), so this is `doc` itself; kept as the one place a save's cleaning would go.
pub fn cleaned(doc: &SettingsDoc) -> SettingsDoc {
    doc.clone()
}

/// `text` without its hidden format characters (`proto::safe_text::is_hidden_format`).
pub fn strip_hidden(text: &str) -> String {
    text.chars()
        .filter(|c| !proto::safe_text::is_hidden_format(*c))
        .collect()
}

/// Milestone 9.8 (M9.8.12): each row's model and fallback must be a model `ModelRef`
/// parses back to itself, and its effort an effort name; the same for the brainstorm
/// pair. The message is the parser's (`models.<row>.<key>: …`).
fn roles(table: &ModelTable, problems: &mut Vec<String>) {
    let model = |path: &str, m: &ModelRef, problems: &mut Vec<String>| {
        if let Err(e) = ModelRef::parse(&m.label()).and_then(|p| {
            (p == *m)
                .then_some(())
                .ok_or_else(|| format!("{:?} is not <runtime>:<model>", m.label()))
        }) {
            problems.push(format!("{path}: {e}"));
        }
    };
    let effort = |path: &str, e: &Option<String>, problems: &mut Vec<String>| {
        if let Some(e) = e.as_deref().filter(|e| !valid_effort(e)) {
            problems.push(format!(
                "{path}: {e:?} is not an effort name (1 to 16 of a-z, 0-9, _ and -)"
            ));
        }
    };
    for role in Role::all() {
        let Some(c) = table.rows.get(&role) else {
            continue;
        };
        let row = format!("models.{}", role.key());
        model(&format!("{row}.model"), &c.model, problems);
        effort(&format!("{row}.effort"), &c.effort, problems);
        if let Some(f) = &c.fallback {
            model(&format!("{row}.fallback"), f, problems);
        }
    }
    if let Some(b) = &table.brainstorm {
        model("models.brainstorm.first", &b.first, problems);
        model("models.brainstorm.second", &b.second, problems);
        effort("models.brainstorm.effort", &b.effort, problems);
    }
}

fn limits(doc: &SettingsDoc, problems: &mut Vec<String>) {
    let l = &doc.limits;
    budget(
        &l.budget_s,
        key::BUDGET_S_CALLS,
        key::BUDGET_S_MINUTES,
        problems,
    );
    budget(
        &l.budget_m,
        key::BUDGET_M_CALLS,
        key::BUDGET_M_MINUTES,
        problems,
    );
    budget(
        &l.budget_l,
        key::BUDGET_L_CALLS,
        key::BUDGET_L_MINUTES,
        problems,
    );
    in_range(
        key::STALL_AFTER_SECS,
        l.stall_after_secs,
        &STALL_AFTER_SECS_RANGE,
        problems,
    );
    in_range(
        key::MAX_WRITERS,
        l.max_writers,
        &MAX_WRITERS_RANGE,
        problems,
    );
    in_range(
        key::MAX_READERS,
        l.max_readers,
        &MAX_READERS_RANGE,
        problems,
    );
    in_range(
        key::MAX_BOUNCES,
        l.max_bounces,
        &MAX_BOUNCES_RANGE,
        problems,
    );
}

fn budget(b: &BudgetLimit, calls: &str, minutes: &str, problems: &mut Vec<String>) {
    for (key, value) in [(calls, b.tool_calls), (minutes, b.minutes)] {
        if value < BUDGET_MIN {
            problems.push(format!("{key}: must be at least {BUDGET_MIN}"));
        }
    }
}

fn in_range<T: PartialOrd + std::fmt::Display>(
    key: &str,
    value: T,
    range: &RangeInclusive<T>,
    problems: &mut Vec<String>,
) {
    if !range.contains(&value) {
        problems.push(format!(
            "{key}: must be between {} and {}",
            range.start(),
            range.end()
        ));
    }
}
