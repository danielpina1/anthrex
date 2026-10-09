//! Milestone 9.8 decision 40 (M9.8.12): the `[models]` writer. Pure.
//!
//! Each row is a `[models.<row>]` table (`parse.rs`'s shapes). A row the file already
//! holds as the document has it is left byte for byte; a changed one is edited in
//! place, its lines' comments kept; a row the document drops loses its keys (and its
//! header, once empty); a new one follows the last `[models.*]` table, or ends the
//! file. The old keys go in the same edit (`write_old.rs`). A `models` key written in
//! any other form (an inline table, a dotted key, an array of tables) refuses the save
//! with [`UNSUPPORTED_FORM`], as does an old key the removal could not find.

use std::collections::BTreeSet;

use proto::models::{ModelTable, Role};

use super::lines::indent_of;
use super::save::{UNOWNED_CHANGED, unowned};
use super::scan::{Edits, Scan, Section};
use super::write::UNSUPPORTED_FORM;
use crate::models::{
    Kept, kept as kept_keys, migrate, old_keys, read_table, render, resolve, resolve_brainstorm,
};

/// What [`edit`] made of a text.
pub(super) struct Edited {
    pub text: String,
    /// At least one old key was removed (decision 40's `.bak`).
    pub removed: bool,
    pub kept: Kept,
}

/// A `[models.*]` table that holds a row, or the brainstorm pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    Row(Role),
    Brainstorm,
}

const ROW_KEYS: [&str; 3] = ["model", "effort", "fallback"];
const BRAINSTORM_KEYS: [&str; 3] = ["first", "second", "effort"];

impl Slot {
    fn of(path: &[String]) -> Option<Slot> {
        let (first, rest) = path.split_first()?;
        if first != "models" || rest.is_empty() {
            return None;
        }
        // Fix round 1 (M3): by segment, so a quoted `"implementer.small"` is no row.
        if rest.len() == 1 && rest[0] == "brainstorm" {
            return Some(Slot::Brainstorm);
        }
        Role::all()
            .find(|r| r.key().split('.').eq(rest.iter().map(String::as_str)))
            .map(Slot::Row)
    }

    fn keys(self) -> [&'static str; 3] {
        match self {
            Slot::Row(_) => ROW_KEYS,
            Slot::Brainstorm => BRAINSTORM_KEYS,
        }
    }

    /// The slot's keys and their values in `table`, `None` without the row.
    fn values(self, table: &ModelTable) -> Option<Vec<(&'static str, String)>> {
        let mut out = Vec::new();
        match self {
            Slot::Row(role) => {
                let c = table.rows.get(&role)?;
                out.push(("model", c.model.label()));
                out.extend(c.effort.clone().map(|e| ("effort", e)));
                out.extend(c.fallback.as_ref().map(|f| ("fallback", f.label())));
            }
            Slot::Brainstorm => {
                let b = table.brainstorm.as_ref()?;
                out.push(("first", b.first.label()));
                out.push(("second", b.second.label()));
                out.extend(b.effort.clone().map(|e| ("effort", e)));
            }
        }
        Some(out)
    }

    fn only(self, table: &ModelTable) -> ModelTable {
        let mut out = ModelTable::default();
        match self {
            Slot::Row(role) => {
                if let Some(c) = table.rows.get(&role) {
                    out.rows.insert(role, c.clone());
                }
            }
            Slot::Brainstorm => out.brainstorm = table.brainstorm.clone(),
        }
        out
    }
}

fn slots(table: &ModelTable) -> BTreeSet<Slot> {
    let rows = table.rows.keys().map(|r| Slot::Row(*r));
    rows.chain(table.brainstorm.as_ref().map(|_| Slot::Brainstorm))
        .collect()
}

/// The save's models step (decision 40): `limits` (the file after the limits edit) with
/// `[models]` set to `roles` and the old keys removed, proved by reading it back
/// against `original`. A row `roles` leaves out that the file's remaining keys would
/// still migrate to another model (a `deciders.mode` of `codex` with the helpers row
/// reset, say) is written as the model `roles` resolves to, so what runs is what the
/// screen showed.
pub(super) fn write(
    original: &str,
    limits: &str,
    roles: &ModelTable,
) -> Result<Edited, Vec<String>> {
    let mut wanted = roles.clone();
    let mut edited = edit(limits, &wanted)?;
    let extra = unwanted(&edited.text, &wanted);
    if !extra.is_empty() {
        wanted = wanted.overlaid(&extra);
        edited = edit(limits, &wanted)?;
    }
    read_back(original, &edited, &wanted)?;
    Ok(edited)
}

/// The rows (and brainstorm pair) `wanted` leaves out that `text` reads as something
/// else: each as `wanted` resolves it.
fn unwanted(text: &str, wanted: &ModelTable) -> ModelTable {
    let mut read = crate::parse(text).0.orchestrator.roles;
    let mut extra = ModelTable::default();
    // `Role::all` lists `helpers` before its kinds, so a fixed `helpers` fixes them.
    for role in Role::all() {
        let want = resolve(role, None, wanted);
        if !wanted.rows.contains_key(&role) && resolve(role, None, &read) != want {
            read.rows.insert(role, want.clone());
            extra.rows.insert(role, want);
        }
    }
    let want = resolve_brainstorm(None, wanted);
    if wanted.brainstorm.is_none() && resolve_brainstorm(None, &read) != want {
        extra.brainstorm = Some(want);
    }
    extra
}

/// Decision 40's proof: `edited` reads every row of `wanted` back as it is, every role
/// resolves as `wanted` resolves it, no note is left but a kept key's, and nothing but
/// the limits, `[models]` and the old keys differs from `original`.
fn read_back(original: &str, edited: &Edited, wanted: &ModelTable) -> Result<(), Vec<String>> {
    let after = crate::parse(&edited.text).0.orchestrator;
    let rows_kept = wanted
        .rows
        .iter()
        .all(|(r, c)| after.roles.rows.get(r) == Some(c))
        && (wanted.brainstorm.is_none() || after.roles.brainstorm == wanted.brainstorm);
    let resolved = Role::all().all(|r| resolve(r, None, &after.roles) == resolve(r, None, wanted))
        && resolve_brainstorm(None, &after.roles) == resolve_brainstorm(None, wanted);
    // Fix round 1 (M2): the only notes left are a kept key's and decision 16's.
    let kept = edited.kept.unmigrated.iter().chain(&edited.kept.with);
    let named: Vec<String> = kept.map(|k| format!("config: {}", k.named)).collect();
    let quiet = after.roles_notes.iter().all(|n| {
        named.iter().any(|k| n.starts_with(k.as_str()))
            || (!edited.kept.is_empty() && n.starts_with("config: orchestrator.default_runtime = "))
    });
    if !(rows_kept && resolved && quiet) {
        return Err(vec![
            "config.toml would not read back as saved (models); nothing changed".to_string(),
        ]);
    }
    let before: toml::Table = original.parse().unwrap_or_default();
    let now: toml::Table = edited.text.parse().unwrap_or_default();
    if unowned(before, true) != unowned(now, true) {
        return Err(vec![UNOWNED_CHANGED.to_string()]);
    }
    Ok(())
}

/// `text` (valid TOML) with `[models]` set to `wanted` and the old keys removed, but
/// those [`kept`] holds. Refuses an unsupported form with problems.
pub(super) fn edit(text: &str, wanted: &ModelTable) -> Result<Edited, Vec<String>> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?;
    let raw = table.get("orchestrator").and_then(toml::Value::as_table);
    let kept = match raw {
        Some(raw) => {
            let o = crate::parse(text).0.orchestrator;
            let migrated = migrate(&o, Some(raw)).0;
            kept_keys(&o, raw, &migrated, wanted)
        }
        None => Kept::default(),
    };
    let scan = Scan::new(text);
    let refused = refused_forms(&scan);
    if !refused.is_empty() {
        return Err(refused);
    }
    let mut e = Edits::default();
    let removed = super::write_old::remove(&scan, &mut e, &kept);
    let file = (table.get("models"))
        .map(|v| read_table(v, &mut Vec::new()))
        .unwrap_or_default();
    let appended = edit_rows(&scan, &mut e, &file, wanted);
    let out = scan.emit(&e, appended);
    left_over(&out, &kept)?;
    Ok(Edited {
        text: out,
        removed,
        kept,
    })
}

/// Edits the rows in place; returns the new rows' block when no `[models.*]` table
/// exists to follow.
fn edit_rows(
    scan: &Scan,
    e: &mut Edits,
    file: &ModelTable,
    wanted: &ModelTable,
) -> Vec<Vec<String>> {
    let mut placed = BTreeSet::new();
    let mut last = None;
    for s in scan
        .sections
        .iter()
        .filter(|s| s.header.is_some() && !s.array)
    {
        let Some(slot) = Slot::of(&s.path) else {
            continue;
        };
        placed.insert(slot);
        last = last.max(scan.end_of(s));
        let (want, have) = (slot.values(wanted), slot.values(file));
        if want == have {
            continue;
        }
        match want {
            // A row the parser skipped (a bad value) is the user's to fix; left alone.
            None if have.is_none() => {}
            None => drop_row(scan, e, s, slot),
            Some(want) => set_row(scan, e, s, slot, &want, have.as_deref()),
        }
    }
    let mut new = ModelTable::default();
    for slot in slots(wanted).difference(&placed) {
        new = new.overlaid(&slot.only(wanted));
    }
    if new.is_empty() {
        return Vec::new();
    }
    let lines: Vec<String> = render(&new).lines().map(str::to_string).collect();
    match last {
        Some(n) => {
            let after = e.after.entry(n).or_default();
            after.push(String::new());
            after.extend(lines);
            Vec::new()
        }
        None => vec![lines],
    }
}

fn toml_string(v: &str) -> String {
    toml::Value::String(v.to_string()).to_string()
}

fn set_row(
    scan: &Scan,
    e: &mut Edits,
    s: &Section,
    slot: Slot,
    want: &[(&'static str, String)],
    have: Option<&[(&'static str, String)]>,
) {
    let value = |list: Option<&[(&'static str, String)]>, k: &str| {
        list.and_then(|l| l.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone()))
    };
    let anchor = scan.end_of(s).expect("a row table has a header");
    let indent = match s.keys.last() {
        Some(&i) => indent_of(&scan.lines[i].0).to_string(),
        None => String::new(),
    };
    for k in slot.keys() {
        let (new, old) = (value(Some(want), k), value(have, k));
        match (scan.line_in(s, k), new) {
            (Some(_), Some(v)) if old.as_deref() == Some(v.as_str()) => {}
            (Some(i), Some(v)) => {
                let (_, _, span) = scan.key_of(i);
                let (a, b) = span.expect("a supported line has a single-line value");
                let line = &scan.lines[i].0;
                e.replace.insert(
                    i,
                    format!("{}{}{}", &line[..a], toml_string(&v), &line[b..]),
                );
            }
            (Some(i), None) => scan.delete_key(e, i),
            (None, Some(v)) => {
                let line = format!("{indent}{k} = {}", toml_string(&v));
                e.after.entry(anchor).or_default().push(line);
            }
            (None, None) => {}
        }
    }
}

/// The row's keys go; its header too when nothing else is left under it.
fn drop_row(scan: &Scan, e: &mut Edits, s: &Section, slot: Slot) {
    for &i in &s.keys {
        let (key, _, _) = scan.key_of(i);
        if key.len() == 1 && slot.keys().contains(&key[0].as_str()) {
            scan.delete_key(e, i);
        }
    }
    if s.keys.iter().all(|i| e.delete.contains(i)) {
        e.delete.insert(s.header.expect("a row table has a header"));
    }
}

fn refusal(key: &str, table: &str) -> String {
    UNSUPPORTED_FORM
        .replace("{key}", key)
        .replace("{table}", table)
}

/// Every `models` key or table written in a form [`edit`] does not edit.
fn refused_forms(scan: &Scan) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |text: String| {
        if !out.contains(&text) {
            out.push(text);
        }
    };
    for s in &scan.sections {
        let models = s.path.first().is_some_and(|p| p == "models");
        if s.header.is_some() && s.array && models {
            push(refusal(&s.path.join("."), &home(&s.path)));
            continue;
        }
        let slot = Slot::of(&s.path).filter(|_| s.header.is_some() && !s.array);
        for &i in &s.keys {
            let (key, last, _) = scan.key_of(i);
            let full: Vec<String> = s.path.iter().chain(key).cloned().collect();
            if full.first().is_none_or(|p| p != "models") {
                continue;
            }
            // Fix round 1 (M4): a row written inline in its parent's table
            // (`run_name = { … }` under `[models.helpers]`).
            if slot.is_none() || key.len() != 1 || last != i || Slot::of(&full).is_some() {
                let shown = match slot {
                    Some(_) => full.join("."),
                    // A table that names no row (`[models."implementer.small"]`).
                    None if s.path.len() >= 2 => s.path.join("."),
                    None => full[..(s.path.len() + 1).min(full.len())].join("."),
                };
                push(refusal(&shown, &home(&full)));
            }
        }
    }
    out
}

/// Where a `models` key belongs: the longest row table its path names, else
/// `models.<row>`.
fn home(path: &[String]) -> String {
    (2..=path.len())
        .rev()
        .find(|&n| Slot::of(&path[..n]).is_some())
        .map_or_else(|| "models.<row>".to_string(), |n| path[..n].join("."))
}

/// Decision 40: an old key the removal could not find (another form) refuses.
fn left_over(out: &str, kept: &Kept) -> Result<(), Vec<String>> {
    let table: toml::Table = out
        .parse()
        .map_err(|e| vec![format!("the edited config.toml would not parse: {e}")])?;
    let Some(raw) = table.get("orchestrator").and_then(toml::Value::as_table) else {
        return Ok(());
    };
    let problems: Vec<String> = old_keys(raw)
        .into_iter()
        .filter(|k| !kept.holds(&k.path))
        .map(|k| {
            let home = match k.path.as_str() {
                "orchestrator.models" => "[orchestrator.models]".to_string(),
                p if p.starts_with("orchestrator.routes.") => p.to_string(),
                p => p.rsplit_once('.').map_or(p, |(t, _)| t).to_string(),
            };
            refusal(&k.path, &home)
        })
        .collect();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}
