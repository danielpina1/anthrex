//! Milestone 9.8 decisions 18 and 40 (M9.8.12): the old model keys a file holds, which
//! a save removes, and the few it keeps. Pure.
//!
//! **Kept keys** (Task 3's review, fix round 1's I1): a list none of whose models is
//! known, and a key whose every row has no row in the migrated table nor in the table
//! being saved (decision 16's `default_runtime`), could not be migrated.
//! Removing it would lose the user's setting silently, so a save keeps it, its note
//! says why, and the keys it was read against (the roster and `default_runtime`) stay
//! with it, so it reads the same afterwards. Choosing its row in C-b S and saving
//! removes them all.

use proto::models::{ModelTable, Role};

use super::migrate::{ROUTES, SCALARS};
use super::{resolve, resolve_brainstorm};
use crate::Orchestrator;

/// What an old key fed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Feeds {
    Rows(Vec<Role>),
    Brainstorm,
    /// `[[orchestrator.models]]` and `builtin_models`: replaced by the CLIs' lists.
    Roster,
}

/// One old key present in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldKey {
    /// Its dotted path under the file's root (`orchestrator.routes.hub`).
    pub path: String,
    /// As decision 18's notes name it (`[orchestrator.routes.hub]`,
    /// `[[orchestrator.models]]`, `orchestrator.agent.model`).
    pub named: String,
    pub feeds: Feeds,
}

const CLASSES: [Role; 3] = [
    Role::ImplementerSmall,
    Role::ImplementerMedium,
    Role::ImplementerHub,
];

fn feeds_row(row: &str) -> Feeds {
    Role::parse_key(row).map_or(Feeds::Brainstorm, |r| Feeds::Rows(vec![r]))
}

/// The keys of the table `table` (dotted, `orchestrator.agent`) that fed a row, from
/// the migration's own list (fix round 1, M11: one list, so the writer and the
/// migration cannot drift apart).
pub fn old_lines(table: &str) -> &'static [&'static str] {
    if table == "orchestrator" {
        return &["default_runtime", "builtin_models"];
    }
    let sub = table.strip_prefix("orchestrator.").unwrap_or_default();
    (SCALARS.iter())
        .find(|(t, _, _)| *t == sub)
        .map_or(&[], |(_, keys, _)| *keys)
}

/// Whether `[orchestrator.routes.<name>]` is one decision 18 replaces.
pub fn is_route_name(name: &str) -> bool {
    ROUTES.iter().any(|(n, _)| *n == name)
}

/// Every old key in `raw`, the file's `[orchestrator]` table, in its key order.
pub fn old_keys(raw: &toml::Table) -> Vec<OldKey> {
    let key = |path: String, named: String, feeds| OldKey { path, named, feeds };
    let mut out = Vec::new();
    for (name, value) in raw {
        match name.as_str() {
            "models" => out.push(key(
                "orchestrator.models".into(),
                "[[orchestrator.models]]".into(),
                Feeds::Roster,
            )),
            "builtin_models" | "default_runtime" => {
                let path = format!("orchestrator.{name}");
                let feeds = match name.as_str() {
                    "builtin_models" => Feeds::Roster,
                    _ => Feeds::Rows(CLASSES.to_vec()),
                };
                out.push(key(path.clone(), path, feeds));
            }
            "routes" => {
                for route in value.as_table().into_iter().flat_map(|t| t.keys()) {
                    if let Some((_, row)) = ROUTES.iter().find(|(n, _)| n == route) {
                        let path = format!("orchestrator.routes.{route}");
                        out.push(key(path.clone(), format!("[{path}]"), feeds_row(row)));
                    }
                }
            }
            table => {
                let Some((_, keys, row)) = SCALARS.iter().find(|(t, _, _)| *t == table) else {
                    continue;
                };
                for sub in value.as_table().into_iter().flat_map(|t| t.keys()) {
                    if keys.contains(&sub.as_str()) {
                        let path = format!("orchestrator.{table}.{sub}");
                        out.push(key(path.clone(), path, feeds_row(row)));
                    }
                }
            }
        }
    }
    out
}

/// The old keys a save keeps (see the module doc): the unmigrated ones, then the
/// roster and `default_runtime` they are read against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    pub unmigrated: Vec<OldKey>,
    pub with: Vec<OldKey>,
}

impl Kept {
    pub fn is_empty(&self) -> bool {
        self.unmigrated.is_empty()
    }

    pub fn holds(&self, path: &str) -> bool {
        self.unmigrated
            .iter()
            .chain(&self.with)
            .any(|k| k.path == path)
    }
}

/// [`Kept`] for a file whose `[orchestrator]` table is `raw` (read as `o`), whose old
/// keys migrated to `migrated`, saved (or read) with the table `wanted`.
///
/// A list is unmigrated when none of its models is known (fix round 1, I1) and the
/// table still holds what the migration made of its row: choosing another model for
/// the row in C-b S lets the next save remove it. Any other key is unmigrated when
/// none of its rows exists (decision 16's `default_runtime`).
pub fn kept(
    o: &Orchestrator,
    raw: &toml::Table,
    migrated: &ModelTable,
    wanted: &ModelTable,
) -> Kept {
    let unknown = super::migrate::unknown_lists(o, raw);
    let has = |r: &Role| migrated.rows.contains_key(r) || wanted.rows.contains_key(r);
    let untouched = |k: &OldKey| match &k.feeds {
        Feeds::Rows(rows) => rows
            .iter()
            .all(|r| migrated.rows.get(r) == wanted.rows.get(r)),
        Feeds::Brainstorm => migrated.brainstorm == wanted.brainstorm,
        Feeds::Roster => false,
    };
    let unmigrated = |k: &OldKey| match k.path.strip_prefix("orchestrator.routes.") {
        Some(name) => unknown.contains(&name) && untouched(k),
        None => match &k.feeds {
            Feeds::Rows(rows) => !rows.iter().any(has),
            _ => false,
        },
    };
    let (unmigrated, rest): (Vec<OldKey>, Vec<OldKey>) =
        old_keys(raw).into_iter().partition(unmigrated);
    if unmigrated.is_empty() {
        return Kept::default();
    }
    let with = (rest.into_iter())
        .filter(|k| k.feeds == Feeds::Roster || k.path == "orchestrator.default_runtime")
        .collect();
    Kept { unmigrated, with }
}

/// Decision 18's notes with each kept key's note saying why it stays (Task 3's review).
pub fn kept_notes(notes: &mut Vec<String>, kept: &Kept, wanted: &ModelTable) {
    let rows = |rows: &[Role]| {
        let named: Vec<String> = rows
            .iter()
            .map(|r| format!("[models.{}]", r.key()))
            .collect();
        match named.as_slice() {
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => String::new(),
        }
    };
    let mut why = Vec::new();
    for k in &kept.unmigrated {
        let note = match &k.feeds {
            Feeds::Rows(r) if k.path == "orchestrator.default_runtime" => format!(
                "config: {} could not be migrated: the roster has no model of that runtime, so {} keep their built-ins; a save keeps the key until you choose those rows in C-b S",
                k.named,
                rows(r)
            ),
            Feeds::Rows(r) => {
                let uses = r.first().map(|r| resolve(*r, None, wanted).model.label());
                format!(
                    "config: {}: none of its models are known; {} uses {} until you choose one in C-b S",
                    k.named,
                    rows(r),
                    uses.unwrap_or_default()
                )
            }
            _ => {
                let b = resolve_brainstorm(None, wanted);
                format!(
                    "config: {}: none of its models are known; [models.brainstorm] uses {} and {} until you choose one in C-b S",
                    k.named,
                    b.first.label(),
                    b.second.label()
                )
            }
        };
        why.push((k.named.clone(), note));
    }
    let lists: Vec<&str> = kept.unmigrated.iter().map(|k| k.named.as_str()).collect();
    let verb = if lists.len() == 1 { "is" } else { "are" };
    for k in &kept.with {
        let note = format!(
            "config: {} is kept with {}, which {verb} read against it",
            k.named,
            lists.join(" and ")
        );
        why.push((k.named.clone(), note));
    }
    for (named, note) in why {
        let prefix = format!("config: {named} is replaced by ");
        match notes.iter_mut().find(|n| n.starts_with(&prefix)) {
            Some(n) => *n = note,
            None => notes.push(note),
        }
    }
}
