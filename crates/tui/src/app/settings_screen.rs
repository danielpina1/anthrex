//! Milestone 9.0.6 decision 36: the Settings screen's state, its requests and replies.
//! `C-b S` opens it from `App.settings_cache` (one `Settings(Get)` only when there is no
//! cache yet). Its sections are `claude`, `codex` (the shipped models, then the doc's
//! other models of that runtime, then `custom…`), `orchestrator` (the default runtime
//! and model, over the enabled models) and `limits`. Every rule is the daemon's own:
//! `config::settings::validate` and `warnings` decide what blocks `w` and what only
//! warns; this file never re-implements a range. `w` sends one tagged `Settings(Put)`
//! (`App::settings_put`); its reply comes back by `request_id` through
//! `app/screens.rs::route_settings_reply`. Keys are `settings_keys.rs`; drawing is
//! `ui/settings.rs`. Pure: every request leaves as an `Effect`.

use crate::text_area::TextArea;
use config::settings::{SHIPPED_CLAUDE, SHIPPED_CODEX, ShippedModel, validate, warnings};
use proto::settings::key;
use proto::{
    BudgetLimit, ModelEntry, OrchestratorDefault, Origin, Runtime, SettingsDoc, SettingsLimits,
    Strength, TuningReport,
};
use std::collections::BTreeMap;

/// Interfaces "Exact user-visible text": a `Saved` reply.
pub const SAVED: &str = "saved · new runs use these settings · runs in progress keep theirs";
/// Interfaces "Exact user-visible text": `Esc` with unsaved changes.
pub const DISCARD_ASK: &str = "discard unsaved settings? y";
/// `C-b a`, `C-b m` and `C-b t` while the screen is open (as the Profile screen's).
pub const LEAVE_SETTINGS_FIRST: &str = "leave the settings first (esc)";
/// `w` with nothing changed.
pub const NO_CHANGES: &str = "no changes to save";
/// The screen's `Put` lost its link before any reply.
pub const LINK_LOST: &str = "not saved: link lost";
/// The custom dialog, when the typed name is a shipped model of its runtime.
pub const SHIPPED_FIXED: &str = "shipped model · strength is fixed";
/// Another screen asked for while this one holds unsaved changes.
pub const UNSAVED_FIRST: &str = "unsaved settings: w saves, esc discards";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Claude,
    Codex,
    Orchestrator,
    Limits,
}

impl SettingsSection {
    pub const ALL: [SettingsSection; 4] = [
        SettingsSection::Claude,
        SettingsSection::Codex,
        SettingsSection::Orchestrator,
        SettingsSection::Limits,
    ];

    /// Interfaces "Settings sections".
    pub fn name(self) -> &'static str {
        match self {
            SettingsSection::Claude => "claude",
            SettingsSection::Codex => "codex",
            SettingsSection::Orchestrator => "orchestrator",
            SettingsSection::Limits => "limits",
        }
    }

    /// The model table's runtime.
    pub fn runtime(self) -> Option<Runtime> {
        match self {
            SettingsSection::Claude => Some(Runtime::Claude),
            SettingsSection::Codex => Some(Runtime::Codex),
            _ => None,
        }
    }
}

/// One row of a model table: a shipped model (enabled when the doc has it) or one of the
/// doc's other models of that runtime (`custom`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    pub entry: ModelEntry,
    /// What the row shows: the shipped label, or the custom model's name.
    pub label: String,
    pub enabled: bool,
    pub custom: bool,
}

/// One limit: its key path (`proto::settings::key`), its label and the digits typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitField {
    pub key: &'static str,
    pub label: &'static str,
    pub text: String,
}

/// The limits, in `SETTINGS_KEYS` order.
pub const LIMITS: [(&str, &str); 10] = [
    (key::BUDGET_S_CALLS, "s tool calls"),
    (key::BUDGET_S_MINUTES, "s minutes"),
    (key::BUDGET_M_CALLS, "m tool calls"),
    (key::BUDGET_M_MINUTES, "m minutes"),
    (key::BUDGET_L_CALLS, "l tool calls"),
    (key::BUDGET_L_MINUTES, "l minutes"),
    (key::STALL_AFTER_SECS, "stall after secs"),
    (key::MAX_WRITERS, "max writers"),
    (key::MAX_READERS, "max readers"),
    (key::MAX_BOUNCES, "max bounces"),
];

/// The two-field dialog `custom…` opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomModel {
    pub runtime: Runtime,
    pub model: TextArea,
    pub strength: Strength,
    /// Focus is on the strength field.
    pub on_strength: bool,
    pub error: Option<String>,
}

impl CustomModel {
    /// The typed name is one of the runtime's shipped models: its row is enabled with
    /// the shipped strength (decision 36), so the dialog says so.
    pub fn shipped(&self) -> bool {
        let name = crate::safe_text::one_line(self.model.text());
        let shipped: &[ShippedModel] = match self.runtime {
            Runtime::Codex => &SHIPPED_CODEX,
            _ => &SHIPPED_CLAUDE,
        };
        shipped.iter().any(|m| m.model == name.trim())
    }
}

/// A dialog over the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsPage {
    Custom(CustomModel),
    /// `Esc` with unsaved changes: `discard unsaved settings? y`.
    Discard,
}

/// What the last save came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    /// The daemon's problems (or `no reply from daemon`), every one.
    Refused(Vec<String>),
    /// The link went before any reply: `not saved: link lost`.
    LinkLost,
    /// The connection refused the `Put`'s send while connected: `not sent: daemon is
    /// not responding` (final review I1).
    NotSent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsScreen {
    pub section: SettingsSection,
    pub claude: Vec<ModelRow>,
    pub codex: Vec<ModelRow>,
    /// The orchestrator default: `None` is `configured`; `""` is `default`.
    pub runtime: Option<Runtime>,
    pub model: String,
    pub limits: Vec<LimitField>,
    /// The selected row of the section (a model table's `custom…` is its last row).
    pub selected: usize,
    /// `false` until the first `Current` (no cache when it opened).
    pub loaded: bool,
    /// The document the screen opened on (or last saved): unsaved changes differ from it.
    pub base: SettingsDoc,
    pub origin: BTreeMap<String, Origin>,
    /// Where `w` writes (`SettingsReply::Current.path`).
    pub path: String,
    pub page: Option<SettingsPage>,
    /// The screen's own `Put`, while its reply is awaited (see `App::settings_saving`).
    pub put_id: Option<u64>,
    /// The doc that `Put` carried: a `Saved` reloads the screen only when nothing was
    /// edited since.
    pub sent: Option<SettingsDoc>,
    pub outcome: Option<SaveOutcome>,
    /// Milestone 9.5 decision 48: the read-only `Stats` sent on opening, while its reply
    /// is due, and the project's tuning it brought (the notes beside the budgets and the
    /// orchestrator default; none before it, or with no project).
    pub tuning_request: Option<u64>,
    pub tuning: Option<Box<TuningReport>>,
}

fn row_of(s: &ShippedModel, doc: &SettingsDoc) -> ModelRow {
    let found = doc
        .models
        .iter()
        .find(|m| m.runtime == s.runtime && m.model == s.model);
    ModelRow {
        entry: found.cloned().unwrap_or(ModelEntry {
            runtime: s.runtime,
            model: s.model.to_string(),
            strength: s.strength,
            note: String::new(),
        }),
        label: s.label.to_string(),
        enabled: found.is_some(),
        custom: false,
    }
}

/// The table of `runtime`: every shipped model, then the doc's other models of it.
fn table(shipped: &[ShippedModel], runtime: Runtime, doc: &SettingsDoc) -> Vec<ModelRow> {
    let mut rows: Vec<ModelRow> = shipped.iter().map(|s| row_of(s, doc)).collect();
    for m in doc.models.iter().filter(|m| m.runtime == runtime) {
        if !rows.iter().any(|r| r.entry.model == m.model) {
            rows.push(ModelRow {
                entry: m.clone(),
                label: m.model.clone(),
                enabled: true,
                custom: true,
            });
        }
    }
    rows
}

/// A limit's value in `l`.
pub fn limit_value(l: &SettingsLimits, key: &str) -> u64 {
    let budget =
        |b: &BudgetLimit, calls: bool| u64::from(if calls { b.tool_calls } else { b.minutes });
    match key {
        key::BUDGET_S_CALLS => budget(&l.budget_s, true),
        key::BUDGET_S_MINUTES => budget(&l.budget_s, false),
        key::BUDGET_M_CALLS => budget(&l.budget_m, true),
        key::BUDGET_M_MINUTES => budget(&l.budget_m, false),
        key::BUDGET_L_CALLS => budget(&l.budget_l, true),
        key::BUDGET_L_MINUTES => budget(&l.budget_l, false),
        key::STALL_AFTER_SECS => l.stall_after_secs,
        key::MAX_WRITERS => u64::from(l.max_writers),
        key::MAX_READERS => u64::from(l.max_readers),
        key::MAX_BOUNCES => u64::from(l.max_bounces),
        _ => 0,
    }
}

/// Sets a limit; a value its field cannot hold saturates, so `validate` reports it with
/// the range's own message.
fn set_limit(l: &mut SettingsLimits, key: &str, v: u64) {
    let u32_of = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
    let u8_of = |v: u64| u8::try_from(v).unwrap_or(u8::MAX);
    match key {
        key::BUDGET_S_CALLS => l.budget_s.tool_calls = u32_of(v),
        key::BUDGET_S_MINUTES => l.budget_s.minutes = u32_of(v),
        key::BUDGET_M_CALLS => l.budget_m.tool_calls = u32_of(v),
        key::BUDGET_M_MINUTES => l.budget_m.minutes = u32_of(v),
        key::BUDGET_L_CALLS => l.budget_l.tool_calls = u32_of(v),
        key::BUDGET_L_MINUTES => l.budget_l.minutes = u32_of(v),
        key::STALL_AFTER_SECS => l.stall_after_secs = v,
        key::MAX_WRITERS => l.max_writers = u8_of(v),
        key::MAX_READERS => l.max_readers = u8_of(v),
        key::MAX_BOUNCES => l.max_bounces = u8_of(v),
        _ => {}
    }
}

/// Interfaces "Settings hard stop", tool calls: the engine breaches at
/// `2 × calls ≥ 3 × budget` (`engine/ladder.rs`), so the first breaching count.
pub fn hard_stop_calls(n: u64) -> String {
    format!("hard stop at {} calls", (3 * n).div_ceil(2))
}

/// Interfaces "Settings hard stop", minutes: the breach is at `2 × secs ≥ 180 × budget`,
/// so at `90 × n` seconds, shown as minutes and, when `n` is odd, `30s`.
pub fn hard_stop_minutes(n: u64) -> String {
    let secs = 90 * n;
    match secs % 60 {
        0 => format!("hard stop at {}m", secs / 60),
        s => format!("hard stop at {}m{s}s", secs / 60),
    }
}

/// The strength after (or before) `s`, wrapping.
pub(crate) fn next_strength(s: Strength, forward: bool) -> Strength {
    const ORDER: [Strength; 3] = [Strength::Fast, Strength::Standard, Strength::Frontier];
    let at = ORDER.iter().position(|o| *o == s).unwrap_or(0);
    let next = if forward {
        at + 1
    } else {
        at + ORDER.len() - 1
    };
    ORDER[next % ORDER.len()]
}

pub fn strength_name(s: Strength) -> &'static str {
    match s {
        Strength::Fast => "fast",
        Strength::Standard => "standard",
        Strength::Frontier => "frontier",
    }
}

impl SettingsScreen {
    /// The screen before any `Current` has come: nothing to edit yet.
    pub fn loading() -> Self {
        const NONE: BudgetLimit = BudgetLimit {
            tool_calls: 0,
            minutes: 0,
        };
        let empty = SettingsDoc {
            models: vec![],
            orchestrator: OrchestratorDefault {
                runtime: None,
                model: String::new(),
            },
            limits: SettingsLimits {
                budget_s: NONE,
                budget_m: NONE,
                budget_l: NONE,
                stall_after_secs: 0,
                max_writers: 0,
                max_readers: 0,
                max_bounces: 0,
            },
        };
        let mut s = Self {
            section: SettingsSection::Claude,
            claude: vec![],
            codex: vec![],
            runtime: None,
            model: String::new(),
            limits: vec![],
            selected: 0,
            loaded: false,
            base: empty.clone(),
            origin: BTreeMap::new(),
            path: String::new(),
            page: None,
            put_id: None,
            sent: None,
            outcome: None,
            tuning_request: None,
            tuning: None,
        };
        s.load(&empty, &BTreeMap::new());
        s.loaded = false;
        s
    }

    /// The screen on `doc`.
    pub fn from_doc(doc: &SettingsDoc, origin: &BTreeMap<String, Origin>) -> Self {
        let mut s = Self::loading();
        s.load(doc, origin);
        s
    }

    /// Replaces every value with `doc`'s, keeping the section and the selection.
    pub fn load(&mut self, doc: &SettingsDoc, origin: &BTreeMap<String, Origin>) {
        self.claude = table(&SHIPPED_CLAUDE, Runtime::Claude, doc);
        self.codex = table(&SHIPPED_CODEX, Runtime::Codex, doc);
        self.runtime = doc.orchestrator.runtime;
        self.model = doc.orchestrator.model.clone();
        self.limits = LIMITS
            .iter()
            .map(|(key, label)| LimitField {
                key,
                label,
                text: limit_value(&doc.limits, key).to_string(),
            })
            .collect();
        self.base = doc.clone();
        self.origin = origin.clone();
        self.loaded = true;
        self.selected = self.selected.min(self.last_row());
    }

    pub fn rows(&self, runtime: Runtime) -> &[ModelRow] {
        match runtime {
            Runtime::Codex => &self.codex,
            _ => &self.claude,
        }
    }

    pub(crate) fn rows_mut(&mut self, runtime: Runtime) -> &mut Vec<ModelRow> {
        match runtime {
            Runtime::Codex => &mut self.codex,
            _ => &mut self.claude,
        }
    }

    /// The section's last selectable row.
    pub fn last_row(&self) -> usize {
        match self.section.runtime() {
            Some(r) => self.rows(r).len(),
            None if self.section == SettingsSection::Orchestrator => 1,
            None => self.limits.len().saturating_sub(1),
        }
    }

    /// The document the screen holds, valid or not. The roster keeps the order the
    /// screen opened on; a model enabled since follows, Claude's table first.
    pub fn built(&self) -> SettingsDoc {
        let enabled: Vec<&ModelRow> = self
            .claude
            .iter()
            .chain(&self.codex)
            .filter(|r| r.enabled)
            .collect();
        let same =
            |r: &ModelRow, m: &ModelEntry| r.entry.runtime == m.runtime && r.entry.model == m.model;
        let mut models: Vec<ModelEntry> = Vec::new();
        for m in &self.base.models {
            if let Some(r) = enabled.iter().find(|r| same(r, m))
                && !models.iter().any(|e| same(r, e))
            {
                models.push(r.entry.clone());
            }
        }
        for r in enabled {
            if !models.iter().any(|e| same(r, e)) {
                models.push(r.entry.clone());
            }
        }
        let mut limits = self.base.limits.clone();
        for f in &self.limits {
            // Blank reads 0, which every range refuses with its own message.
            set_limit(&mut limits, f.key, f.text.parse().unwrap_or(0));
        }
        SettingsDoc {
            models,
            orchestrator: OrchestratorDefault {
                runtime: self.runtime,
                model: self.model.clone(),
            },
            limits,
        }
    }

    /// Decision 36: the doc to send, or every reason it cannot be saved (the daemon's
    /// own rules, `config::settings::validate`).
    pub fn doc(&self) -> Result<SettingsDoc, Vec<String>> {
        if !self.loaded {
            return Err(vec![]);
        }
        let doc = self.built();
        let problems = validate(&doc);
        if problems.is_empty() {
            Ok(doc)
        } else {
            Err(problems)
        }
    }

    /// What blocks `w` now.
    pub fn problems(&self) -> Vec<String> {
        self.doc().err().unwrap_or_default()
    }

    /// What only warns (`config::settings::warnings`).
    pub fn warnings(&self) -> Vec<String> {
        if self.loaded {
            warnings(&self.built())
        } else {
            vec![]
        }
    }

    /// Whether anything differs from the document the screen opened on.
    pub fn dirty(&self) -> bool {
        self.loaded && self.built() != self.base
    }

    /// The orchestrator model picker's options: `""` (`default`), then the enabled
    /// models of the chosen runtime; with runtime `configured`, `default` only.
    pub fn model_options(&self) -> Vec<String> {
        let mut out = vec![String::new()];
        if let Some(runtime) = self.runtime {
            out.extend(
                self.rows(runtime)
                    .iter()
                    .filter(|r| r.enabled && !r.entry.model.is_empty())
                    .map(|r| r.entry.model.clone()),
            );
        }
        out
    }

    /// Decision 26: `key`'s value comes from the built-in defaults and is unchanged.
    pub fn is_default(&self, key: &str) -> bool {
        if self.origin.get(key) != Some(&Origin::Default) {
            return false;
        }
        let (now, base) = (self.built(), &self.base);
        match key {
            key::MODELS => now.models == base.models,
            key::AGENT_RUNTIME => now.orchestrator.runtime == base.orchestrator.runtime,
            key::AGENT_MODEL => now.orchestrator.model == base.orchestrator.model,
            k => limit_value(&now.limits, k) == limit_value(&base.limits, k),
        }
    }

    /// Ruling RH-5: `refit: <budget>` beside the budget of `class` (`"S"` or `"M"`) when
    /// the project's tuning has a refit for it and config does not set it explicitly.
    pub fn refit_note(&self, class: &str) -> Option<String> {
        let c = (self.tuning.as_ref()?.classes.iter()).find(|c| c.class == class)?;
        let budget = c.refit_budget.as_ref().filter(|_| !c.configured)?;
        Some(format!(
            "refit: {}",
            daemon::run::refit::budget_text(budget)
        ))
    }

    /// Ruling RH-5: the orchestrator default is overridden by a model list.
    pub fn orchestrator_overridden(&self) -> bool {
        (self.tuning.as_ref()).is_some_and(|t| t.orchestrator_list.is_some())
    }

    /// An edit makes the last outcome stale.
    pub(crate) fn touched(&mut self) {
        self.outcome = None;
    }
}

#[path = "settings_flow.rs"]
mod flow;
#[path = "settings_keys.rs"]
mod keys;
