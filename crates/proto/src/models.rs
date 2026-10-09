//! Milestone 9.8 (MR §3, §4.2): the role table and the discovered model catalogs.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::types::Runtime;

/// `ModelRef`'s `id`, at most this many characters (the MCP schema's old model bound).
pub const MODEL_ID_MAX_CHARS: usize = 100;
/// An effort name, at most this many characters of `[a-z0-9_-]`.
pub const EFFORT_MAX_CHARS: usize = 16;

/// A model as a row names it: `claude:<id>`, `codex:<id>`, or `<runtime>:default`
/// (`id: None`, whatever that CLI is configured to use).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelRef {
    pub runtime: Runtime,
    pub id: Option<String>,
}

impl ModelRef {
    pub fn default_of(runtime: Runtime) -> ModelRef {
        ModelRef { runtime, id: None }
    }

    /// `<runtime>:<id>` or `<runtime>:default`.
    pub fn label(&self) -> String {
        format!(
            "{}:{}",
            self.runtime.label(),
            self.id.as_deref().unwrap_or("default")
        )
    }

    /// The model as a route spells it: the id, or `""` for the CLI's default.
    pub fn route_model(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }

    pub fn parse(text: &str) -> Result<ModelRef, String> {
        let (runtime, id) = text
            .split_once(':')
            .ok_or_else(|| format!("{text:?} is not <runtime>:<model>"))?;
        let runtime = match runtime {
            "claude" => Runtime::Claude,
            "codex" => Runtime::Codex,
            other => return Err(format!("{other:?} is not claude or codex")),
        };
        if id == "default" {
            return Ok(ModelRef { runtime, id: None });
        }
        model_id_problem(id)?;
        Ok(ModelRef {
            runtime,
            id: Some(id.to_string()),
        })
    }
}

impl std::fmt::Display for ModelRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

impl Serialize for ModelRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.label())
    }
}

impl<'de> Deserialize<'de> for ModelRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        ModelRef::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// Whether `id` is a model name: 1 to [`MODEL_ID_MAX_CHARS`] visible characters, no
/// whitespace, control or hidden-format character, and (M9.8.13 fix round 1) no leading
/// `-`, which a CLI's `-m`/`--model` would read as an option.
pub fn valid_model_id(id: &str) -> bool {
    model_id_problem(id).is_ok()
}

/// Why `id` is not a model name ([`valid_model_id`]), as `ModelRef::parse` and a user's
/// route refuse it.
pub fn model_id_problem(id: &str) -> Result<(), String> {
    let bad =
        |c: char| c.is_whitespace() || c.is_control() || crate::safe_text::is_hidden_format(c);
    if id.is_empty() || id.chars().count() > MODEL_ID_MAX_CHARS || id.chars().any(bad) {
        return Err(format!(
            "{id:?} is not a model name (1 to {MODEL_ID_MAX_CHARS} visible characters, no spaces)"
        ));
    }
    if id.starts_with('-') {
        return Err(format!(
            "{id:?} is not a model name (it may not start with -)"
        ));
    }
    Ok(())
}

/// Whether `name` is an effort name (decision 5).
pub fn valid_effort(name: &str) -> bool {
    (1..=EFFORT_MAX_CHARS).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// One row (MR §3.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleChoice {
    pub model: ModelRef,
    /// `None`: the model's default effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// "If it struggles" (D2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<ModelRef>,
}

/// The brainstorm row: two models and one effort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrainstormChoice {
    pub first: ModelRef,
    pub second: ModelRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// The six one-shot calls (MR D5), in `DeciderKind::ALL`'s order but `run_name` first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperKind {
    RunName,
    Triage,
    SizeCheck,
    CheckSummary,
    BlockedReason,
    CiSummary,
}

impl HelperKind {
    pub const ALL: [HelperKind; 6] = [
        HelperKind::RunName,
        HelperKind::Triage,
        HelperKind::SizeCheck,
        HelperKind::CheckSummary,
        HelperKind::BlockedReason,
        HelperKind::CiSummary,
    ];

    /// `run_name`, `triage`, … (the TOML table and the `DeciderKind::label`).
    pub fn key(self) -> &'static str {
        match self {
            HelperKind::RunName => "run_name",
            HelperKind::Triage => "triage",
            HelperKind::SizeCheck => "size_check",
            HelperKind::CheckSummary => "check_summary",
            HelperKind::BlockedReason => "blocked_reason",
            HelperKind::CiSummary => "ci_summary",
        }
    }

    /// What the table shows: `run name`, `triage`, `size check`, `check summary`,
    /// `blocked reason`, `ci summary`.
    pub fn label(self) -> &'static str {
        match self {
            HelperKind::RunName => "run name",
            HelperKind::Triage => "triage",
            HelperKind::SizeCheck => "size check",
            HelperKind::CheckSummary => "check summary",
            HelperKind::BlockedReason => "blocked reason",
            HelperKind::CiSummary => "ci summary",
        }
    }
}

/// A row's name. `Helper(kind)` is a helpers override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    Orchestrator,
    Planner,
    ImplementerSmall,
    ImplementerMedium,
    ImplementerHub,
    TestWriter,
    Reviewer,
    Research,
    Helpers,
    Helper(HelperKind),
}

impl Role {
    /// The table's rows in MR §5.1's order (brainstorm is its own field).
    pub const ROWS: [Role; 9] = [
        Role::Orchestrator,
        Role::Planner,
        Role::ImplementerSmall,
        Role::ImplementerMedium,
        Role::ImplementerHub,
        Role::TestWriter,
        Role::Reviewer,
        Role::Research,
        Role::Helpers,
    ];

    /// `ROWS`, then every helper kind.
    pub fn all() -> impl Iterator<Item = Role> {
        Role::ROWS
            .into_iter()
            .chain(HelperKind::ALL.into_iter().map(Role::Helper))
    }

    /// `orchestrator`, `implementer.small`, `test_writer`, `helpers.run_name`, …
    pub fn key(self) -> String {
        match self {
            Role::Orchestrator => "orchestrator".into(),
            Role::Planner => "planner".into(),
            Role::ImplementerSmall => "implementer.small".into(),
            Role::ImplementerMedium => "implementer.medium".into(),
            Role::ImplementerHub => "implementer.hub".into(),
            Role::TestWriter => "test_writer".into(),
            Role::Reviewer => "reviewer".into(),
            Role::Research => "research".into(),
            Role::Helpers => "helpers".into(),
            Role::Helper(kind) => format!("helpers.{}", kind.key()),
        }
    }

    pub fn parse_key(key: &str) -> Option<Role> {
        Role::all().find(|r| r.key() == key)
    }

    /// MR §5.1's names: `orchestrator`, `planner`, `implementer · small` (`· medium`,
    /// `· hub`), `test writer`, `reviewer`, `research`, `helpers`, and a kind's
    /// `HelperKind::label` (`run name`, …); the table adds `▸` and the indent.
    pub fn label(self) -> String {
        match self {
            Role::Orchestrator => "orchestrator".into(),
            Role::Planner => "planner".into(),
            Role::ImplementerSmall => "implementer · small".into(),
            Role::ImplementerMedium => "implementer · medium".into(),
            Role::ImplementerHub => "implementer · hub".into(),
            Role::TestWriter => "test writer".into(),
            Role::Reviewer => "reviewer".into(),
            Role::Research => "research".into(),
            Role::Helpers => "helpers".into(),
            Role::Helper(kind) => kind.label().into(),
        }
    }
}

impl Serialize for Role {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.key())
    }
}

impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Role::parse_key(&key)
            .ok_or_else(|| serde::de::Error::custom(format!("{key:?} is not a role")))
    }
}

/// One `[models]` table: only the rows it sets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTable {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rows: BTreeMap<Role, RoleChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brainstorm: Option<BrainstormChoice>,
}

impl ModelTable {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.brainstorm.is_none()
    }

    /// `self` with every row `top` sets replaced by `top`'s (decision 19).
    pub fn overlaid(&self, top: &ModelTable) -> ModelTable {
        let mut out = self.clone();
        out.rows
            .extend(top.rows.iter().map(|(r, c)| (*r, c.clone())));
        if top.brainstorm.is_some() {
            out.brainstorm = top.brainstorm.clone();
        }
        out
    }
}

/// Where a catalog came from (MR §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSource {
    Live,
    Cached,
    Builtin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModel {
    /// The id a route passes (`--model`), `default` for Claude's default entry.
    pub id: String,
    /// The model the CLI resolves `id` to (Claude's `resolvedModel`: `opus[1m]` is
    /// `claude-opus-5-5[1m]`), when it says. Real-CLI manual check fix (2026-10-09).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    pub label: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub runtime: Runtime,
    /// `2.1.290`, `0.160.1`, or `unknown`.
    pub cli_version: String,
    /// Seconds since the epoch.
    pub fetched_at: u64,
    pub source: CatalogSource,
    pub models: Vec<CatalogModel>,
    /// Decision 24: why the catalog is not live (`codex not found`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

impl CatalogModel {
    /// Decision 2 of the real-CLI manual check fix: the model this entry runs, as one
    /// id: its resolved model, else its id, without a context tag or a date
    /// ([`base_model_id`]).
    pub fn canonical(&self) -> &str {
        base_model_id(self.resolved.as_deref().unwrap_or(&self.id))
    }

    /// Whether a row's `id` names this entry: it is the entry's id or its resolved
    /// model, both sides compared without a context tag or a date.
    pub fn answers_to(&self, id: &str) -> bool {
        let id = base_model_id(id);
        base_model_id(&self.id) == id || self.resolved.as_deref().map(base_model_id) == Some(id)
    }
}

/// `id` without a trailing bracketed context tag (`[1m]`), then without a trailing
/// `-YYYYMMDD` date: `claude-opus-5-5[1m]` and `claude-haiku-4-5-20251001` are
/// `claude-opus-5-5` and `claude-haiku-4-5`.
pub fn base_model_id(id: &str) -> &str {
    let id = match id
        .strip_suffix(']')
        .and_then(|s| s.rfind('[').map(|at| &id[..at]))
    {
        Some(bare) if !bare.is_empty() => bare,
        _ => id,
    };
    match id.len().checked_sub(9).map(|at| id.split_at(at)) {
        Some((bare, date))
            if !bare.is_empty()
                && date.starts_with('-')
                && date[1..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            bare
        }
        _ => id,
    }
}

/// Decision 2: the one id `m` runs as, for every same-model rule: the entry a catalog
/// of `catalogs` finds for it ([`ModelCatalog::find`]), canonical; else `m`'s own id
/// without a context tag or a date (`""` for a CLI default no catalog names).
pub fn canonical_id(catalogs: &[ModelCatalog], m: &ModelRef) -> String {
    (catalogs.iter())
        .find_map(|c| c.find(m))
        .map(CatalogModel::canonical)
        .unwrap_or_else(|| base_model_id(m.route_model()))
        .to_string()
}

impl ModelCatalog {
    /// The model `m` names, if the catalog lists it (`id: None` is the entry with
    /// `is_default`, else none). An id finds the entry of that id; else (the real-CLI
    /// manual check fix, decision 1) the entry that answers to it
    /// ([`CatalogModel::answers_to`]: Claude lists `opus[1m]` resolving to
    /// `claude-opus-5-5[1m]` for the built-in `claude-opus-5-5`), the default entry
    /// last.
    pub fn find(&self, m: &ModelRef) -> Option<&CatalogModel> {
        if m.runtime != self.runtime {
            return None;
        }
        let Some(id) = &m.id else {
            return self.models.iter().find(|c| c.is_default);
        };
        let answers = |c: &&CatalogModel| c.answers_to(id);
        (self.models.iter().find(|c| &c.id == id))
            .or_else(|| self.models.iter().filter(|c| !c.is_default).find(answers))
            .or_else(|| self.models.iter().find(answers))
    }
}
