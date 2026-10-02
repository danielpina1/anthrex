//! Decision 27: the rules a settings save must pass, shared by the daemon (before it
//! writes) and the Settings screen (inline, before it sends). Pure.
//!
//! A limit's message is exactly the one `config::parse` gives for the same value
//! (`<key>: <message>`), since both read the ranges from `proto::settings`
//! (`the_screen_and_the_parser_agree_on_ranges`).

use std::ops::RangeInclusive;

use proto::settings::{
    BUDGET_MIN, MAX_BOUNCES_RANGE, MAX_READERS_RANGE, MAX_WRITERS_RANGE, STALL_AFTER_SECS_RANGE,
    key,
};
use proto::{BudgetLimit, Runtime, SettingsDoc, Strength};

use crate::orchestrator::MODEL_NOTE_MAX;

/// Every reason `doc` cannot be saved, in a fixed order: the roster, the orchestrator
/// default, then the limits in `SETTINGS_KEYS` order. Empty when it can. The rules
/// apply to what a save writes, [`cleaned`]`(doc)`: a hidden format character never
/// refuses a save (M9.2.6 fix round 2), it is dropped.
pub fn validate(doc: &SettingsDoc) -> Vec<String> {
    let doc = &cleaned(doc);
    let mut problems = Vec::new();
    roster(doc, &mut problems);
    orchestrator(doc, &mut problems);
    limits(doc, &mut problems);
    problems
}

/// `doc` as a save writes it (M9.2.6 fix round 2): every hidden format character
/// (`proto::safe_text::is_hidden_format`: a variation selector in `⚠️`, a soft hyphen,
/// a bidi control) dropped from each model's name and note and from the orchestrator
/// default's model, which names one of them. A control character is kept, for
/// [`validate`] to refuse.
pub fn cleaned(doc: &SettingsDoc) -> SettingsDoc {
    let strip = |text: &str| -> String {
        text.chars()
            .filter(|c| !proto::safe_text::is_hidden_format(*c))
            .collect()
    };
    let mut out = doc.clone();
    for m in &mut out.models {
        m.model = strip(&m.model);
        m.note = strip(&m.note);
    }
    out.orchestrator.model = strip(&out.orchestrator.model);
    out
}

/// Advice that never refuses a save: each strength with an enabled model on exactly one
/// runtime, since the cross-runtime reviewer then cannot be chosen for it.
pub fn warnings(doc: &SettingsDoc) -> Vec<String> {
    let mut out = Vec::new();
    for strength in [Strength::Fast, Strength::Standard, Strength::Frontier] {
        let on = |r: Runtime| {
            doc.models
                .iter()
                .any(|m| m.runtime == r && m.strength == strength)
        };
        let missing = match (on(Runtime::Claude), on(Runtime::Codex)) {
            (true, false) => Runtime::Codex,
            (false, true) => Runtime::Claude,
            _ => continue,
        };
        let s = strength_name(strength);
        out.push(format!(
            "no {missing} model is {s}: the cross-runtime reviewer cannot be chosen for {s} tasks"
        ));
    }
    out
}

fn roster(doc: &SettingsDoc, problems: &mut Vec<String>) {
    if doc.models.is_empty() {
        problems.push("enable at least one model".to_string());
        return;
    }
    if doc
        .models
        .iter()
        .any(|m| m.runtime == Runtime::Claude && m.model.is_empty())
    {
        problems.push("claude models need a name".to_string());
    }
    for (i, m) in doc.models.iter().enumerate() {
        let earlier = &doc.models[..i];
        let first = !earlier
            .iter()
            .any(|e| e.runtime == m.runtime && e.model == m.model);
        let again = doc.models[i + 1..]
            .iter()
            .any(|e| e.runtime == m.runtime && e.model == m.model);
        if first && again {
            problems.push(format!(
                "{} {} is listed twice",
                m.runtime,
                model_name(&m.model)
            ));
        }
    }
    for (n, m) in doc.models.iter().enumerate() {
        for (field, text) in [("name", &m.model), ("note", &m.note)] {
            if text.chars().any(unsafe_char) {
                problems.push(format!(
                    "model {} ({}): its {field} holds a control character",
                    n + 1,
                    m.runtime
                ));
            }
        }
    }
    for m in &doc.models {
        if m.note.chars().count() > MODEL_NOTE_MAX {
            problems.push(format!(
                "note of {}: at most {MODEL_NOTE_MAX} characters",
                model_name(&m.model)
            ));
        }
    }
}

fn orchestrator(doc: &SettingsDoc, problems: &mut Vec<String>) {
    let o = &doc.orchestrator;
    if o.model.is_empty() {
        return;
    }
    let Some(runtime) = o.runtime else {
        problems.push(format!("{}: choose a runtime first", key::AGENT_MODEL));
        return;
    };
    if !doc
        .models
        .iter()
        .any(|m| m.runtime == runtime && m.model == o.model)
    {
        problems.push(format!(
            "{}: {} is not an enabled {runtime} model",
            key::AGENT_MODEL,
            o.model
        ));
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

/// A character a config line or a screen row must not hold: a control character
/// (newline, tab, ESC, C1) or a line or paragraph separator. A hidden format character
/// is not refused: [`cleaned`] drops it before any rule runs.
fn unsafe_char(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

/// How a model is named in a message: Codex's empty model is its configured default.
pub(super) fn model_name(model: &str) -> &str {
    if model.is_empty() { "(default)" } else { model }
}

pub(super) fn strength_name(s: Strength) -> &'static str {
    match s {
        Strength::Fast => "fast",
        Strength::Standard => "standard",
        Strength::Frontier => "frontier",
    }
}
