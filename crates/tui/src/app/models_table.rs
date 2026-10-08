//! Milestone 9.8 (MR §5.1, decisions 35 and 36): the Settings screen's models table as
//! state. One row per role in the spec's order, `brainstorm` (two models) before
//! `helpers ▸`, whose six kinds open inline. A row shows what the scope resolves to:
//! `everywhere` edits the global table (`SettingsDoc.roles`), a row it does not set
//! showing the built-in; `this repo` edits the repository's table, every row it does not
//! override drawn as inherited. Labels come from the catalogs (`model_picker.rs`).
//! Keys are `models_keys.rs`; drawing is M9.8.11's. Pure: no I/O.

use super::model_picker::{Catalogs, ModelPicker, Picked, PickerEntry, PickerFor};
use config::models::{resolve, resolve_brainstorm};
use proto::models::{BrainstormChoice, HelperKind, ModelRef, ModelTable, Role, RoleChoice};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A helper kind's model while no table sets it (MR §5.1).
pub const SAME_AS_HELPERS: &str = "same as helpers";
/// No effort (the model's default), no fallback.
pub const DASH: &str = "—";

/// The repository scope with nothing loaded yet: every row inherited.
static EMPTY: ModelTable = ModelTable {
    rows: BTreeMap::new(),
    brainstorm: None,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Everywhere,
    Repo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKey {
    Role(Role),
    Brainstorm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    pub key: RowKey,
    /// `implementer · medium`, `helpers ▸`, `  run name`.
    pub role: String,
    /// `Codex  · gpt-6 sol`, or `same as helpers`.
    pub model: String,
    /// `medium`, `—`.
    pub effort: String,
    /// `Claude · Opus 5.5`, `—`.
    pub fallback: String,
    /// `this repo` scope: the repository overrides this row (`●`).
    pub overridden: bool,
    /// `this repo` scope: inherited, drawn dimmed with `(everywhere)`.
    pub inherited: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoTable {
    pub table: ModelTable,
    /// The table as the daemon last read or saved it: unsaved changes differ from it.
    pub base: ModelTable,
    /// Where `w` writes (`SettingsReply::RepoModels.path`).
    pub path: PathBuf,
    /// The scope's own `PutRepoModels`, while its reply is awaited.
    pub put_id: Option<u64>,
    /// The table that `PutRepoModels` carried: a `RepoSaved` replaces the table only
    /// when nothing was edited since.
    pub sent: Option<ModelTable>,
    /// M9.8.12 fix round 1 (I2): the file's rows the daemon could not read, shown as
    /// warnings in `this repo` (a save is refused while there are any).
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsTable {
    pub scope: Scope,
    pub global: ModelTable,
    pub repo: Option<RepoTable>,
    /// `App::goal_project()` when the screen opened: `this repo`'s repository.
    pub project: Option<PathBuf>,
    pub selected: usize,
    pub helpers_open: bool,
    pub picker: Option<ModelPicker>,
}

impl Default for ModelsTable {
    fn default() -> Self {
        Self {
            scope: Scope::Everywhere,
            global: ModelTable::default(),
            repo: None,
            project: None,
            selected: 0,
            helpers_open: false,
            picker: None,
        }
    }
}

/// The effort after `now` among `efforts`, then `default` (`None`), then the first.
fn next_effort(efforts: &[String], now: Option<&str>) -> Option<String> {
    let at = now.and_then(|e| efforts.iter().position(|o| o == e));
    match (now, at) {
        (Some(_), Some(i)) => efforts.get(i + 1).cloned(),
        _ => efforts.first().cloned(),
    }
}

/// A new model for a row: an effort the new model's catalog does not offer becomes its
/// default (none when it offers no effort); an unknown model keeps the effort.
fn retarget(model: &mut ModelRef, effort: &mut Option<String>, to: ModelRef, catalogs: &Catalogs) {
    if *model == to {
        return;
    }
    if catalogs.knows(&to)
        && let Some(e) = effort.as_deref()
    {
        let offered = catalogs.efforts(&to);
        if !offered.iter().any(|o| o == e) {
            *effort = if offered.is_empty() {
                None
            } else {
                catalogs.default_effort(&to)
            };
        }
    }
    *model = to;
}

fn effort_text(effort: &Option<String>) -> String {
    effort.clone().unwrap_or_else(|| DASH.to_string())
}

/// `⚠ effort 'max' not offered`, `⚠ not reported by claude 2.1.290`.
fn warnings_of(model: &ModelRef, effort: Option<&str>, catalogs: &Catalogs) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(e) = effort
        && catalogs.not_offered(model, e)
    {
        out.push(format!("⚠ effort '{e}' not offered"));
    }
    out.extend(catalogs.unreported(model).map(|u| format!("⚠ {u}")));
    out
}

impl ModelsTable {
    /// The rows' keys in order: the spec's rows, brainstorm, helpers and, open, its
    /// six kinds.
    pub fn keys(&self) -> Vec<RowKey> {
        let mut out: Vec<RowKey> = (Role::ROWS.iter())
            .filter(|r| **r != Role::Helpers)
            .map(|r| RowKey::Role(*r))
            .collect();
        out.push(RowKey::Brainstorm);
        out.push(RowKey::Role(Role::Helpers));
        if self.helpers_open {
            out.extend(HelperKind::ALL.map(|k| RowKey::Role(Role::Helper(k))));
        }
        out
    }

    pub fn selected_key(&self) -> RowKey {
        let keys = self.keys();
        keys[self.selected.min(keys.len() - 1)]
    }

    /// The repository's table while the scope is `this repo`.
    fn repo_table(&self) -> Option<&ModelTable> {
        match self.scope {
            Scope::Repo => Some(self.repo.as_ref().map_or(&EMPTY, |r| &r.table)),
            Scope::Everywhere => None,
        }
    }

    /// The table the scope edits.
    pub fn editing(&self) -> &ModelTable {
        self.repo_table().unwrap_or(&self.global)
    }

    /// The table the scope edits, once it is loaded.
    fn editing_mut(&mut self) -> Option<&mut ModelTable> {
        match self.scope {
            Scope::Everywhere => Some(&mut self.global),
            Scope::Repo => self.repo.as_mut().map(|r| &mut r.table),
        }
    }

    /// What `role` resolves to in the scope (decision 7).
    pub fn choice(&self, role: Role) -> RoleChoice {
        resolve(role, self.repo_table(), &self.global)
    }

    pub fn brainstorm(&self) -> BrainstormChoice {
        resolve_brainstorm(self.repo_table(), &self.global)
    }

    /// Whether the repository overrides `key` (`this repo` only).
    fn overrides(&self, key: RowKey) -> bool {
        self.repo_table().is_some_and(|t| match key {
            RowKey::Role(r) => t.rows.contains_key(&r),
            RowKey::Brainstorm => t.brainstorm.is_some(),
        })
    }

    pub fn rows(&self, catalogs: &Catalogs) -> Vec<TableRow> {
        let scoped = self.scope == Scope::Repo;
        (self.keys().into_iter())
            .map(|key| {
                let overridden = self.overrides(key);
                let mut row = match key {
                    RowKey::Brainstorm => self.brainstorm_row(catalogs),
                    RowKey::Role(role) => self.role_row(role, catalogs),
                };
                row.overridden = overridden;
                row.inherited = scoped && !overridden;
                row
            })
            .collect()
    }

    fn role_row(&self, role: Role, catalogs: &Catalogs) -> TableRow {
        let label = match role {
            Role::Helpers if self.helpers_open => "helpers ▾".to_string(),
            Role::Helpers => "helpers ▸".to_string(),
            Role::Helper(kind) => format!("  {}", kind.label()),
            other => other.label(),
        };
        let row = |model: String, effort: String, fallback: String, warnings| TableRow {
            key: RowKey::Role(role),
            role: label.clone(),
            model,
            effort,
            fallback,
            overridden: false,
            inherited: false,
            warnings,
        };
        let set = |t: &ModelTable| t.rows.contains_key(&role);
        if matches!(role, Role::Helper(_))
            && !set(&self.global)
            && !self.repo_table().is_some_and(set)
        {
            return row(SAME_AS_HELPERS.into(), String::new(), String::new(), vec![]);
        }
        let c = self.choice(role);
        row(
            catalogs.label(&c.model, true),
            effort_text(&c.effort),
            (c.fallback.as_ref()).map_or_else(|| DASH.to_string(), |f| catalogs.label(f, false)),
            warnings_of(&c.model, c.effort.as_deref(), catalogs),
        )
    }

    fn brainstorm_row(&self, catalogs: &Catalogs) -> TableRow {
        let b = self.brainstorm();
        let mut warnings = warnings_of(&b.first, b.effort.as_deref(), catalogs);
        for w in warnings_of(&b.second, b.effort.as_deref(), catalogs) {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        TableRow {
            key: RowKey::Brainstorm,
            role: "brainstorm".into(),
            model: format!(
                "{}  +  {}",
                catalogs.label(&b.first, false),
                catalogs.label(&b.second, false)
            ),
            effort: effort_text(&b.effort),
            fallback: String::new(),
            overridden: false,
            inherited: false,
            warnings,
        }
    }

    /// `j`/`k`: the selection moved by `delta`, within the rows.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.keys().len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    /// `space` on `helpers` or one of its kinds: the six kinds open or close; closed,
    /// the selection is on `helpers`.
    pub fn toggle_helpers(&mut self) {
        if !matches!(
            self.selected_key(),
            RowKey::Role(Role::Helpers | Role::Helper(_))
        ) {
            return;
        }
        self.helpers_open = !self.helpers_open;
        if !self.helpers_open {
            let keys = self.keys();
            self.selected = (keys.iter())
                .position(|k| *k == RowKey::Role(Role::Helpers))
                .unwrap_or(0);
        }
    }

    /// `←`/`→`: `true` when `this repo` is entered with its table not yet read (the app
    /// asks for it). With no project, `this repo` cannot be entered.
    pub fn set_scope(&mut self, scope: Scope) -> bool {
        if scope == Scope::Repo && self.project.is_none() {
            return false;
        }
        self.scope = scope;
        scope == Scope::Repo && self.repo.is_none()
    }

    /// The scope's row for `role`, created from what it resolves to now.
    fn row_mut(&mut self, role: Role) -> Option<&mut RoleChoice> {
        let now = self.choice(role);
        Some(self.editing_mut()?.rows.entry(role).or_insert(now))
    }

    fn brainstorm_mut(&mut self) -> Option<&mut BrainstormChoice> {
        let now = self.brainstorm();
        Some(self.editing_mut()?.brainstorm.get_or_insert(now))
    }

    /// `e`: the row model's catalog efforts in turn, then `default`; nothing when the
    /// catalog reports none. `true` when it changed the table.
    pub fn cycle_effort(&mut self, catalogs: &Catalogs) -> bool {
        let (model, now) = match self.selected_key() {
            RowKey::Role(role) => {
                let c = self.choice(role);
                (c.model, c.effort)
            }
            RowKey::Brainstorm => {
                let b = self.brainstorm();
                (b.first, b.effort)
            }
        };
        let efforts = catalogs.efforts(&model);
        if efforts.is_empty() {
            return false;
        }
        let next = next_effort(&efforts, now.as_deref());
        let slot = match self.selected_key() {
            RowKey::Role(role) => self.row_mut(role).map(|c| &mut c.effort),
            RowKey::Brainstorm => self.brainstorm_mut().map(|b| &mut b.effort),
        };
        slot.map(|e| *e = next).is_some()
    }

    /// `x`: the scope's row removed (`this repo`: the override; `everywhere`: back to
    /// the built-in). `true` when it changed the table.
    pub fn reset(&mut self) -> bool {
        let key = self.selected_key();
        let Some(t) = self.editing_mut() else {
            return false;
        };
        match key {
            RowKey::Role(role) => t.rows.remove(&role).is_some(),
            RowKey::Brainstorm => t.brainstorm.take().is_some(),
        }
    }

    /// `⏎` (`fallback` false) or `f`: the picker on the selected row's current model
    /// (`⏎` on brainstorm picks its first model). Nothing while `this repo` loads, and
    /// `f` on brainstorm, which has no fallback.
    pub fn open_picker(&mut self, catalogs: &Catalogs, fallback: bool) {
        if self.editing_mut().is_none() {
            return;
        }
        let (target, current, first) = match (self.selected_key(), fallback) {
            (RowKey::Role(role), false) => {
                (PickerFor::Row(role), Some(self.choice(role).model), None)
            }
            (RowKey::Role(role), true) => (
                PickerFor::Fallback(role),
                self.choice(role).fallback,
                Some(PickerEntry::NoFallback),
            ),
            (RowKey::Brainstorm, false) => (
                PickerFor::Brainstorm(0),
                Some(self.brainstorm().first),
                None,
            ),
            (RowKey::Brainstorm, true) => return,
        };
        self.picker = Some(ModelPicker::new(target, catalogs, current.as_ref(), first));
    }

    /// What the picker picked, applied to its row; brainstorm's first pick opens the
    /// second. `Refresh` is the app's (it keeps the picker open). `true` when it changed
    /// the table.
    pub fn on_picked(&mut self, picked: Picked, catalogs: &Catalogs) -> bool {
        if picked == Picked::Refresh {
            return false;
        }
        let Some(target) = self.picker.take().map(|p| p.target) else {
            return false;
        };
        match (target, picked) {
            (PickerFor::Row(role), Picked::Model(m)) => self
                .row_mut(role)
                .map(|c| retarget(&mut c.model, &mut c.effort, m, catalogs))
                .is_some(),
            (PickerFor::Fallback(role), Picked::Model(m)) => {
                self.row_mut(role).map(|c| c.fallback = Some(m)).is_some()
            }
            (PickerFor::Fallback(role), Picked::NoFallback) => {
                self.row_mut(role).map(|c| c.fallback = None).is_some()
            }
            (PickerFor::Brainstorm(0), Picked::Model(m)) => {
                let changed = (self.brainstorm_mut())
                    .map(|b| retarget(&mut b.first, &mut b.effort, m, catalogs))
                    .is_some();
                let second = self.brainstorm().second;
                self.picker = Some(ModelPicker::new(
                    PickerFor::Brainstorm(1),
                    catalogs,
                    Some(&second),
                    None,
                ));
                changed
            }
            (PickerFor::Brainstorm(_), Picked::Model(m)) => {
                (self.brainstorm_mut()).map(|b| b.second = m).is_some()
            }
            _ => false,
        }
    }

    /// Whether the repository's table differs from what the daemon last said.
    pub fn repo_dirty(&self) -> bool {
        self.repo.as_ref().is_some_and(|r| r.table != r.base)
    }
}

#[cfg(test)]
#[path = "models_table_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "models_table_tests_save.rs"]
mod tests_save;
