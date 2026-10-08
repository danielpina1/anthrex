//! Milestone 9.8 (MR §5.2, decisions 37 and 38): the client's copy of the discovered
//! model catalogs, and the model picker as state. The catalogs arrive only as
//! `DaemonMsg::Models` (`app/daemon.rs`); every string a catalog carries (a label, a
//! description, an effort name) is cleaned here with `safe_text::one_line` and trimmed
//! (preflight F19: control characters become spaces, format characters are dropped),
//! and an effort that is not an effort name is dropped. The renderer (M9.8.11) cuts
//! each to its column. A model typed under `custom…` is accepted only as
//! `ModelRef::parse` accepts it. Pure: no I/O; ages are taken as arguments (F10).

use crate::safe_text::one_line;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::models::{CatalogModel, CatalogSource, ModelRef, Role, valid_effort};
use proto::{ModelCatalog, Runtime};
use std::time::{Duration, Instant};

/// The runtimes the picker lists, in order.
pub const RUNTIMES: [Runtime; 2] = [Runtime::Claude, Runtime::Codex];

/// The description of a current model the catalog does not list.
pub const NOT_REPORTED: &str = "not reported";

/// What a model entry's efforts read when it has none.
pub const NO_EFFORT: &str = "—";

/// A catalog string as the screen may hold it (decision 38, preflight F19).
pub(crate) fn clean(text: &str) -> String {
    one_line(text).trim().to_string()
}

/// `Claude`, `Codex`: the runtime as the table names it.
pub fn runtime_name(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Claude => "Claude",
        Runtime::Codex => "Codex",
        Runtime::Shell => "Shell",
    }
}

/// The catalogs `DaemonMsg::Models` has delivered, one per runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalogs {
    pub list: Vec<ModelCatalog>,
    /// Preflight F10: when this client received each runtime's catalog (the picker's
    /// `updated <age> ago`), stamped by `app/daemon.rs::on_daemon`.
    pub received_at: Vec<(Runtime, Instant)>,
}

impl Catalogs {
    /// Stores `incoming`, received at `now`, a catalog replacing the one already held
    /// for its runtime (a `ListModels` for one runtime says nothing about the other).
    pub fn absorb(&mut self, incoming: Vec<ModelCatalog>, now: Instant) {
        for catalog in incoming {
            let runtime = catalog.runtime;
            match self.list.iter_mut().find(|c| c.runtime == runtime) {
                Some(held) => *held = catalog,
                None => self.list.push(catalog),
            }
            self.received_at.retain(|(r, _)| *r != runtime);
            self.received_at.push((runtime, now));
        }
    }

    /// When `runtime`'s catalog was received.
    pub fn received(&self, runtime: Runtime) -> Option<Instant> {
        (self.received_at.iter())
            .find(|(r, _)| *r == runtime)
            .map(|(_, at)| *at)
    }

    /// How long ago, at `now`, `runtime`'s catalog was received.
    pub fn age(&self, runtime: Runtime, now: Instant) -> Option<Duration> {
        self.received(runtime)
            .map(|at| now.saturating_duration_since(at))
    }

    /// The picker's `updated <age> ago` at `now`: the oldest catalog's age, since `r`
    /// refreshes them all; `None` before any arrived.
    pub fn updated_age(&self, now: Instant) -> Option<Duration> {
        (self.received_at.iter())
            .map(|(_, at)| now.saturating_duration_since(*at))
            .max()
    }

    pub fn of(&self, runtime: Runtime) -> Option<&ModelCatalog> {
        self.list.iter().find(|c| c.runtime == runtime)
    }

    /// The catalog's entry for `model`, from any source.
    fn find(&self, model: &ModelRef) -> Option<&CatalogModel> {
        self.of(model.runtime)?.find(model)
    }

    /// Whether `runtime`'s catalog is one the CLI reported (`Live` or `Cached`):
    /// decision 21, a `Builtin` list is a guess and warns about nothing.
    fn reported(&self, runtime: Runtime) -> Option<&ModelCatalog> {
        self.of(runtime)
            .filter(|c| matches!(c.source, CatalogSource::Live | CatalogSource::Cached))
    }

    /// `Claude · Opus 5.5`: the runtime name, padded to 6 when `pad`, then the
    /// catalog's label, else the id, else `default`; cleaned (decision 38). A
    /// `<runtime>:default` row reads `default`: it is whatever that CLI is set to use.
    pub fn label(&self, model: &ModelRef, pad: bool) -> String {
        let name = runtime_name(model.runtime);
        let name = if pad {
            format!("{name:<6}")
        } else {
            name.to_string()
        };
        let label = match &model.id {
            None => "default".to_string(),
            Some(id) => self
                .find(model)
                .map(|m| clean(&m.label))
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| clean(id)),
        };
        format!("{name} · {label}")
    }

    /// The model's efforts as the catalog reports them (empty when unknown), each an
    /// effort name (`proto::models::valid_effort`).
    pub fn efforts(&self, model: &ModelRef) -> Vec<String> {
        self.find(model).map(efforts_of).unwrap_or_default()
    }

    /// The model's default effort, when the catalog names a valid one.
    pub fn default_effort(&self, model: &ModelRef) -> Option<String> {
        let found = self.find(model)?;
        found.default_effort.clone().filter(|e| valid_effort(e))
    }

    /// Whether the catalog knows `model` (from any source).
    pub fn knows(&self, model: &ModelRef) -> bool {
        self.find(model).is_some()
    }

    /// MR §4.4: `not reported by codex 0.160.1` when the runtime's catalog is Live or
    /// Cached and does not list the model. A `<runtime>:default` row is never warned.
    pub fn unreported(&self, model: &ModelRef) -> Option<String> {
        model.id.as_ref()?;
        let catalog = self.reported(model.runtime)?;
        if catalog.find(model).is_some() {
            return None;
        }
        Some(format!(
            "not reported by {} {}",
            model.runtime.label(),
            clean(&catalog.cli_version)
        ))
    }

    /// Decision 21's rule as the daemon applies it at launch: a Live or Cached catalog
    /// lists `model` with efforts, and `effort` is not one of them.
    pub fn not_offered(&self, model: &ModelRef, effort: &str) -> bool {
        let Some(found) = self.reported(model.runtime).and_then(|c| c.find(model)) else {
            return false;
        };
        !found.efforts.is_empty() && !found.efforts.iter().any(|e| e == effort)
    }
}

fn efforts_of(m: &CatalogModel) -> Vec<String> {
    (m.efforts.iter())
        .filter(|e| valid_effort(e))
        .cloned()
        .collect()
}

/// `—`, `low`, or `low … max`.
fn efforts_text(efforts: &[String]) -> String {
    match efforts {
        [] => NO_EFFORT.to_string(),
        [one] => one.clone(),
        [first, .., last] => format!("{first} … {last}"),
    }
}

/// What the picker chooses for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerFor {
    Row(Role),
    Fallback(Role),
    Brainstorm(u8),
    Goal,
    TaskEdit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerEntry {
    /// `CLAUDE  (claude 2.1.290)`, `(cached)`, `(built-in list)`, or `codex not found`.
    Header {
        runtime: Runtime,
        text: String,
        missing: bool,
    },
    Model {
        model: ModelRef,
        label: String,
        description: String,
        efforts: String,
        current: bool,
    },
    /// `f`'s first entry.
    NoFallback,
    /// The goal and task edit forms' first entry: no choice.
    RoleTable(String),
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Model(ModelRef),
    NoFallback,
    RoleTable,
    Refresh,
    Cancel,
}

/// `custom…`'s two steps (preflight F36): the runtime, then the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomModel {
    pub runtime: Runtime,
    pub name: TextArea,
    /// `false` while the runtime is chosen, `true` once the name is typed.
    pub naming: bool,
    /// Why the typed name is not a model (`ModelRef::parse`'s message).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPicker {
    pub target: PickerFor,
    pub entries: Vec<PickerEntry>,
    /// An index into `entries`; never a header or a missing runtime's row.
    pub selected: usize,
    /// `custom…`'s two steps: the runtime, then the name.
    pub custom: Option<CustomModel>,
    /// The row's model when the picker opened (`●`), kept across a refresh.
    pub current: Option<ModelRef>,
}

/// `runtime`'s header: its version when live, else where the list came from; a CLI
/// that is not installed (decision 24's `<runtime> not found`) is `missing`.
fn header(runtime: Runtime, catalog: Option<&ModelCatalog>) -> PickerEntry {
    let upper = runtime.label().to_uppercase();
    let not_found = format!("{} not found", runtime.label());
    let (what, missing) = match catalog {
        None => ("(no list yet)".to_string(), false),
        Some(c) if c.problem.as_deref() == Some(not_found.as_str()) => (not_found, true),
        Some(c) => match c.source {
            CatalogSource::Live => (
                format!("({} {})", runtime.label(), clean(&c.cli_version)),
                false,
            ),
            CatalogSource::Cached => ("(cached)".to_string(), false),
            CatalogSource::Builtin => ("(built-in list)".to_string(), false),
        },
    };
    PickerEntry::Header {
        runtime,
        text: format!("{upper}  {what}"),
        missing,
    }
}

fn entry(runtime: Runtime, m: &CatalogModel, current: Option<&ModelRef>) -> Option<PickerEntry> {
    let model = ModelRef::parse(&format!("{}:{}", runtime.label(), m.id)).ok()?;
    let label = Some(clean(&m.label))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| clean(&m.id));
    let mut description = clean(&m.description);
    if m.is_default {
        let mark = format!("({} default)", runtime_name(runtime));
        description = if description.is_empty() {
            mark
        } else {
            format!("{description} {mark}")
        };
    }
    Some(PickerEntry::Model {
        current: current == Some(&model),
        model,
        label,
        description,
        efforts: efforts_text(&efforts_of(m)),
    })
}

fn entries(
    catalogs: &Catalogs,
    current: Option<&ModelRef>,
    first: Option<PickerEntry>,
) -> Vec<PickerEntry> {
    let mut out: Vec<PickerEntry> = first.into_iter().collect();
    for runtime in RUNTIMES {
        let catalog = catalogs.of(runtime);
        out.push(header(runtime, catalog));
        let from = out.len();
        for m in catalog.map(|c| c.models.as_slice()).unwrap_or_default() {
            out.extend(entry(runtime, m, current));
        }
        // Fix round 1 (controller ruling): a current model no catalog entry is, under its
        // runtime, so `⏎` keeps it and never swaps in another.
        let listed = out[from..]
            .iter()
            .any(|e| matches!(e, PickerEntry::Model { current: true, .. }));
        if let Some(m) = current.filter(|m| m.runtime == runtime && !listed) {
            out.push(PickerEntry::Model {
                model: m.clone(),
                label: m.id.as_deref().map_or_else(|| "default".into(), clean),
                description: NOT_REPORTED.into(),
                efforts: NO_EFFORT.into(),
                current: true,
            });
        }
    }
    out.push(PickerEntry::Custom);
    out
}

impl ModelPicker {
    pub fn new(
        target: PickerFor,
        catalogs: &Catalogs,
        current: Option<&ModelRef>,
        first: Option<PickerEntry>,
    ) -> ModelPicker {
        let mut p = ModelPicker {
            target,
            entries: entries(catalogs, current, first),
            selected: 0,
            custom: None,
            current: current.cloned(),
        };
        let on_current = (0..p.entries.len()).find(|&i| {
            p.selectable(i) && matches!(p.entries[i], PickerEntry::Model { current: true, .. })
        });
        p.selected = on_current.or_else(|| p.next_from(0, 1)).unwrap_or(0);
        p
    }

    /// A new `Models` reply (decision 37): the entries rebuilt from `catalogs`, the
    /// selection kept on the same model (or the same first entry, or `custom…`).
    pub fn refresh(&mut self, catalogs: &Catalogs) {
        let first = (self.entries.first())
            .filter(|e| matches!(e, PickerEntry::NoFallback | PickerEntry::RoleTable(_)))
            .cloned();
        let on = self.entries.get(self.selected).cloned();
        let current = self.current.clone();
        let mut fresh = ModelPicker::new(self.target, catalogs, current.as_ref(), first);
        let same = (0..fresh.entries.len()).find(|&i| {
            fresh.selectable(i)
                && match (&on, &fresh.entries[i]) {
                    (
                        Some(PickerEntry::Model { model: a, .. }),
                        PickerEntry::Model { model: b, .. },
                    ) => a == b,
                    (Some(PickerEntry::Model { .. }), _) | (None, _) => false,
                    (Some(a), b) => std::mem::discriminant(a) == std::mem::discriminant(b),
                }
        });
        if let Some(i) = same {
            fresh.selected = i;
        }
        fresh.custom = self.custom.take();
        *self = fresh;
    }

    /// Whether entry `i` can be selected: not a header, not a missing runtime's model.
    pub fn selectable(&self, i: usize) -> bool {
        match self.entries.get(i) {
            None | Some(PickerEntry::Header { .. }) => false,
            // Under its runtime's header: greyed when that CLI is missing.
            Some(PickerEntry::Model { .. }) => !(self.entries[..i].iter().rev())
                .find(|e| matches!(e, PickerEntry::Header { .. }))
                .is_some_and(|h| matches!(h, PickerEntry::Header { missing: true, .. })),
            Some(_) => true,
        }
    }

    /// The first selectable entry from `from` on, stepping by `step` (±1).
    fn next_from(&self, from: usize, step: isize) -> Option<usize> {
        let mut i = from as isize;
        while i >= 0 && (i as usize) < self.entries.len() {
            if self.selectable(i as usize) {
                return Some(i as usize);
            }
            i += step;
        }
        None
    }

    /// A key while the picker is open: what was picked, or `None` to stay open.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<Picked> {
        if self.custom.is_some() {
            return self.custom_key(key);
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(i) = self.next_from(self.selected + 1, 1) {
                    self.selected = i;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(i) = self
                    .selected
                    .checked_sub(1)
                    .and_then(|s| self.next_from(s, -1))
                {
                    self.selected = i;
                }
            }
            KeyCode::Char('r') => return Some(Picked::Refresh),
            KeyCode::Esc => return Some(Picked::Cancel),
            KeyCode::Enter if self.selectable(self.selected) => {
                return match &self.entries[self.selected] {
                    PickerEntry::Model { model, .. } => Some(Picked::Model(model.clone())),
                    PickerEntry::NoFallback => Some(Picked::NoFallback),
                    PickerEntry::RoleTable(_) => Some(Picked::RoleTable),
                    PickerEntry::Custom => {
                        let runtime =
                            (self.current.as_ref()).map_or(Runtime::Claude, |m| m.runtime);
                        self.custom = Some(CustomModel {
                            runtime,
                            name: TextArea::new(),
                            naming: false,
                            error: None,
                        });
                        None
                    }
                    PickerEntry::Header { .. } => None,
                };
            }
            _ => {}
        }
        None
    }

    /// `custom…`: `←`/`→` choose the runtime and `⏎` moves to the name; there `⏎`
    /// accepts only what `ModelRef::parse` accepts, showing its error. `esc` goes back
    /// to the list.
    fn custom_key(&mut self, key: KeyEvent) -> Option<Picked> {
        let c = self.custom.as_mut()?;
        if key.code == KeyCode::Esc {
            self.custom = None;
            return None;
        }
        if !c.naming {
            match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ' | 'h' | 'l') => {
                    c.runtime = match c.runtime {
                        Runtime::Codex => Runtime::Claude,
                        _ => Runtime::Codex,
                    };
                }
                KeyCode::Enter => c.naming = true,
                _ => {}
            }
            return None;
        }
        match key.code {
            KeyCode::Enter => {
                let name = c.name.text().trim().to_string();
                match ModelRef::parse(&format!("{}:{name}", c.runtime.label())) {
                    Ok(model) => return Some(Picked::Model(model)),
                    Err(e) => c.error = Some(e),
                }
            }
            // One line: Ctrl-J is the text area's newline, not this field's.
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            _ => {
                if c.name.on_key(key) {
                    c.error = None;
                }
            }
        }
        None
    }

    /// A paste reaches the custom name, as one line, or nothing.
    pub fn on_paste(&mut self, text: &str) {
        if let Some(c) = self.custom.as_mut().filter(|c| c.naming) {
            c.name.on_paste(&text.replace(['\r', '\n'], " "));
            c.error = None;
        }
    }
}

#[cfg(test)]
#[path = "model_picker_tests.rs"]
pub(crate) mod tests;
