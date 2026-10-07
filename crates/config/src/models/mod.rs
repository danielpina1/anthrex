//! Milestone 9.8 (MR §3): the role table. Every role names one row; a row is a model,
//! an effort and an "if it struggles" fallback. The types live in `proto::models` (the
//! settings document carries them); this module adds the built-in rows, the resolution
//! order (repository file, then the global table, then the built-in), reading and
//! rendering a `[models]` table (`parse.rs`) and the repository file (`repo.rs`, the
//! only file here with I/O).

mod parse;
mod repo;

pub(crate) use parse::read_table;
pub use parse::{parse_text, render};
pub use proto::models::{
    BrainstormChoice, HelperKind, ModelRef, ModelTable, Role, RoleChoice, valid_effort,
};
pub use repo::{load_repo, save_repo};

/// The repository file's name, in the repository's data directory (MR §3.3).
pub const REPO_FILE: &str = "models.toml";

const OPUS: &str = "claude:claude-opus-5-5";
const SONNET: &str = "claude:claude-sonnet-5";
const HAIKU: &str = "claude:claude-haiku-4-5";
const CODEX: &str = "codex:default";

fn row(model: &str, effort: Option<&str>, fallback: Option<&str>) -> RoleChoice {
    let model_ref = |m: &str| ModelRef::parse(m).expect("a built-in model parses");
    RoleChoice {
        model: model_ref(model),
        effort: effort.map(str::to_string),
        fallback: fallback.map(model_ref),
    }
}

/// MR §6.1's table, with `research` corrected to today's scouts (decision 8). A helper
/// kind's built-in is the `helpers` row's.
pub fn builtin_choice(role: Role) -> RoleChoice {
    match role {
        Role::Orchestrator | Role::Planner | Role::ImplementerHub => row(OPUS, Some("high"), None),
        Role::ImplementerSmall => row(SONNET, Some("low"), None),
        Role::ImplementerMedium => row(SONNET, Some("medium"), None),
        Role::TestWriter => row(CODEX, Some("medium"), None),
        Role::Reviewer => row(CODEX, Some("high"), Some(OPUS)),
        Role::Research => row(HAIKU, Some("low"), None),
        Role::Helpers | Role::Helper(_) => row(HAIKU, None, None),
    }
}

/// Opus and the Codex default, at `high`.
pub fn builtin_brainstorm() -> BrainstormChoice {
    BrainstormChoice {
        first: ModelRef::parse(OPUS).expect("a built-in model parses"),
        second: ModelRef::parse(CODEX).expect("a built-in model parses"),
        effort: Some("high".to_string()),
    }
}

/// Decision 7 (MR §3.4): the repository's row, else the global row, else the built-in.
/// A helper kind takes the repository's kind, the global kind, the repository's
/// `helpers`, the global `helpers`, then the built-in. A row is taken whole.
pub fn resolve(role: Role, repo: Option<&ModelTable>, global: &ModelTable) -> RoleChoice {
    let row = |t: Option<&ModelTable>, r: Role| t.and_then(|t| t.rows.get(&r)).cloned();
    let found = match role {
        Role::Helper(_) => row(repo, role)
            .or_else(|| row(Some(global), role))
            .or_else(|| row(repo, Role::Helpers))
            .or_else(|| row(Some(global), Role::Helpers)),
        _ => row(repo, role).or_else(|| row(Some(global), role)),
    };
    found.unwrap_or_else(|| builtin_choice(role))
}

/// The repository's brainstorm pair, else the global one, else the built-in.
pub fn resolve_brainstorm(repo: Option<&ModelTable>, global: &ModelTable) -> BrainstormChoice {
    repo.and_then(|t| t.brainstorm.clone())
        .or_else(|| global.brainstorm.clone())
        .unwrap_or_else(builtin_brainstorm)
}

#[cfg(test)]
mod tests;
