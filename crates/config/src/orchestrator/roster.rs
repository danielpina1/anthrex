//! `[[orchestrator.models]]` and the built-in roster (decision 23). Split out of
//! `orchestrator.rs` to keep that file under the 600-line rule and because the roster
//! is a responsibility of its own: the built-in list, in order, and the
//! replace-in-place-or-append merge a `[[orchestrator.models]]` entry does against it.
//! Milestone 9.8 (task M9.8.14): read only for migration (`models::migrate`,
//! `models::old_keys`); strength is gone everywhere else, so the roster's types are
//! this crate's own.

use proto::Runtime;

use super::Orchestrator;
use crate::Problem;

/// `orchestrator.models[i].note`'s length limit, in characters. The note is a short
/// annotation shown beside a roster entry, not a place for prose.
pub const MODEL_NOTE_MAX: usize = 80;

/// How capable an old roster entry, planner, scout or decider was told to be. Read only
/// by the migration of the old keys. Ordered, as the old rules compared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum LegacyStrength {
    Fast,
    Standard,
    Frontier,
}

impl LegacyStrength {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LegacyStrength::Fast => "fast",
            LegacyStrength::Standard => "standard",
            LegacyStrength::Frontier => "frontier",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<LegacyStrength> {
        match text {
            "fast" => Some(LegacyStrength::Fast),
            "standard" => Some(LegacyStrength::Standard),
            "frontier" => Some(LegacyStrength::Frontier),
            _ => None,
        }
    }
}

/// One `[[orchestrator.models]]` entry (or a built-in one), as migration reads it. Its
/// fields are this crate's: nothing outside it chooses a model from the roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyModel {
    pub(crate) runtime: Runtime,
    pub(crate) model: String,
    pub(crate) strength: LegacyStrength,
}

/// The built-in roster, decision 23's exact order: `claude`/`claude-haiku-4-5`/`fast`,
/// `claude`/`claude-sonnet-5`/`standard`, `claude`/`claude-opus-5-5`/`frontier`,
/// `codex`/`""`/`standard`. An empty Codex model means "use Codex's configured
/// default"; that is the only runtime an empty model is valid for.
pub(crate) fn default_roster() -> Vec<LegacyModel> {
    vec![
        LegacyModel {
            runtime: Runtime::Claude,
            model: "claude-haiku-4-5".to_string(),
            strength: LegacyStrength::Fast,
        },
        LegacyModel {
            runtime: Runtime::Claude,
            model: "claude-sonnet-5".to_string(),
            strength: LegacyStrength::Standard,
        },
        LegacyModel {
            runtime: Runtime::Claude,
            model: "claude-opus-5-5".to_string(),
            strength: LegacyStrength::Frontier,
        },
        LegacyModel {
            runtime: Runtime::Codex,
            model: String::new(),
            strength: LegacyStrength::Standard,
        },
    ]
}

/// Builds `Orchestrator.models` from `orchestrator.builtin_models` (already read) and
/// `[[orchestrator.models]]`. Starts from the built-in roster, or from nothing when
/// `builtin_models = false`; each entry replaces a built-in with the same
/// `(runtime, model)` in place, or is appended. An invalid entry is skipped with its
/// index as a [`Problem`]; an empty result falls back to the built-in roster, also
/// with a `Problem`.
pub(super) fn read_models(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    let mut roster: Vec<LegacyModel> = if o.builtin_models {
        default_roster()
    } else {
        Vec::new()
    };

    if let Some(value) = table.get("models") {
        match value.as_array() {
            Some(entries) => {
                for (i, entry) in entries.iter().enumerate() {
                    match parse_model_entry(entry) {
                        Ok(parsed) => {
                            match roster.iter().position(|m| {
                                m.runtime == parsed.runtime && m.model == parsed.model
                            }) {
                                Some(pos) => roster[pos] = parsed,
                                None => roster.push(parsed),
                            }
                        }
                        Err(message) => problems.push(Problem {
                            key: format!("orchestrator.models[{i}]"),
                            message,
                            default: crate::ENTRY_SKIPPED.to_string(),
                        }),
                    }
                }
            }
            None => problems.push(Problem {
                key: "orchestrator.models".to_string(),
                message: "expected an array of tables".to_string(),
                default: "the built-in roster".to_string(),
            }),
        }
    }

    if roster.is_empty() {
        problems.push(Problem {
            key: "orchestrator.models".to_string(),
            message: "no valid entries".to_string(),
            default: "the built-in roster".to_string(),
        });
        roster = default_roster();
    }

    o.models = roster;
}

fn parse_model_entry(value: &toml::Value) -> Result<LegacyModel, String> {
    let Some(table) = value.as_table() else {
        return Err("expected a table".to_string());
    };

    let runtime = match table.get("runtime").and_then(|v| v.as_str()) {
        Some("claude") => Runtime::Claude,
        Some("codex") => Runtime::Codex,
        Some(other) => return Err(format!("invalid runtime {other:?}")),
        None => return Err("runtime is required".to_string()),
    };
    let model = match table.get("model").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return Err("model is required".to_string()),
    };
    if model.is_empty() && runtime != Runtime::Codex {
        return Err("empty model is only allowed for codex".to_string());
    }
    let strength = match table.get("strength").and_then(|v| v.as_str()) {
        Some(text) => match LegacyStrength::parse(text) {
            Some(strength) => strength,
            None => return Err(format!("invalid strength {text:?}")),
        },
        None => return Err("strength is required".to_string()),
    };
    let note = match table.get("note").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => String::new(),
    };
    if note.chars().count() > MODEL_NOTE_MAX {
        return Err(format!("note must be at most {MODEL_NOTE_MAX} characters"));
    }

    Ok(LegacyModel {
        runtime,
        model,
        strength,
    })
}
