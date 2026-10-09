//! Milestone 9.0.6 decision 36: the Settings screen's state, its requests and replies.
//! `C-b S` opens it from `App.settings_cache` (one `Settings(Get)` only when there is no
//! cache yet). Its sections are `models` (milestone 9.8 decision 35: the role table,
//! `models_table.rs`, and its picker, `model_picker.rs`) and `limits`. Every rule is the
//! daemon's own:
//! `config::settings::validate` decides what blocks `w`; this file never re-implements a
//! range. `w` sends one tagged `Settings(Put)`
//! (`App::settings_put`); its reply comes back by `request_id` through
//! `app/screens.rs::route_settings_reply`. Keys are `settings_keys.rs`; drawing is
//! `ui/settings.rs`. Pure: every request leaves as an `Effect`.

use super::models_table::ModelsTable;
use config::settings::validate;
use proto::settings::key;
use proto::{BudgetLimit, Origin, SettingsDoc, SettingsLimits, TuningReport};
use std::collections::BTreeMap;

/// Interfaces "Exact user-visible text": a `Saved` reply.
pub const SAVED: &str = "saved · new runs use these settings · runs in progress keep theirs";
/// Milestone 9.8 (MR §5.1): the status line after `w` on the `models` section.
pub const MODELS_SAVED: &str = "saved · new runs use these models · runs in progress keep theirs";
/// Interfaces "Exact user-visible text": `Esc` with unsaved changes.
pub const DISCARD_ASK: &str = "discard unsaved settings? y";
/// `C-b a`, `C-b m` and `C-b t` while the screen is open (as the Profile screen's).
pub const LEAVE_SETTINGS_FIRST: &str = "leave the settings first (esc)";
/// `w` with nothing changed.
pub const NO_CHANGES: &str = "no changes to save";
/// The screen's `Put` lost its link before any reply.
pub const LINK_LOST: &str = "not saved: link lost";
/// Another screen asked for while this one holds unsaved changes.
pub const UNSAVED_FIRST: &str = "unsaved settings: w saves, esc discards";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Models,
    Limits,
}

impl SettingsSection {
    pub const ALL: [SettingsSection; 2] = [SettingsSection::Models, SettingsSection::Limits];

    /// Interfaces "Settings sections".
    pub fn name(self) -> &'static str {
        match self {
            SettingsSection::Models => "models",
            SettingsSection::Limits => "limits",
        }
    }
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

/// A dialog over the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsPage {
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
    /// Milestone 9.8: the role table, its scopes and its picker.
    pub models: ModelsTable,
    pub limits: Vec<LimitField>,
    /// The selected limit (the models table keeps its own selection).
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

impl SettingsScreen {
    /// The screen before any `Current` has come: nothing to edit yet.
    pub fn loading() -> Self {
        const NONE: BudgetLimit = BudgetLimit {
            tool_calls: 0,
            minutes: 0,
        };
        let empty = SettingsDoc {
            limits: SettingsLimits {
                budget_s: NONE,
                budget_m: NONE,
                budget_l: NONE,
                stall_after_secs: 0,
                max_writers: 0,
                max_readers: 0,
                max_bounces: 0,
            },
            design_default: None,
            roles: Default::default(),
        };
        let mut s = Self {
            section: SettingsSection::Models,
            models: ModelsTable::default(),
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

    /// Replaces every value with `doc`'s, keeping the section, the selection, the
    /// scope and the repository's table.
    pub fn load(&mut self, doc: &SettingsDoc, origin: &BTreeMap<String, Origin>) {
        self.models.global = doc.roles.clone();
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

    /// The limits' last row.
    pub fn last_row(&self) -> usize {
        self.limits.len().saturating_sub(1)
    }

    /// The document the screen holds, valid or not: the limits, and the role table of
    /// the `everywhere` scope (M9.8.12: the roster and the orchestrator default left the
    /// document).
    pub fn built(&self) -> SettingsDoc {
        let mut limits = self.base.limits.clone();
        for f in &self.limits {
            // Blank reads 0, which every range refuses with its own message.
            set_limit(&mut limits, f.key, f.text.parse().unwrap_or(0));
        }
        SettingsDoc {
            limits,
            // Ruling T18-2: read-only here; the screen carries what the daemon said.
            design_default: self.base.design_default,
            roles: self.models.global.clone(),
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

    /// Whether the global document differs from the one the screen opened on: what
    /// `w` in `everywhere` (or on `limits`) would save.
    pub fn doc_dirty(&self) -> bool {
        self.loaded && self.built() != self.base
    }

    /// Whether anything differs from what the daemon last said: the document or the
    /// repository's table (`esc` asks before discarding either).
    pub fn dirty(&self) -> bool {
        self.doc_dirty() || (self.loaded && self.models.repo_dirty())
    }

    /// Decision 26: `key`'s value comes from the built-in defaults and is unchanged.
    pub fn is_default(&self, key: &str) -> bool {
        if self.origin.get(key) != Some(&Origin::Default) {
            return false;
        }
        let (now, base) = (self.built(), &self.base);
        limit_value(&now.limits, key) == limit_value(&base.limits, key)
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

    /// An edit makes the last outcome stale.
    pub(crate) fn touched(&mut self) {
        self.outcome = None;
    }
}

#[path = "settings_flow.rs"]
mod flow;
#[path = "settings_keys.rs"]
mod keys;
