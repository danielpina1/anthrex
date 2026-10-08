//! Decision 30: the line-based writer. `toml_edit` is not in the tree, so the writer
//! edits the text line by line and proves its own output by reading it back.
//!
//! It knows exactly two shapes: an owned scalar as a single-line `key = value` under its
//! own `[table]` header, and every other line, which it never touches. Milestone 9.8
//! (M9.8.12) moved the roster and the orchestrator default out: the role table is
//! written by `write_models.rs`, and the old keys removed by `write_old.rs`. Headers and keys are recognised after leading whitespace (preflight
//! F17). An owned key or table written any other way refuses the whole save with
//! [`UNSUPPORTED_FORM`].

use std::collections::BTreeMap;

use proto::SettingsDoc;

use super::lines::indent_of;
use super::origin::lookup;
use super::scan::{Edits, Scan, Section, is, split};
use crate::Orchestrator;

/// Decision 30's refusal; `{key}` is the key or table as written, `{table}` where it
/// belongs.
pub const UNSUPPORTED_FORM: &str = "{key} is written in a form the settings screen does not edit (a dotted key, an inline table or a multi-line value); edit config.toml by hand, or move it under [{table}]";

/// The owned scalars, `(table, key)`, in the order a missing key is inserted.
const SCALARS: [(&str, &str); 8] = [
    ("orchestrator", "max_writers"),
    ("orchestrator", "max_readers"),
    ("orchestrator", "max_bounces"),
    ("orchestrator", "stall_after_secs"),
    ("orchestrator.budget.s", "tool_calls"),
    ("orchestrator.budget.s", "minutes"),
    ("orchestrator.budget.m", "tool_calls"),
    ("orchestrator.budget.m", "minutes"),
];
const BUDGET_L: [(&str, &str); 2] = [
    ("orchestrator.budget.l", "tool_calls"),
    ("orchestrator.budget.l", "minutes"),
];
/// Tables only the writer may shape.
const CONTAINERS: [&str; 5] = [
    "orchestrator",
    "orchestrator.budget",
    "orchestrator.budget.s",
    "orchestrator.budget.m",
    "orchestrator.budget.l",
];

/// Pure: `text` with every owned key set as `doc` says (decision 30). Refuses with
/// problems on an unsupported form, on invalid TOML, or on an output that does not read
/// back as `doc` with every other key unchanged.
pub(crate) fn edit_text(text: &str, doc: &SettingsDoc) -> Result<String, Vec<String>> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?;
    let scan = Scan::new(text);
    check(&scan, &table)?;
    let out = edit(&scan, &table, doc);
    super::save::read_back(&table, &out, doc)?;
    Ok(out)
}

fn scalars() -> impl Iterator<Item = (&'static str, &'static str)> {
    SCALARS.into_iter().chain(BUDGET_L)
}

fn is_scalar(path: &[String]) -> bool {
    scalars().any(|(t, k)| {
        path.len() >= 2 && is(&path[..path.len() - 1], t) && path[path.len() - 1] == k
    })
}

fn is_container(path: &[String]) -> bool {
    CONTAINERS.iter().any(|c| is(path, c))
}

/// Where an owned key or table belongs, for [`UNSUPPORTED_FORM`]'s `{table}`.
fn home(path: &[String]) -> String {
    let seg = |i: usize| path.get(i).map(String::as_str);
    match (seg(1), seg(2)) {
        (Some("budget"), Some(r @ ("s" | "m" | "l"))) => format!("orchestrator.budget.{r}"),
        (Some("budget"), _) => "orchestrator.budget.<s|m|l>".to_string(),
        _ => "orchestrator".to_string(),
    }
}

fn refusal(path: &[String]) -> String {
    UNSUPPORTED_FORM
        .replace("{key}", &path.join("."))
        .replace("{table}", &home(path))
}

/// Decision 30's supported forms, checked on the lines and against the parsed table.
fn check(scan: &Scan, table: &toml::Table) -> Result<(), Vec<String>> {
    let mut refused = refused_lines(scan);
    if refused.is_empty() {
        // Whatever the lines missed, the parsed table still names.
        for (t, k) in scalars() {
            let path = format!("{t}.{k}");
            if lookup(table, &path).is_some() && scan.scalar_line(t, k).is_none() {
                refused.push(split(&path));
            }
        }
    }
    let mut problems: Vec<String> = Vec::new();
    for text in refused.iter().map(|p| refusal(p)) {
        if !problems.contains(&text) {
            problems.push(text);
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// Every owned key or table whose header or key line has an unsupported form. A refused
/// header's own keys are not reported again.
fn refused_lines(scan: &Scan) -> Vec<Vec<String>> {
    let mut refused = Vec::new();
    for s in &scan.sections {
        if s.header.is_some() {
            let h = &s.path;
            let bad = if s.array {
                is_container(h) || is_scalar(h)
            } else {
                is(h, "orchestrator.budget") || is_scalar(h)
            };
            if bad {
                refused.push(h.clone());
                continue;
            }
        }
        refused.extend(refused_keys(scan, s));
    }
    refused
}

fn refused_keys(scan: &Scan, s: &Section) -> Vec<Vec<String>> {
    let mut refused = Vec::new();
    for &i in &s.keys {
        let (key, last, _) = scan.key_of(i);
        let full: Vec<String> = s.path.iter().chain(key).cloned().collect();
        let bad = if is_scalar(&full) {
            key.len() != 1 || last != i || s.array
        } else {
            is_container(&full)
                || (s.path.len()..full.len() - 1)
                    .map(|j| &full[..=j])
                    .any(|p| is_container(p) || is_scalar(p))
        };
        if bad {
            refused.push(full);
        }
    }
    refused
}

fn edit(scan: &Scan, table: &toml::Table, doc: &SettingsDoc) -> String {
    let mut e = Edits::default();
    let defaults = wanted(&super::doc_of(&Orchestrator::default()));
    let want = wanted(doc);
    let mut missing: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (t, k) in scalars() {
        let path = format!("{t}.{k}");
        let Some(v) = want.get(path.as_str()) else {
            continue;
        };
        match scan.scalar_line(t, k) {
            Some(i) if lookup(table, &path) != Some(v) => {
                let (_, _, span) = scan.key_of(i);
                let (a, b) = span.expect("a supported line has a single-line value");
                let line = &scan.lines[i].0;
                e.replace
                    .insert(i, format!("{}{v}{}", &line[..a], &line[b..]));
            }
            None if defaults.get(path.as_str()) != Some(v) => {
                missing.entry(t).or_default().push(format!("{k} = {v}"));
            }
            _ => {}
        }
    }
    let mut appended: Vec<Vec<String>> = Vec::new();
    for t in CONTAINERS {
        let Some(keys) = missing.remove(t) else {
            continue;
        };
        match scan.table_section(t) {
            Some(s) => {
                let anchor = scan.end_of(s).expect("a table section has a header");
                let indent = indent_of(&scan.lines[anchor].0);
                let lines = keys.into_iter().map(|k| format!("{indent}{k}"));
                e.after.entry(anchor).or_default().extend(lines);
            }
            None => {
                let mut block = vec![format!("[{t}]")];
                block.extend(keys);
                appended.push(block);
            }
        }
    }
    scan.emit(&e, appended)
}

/// Each owned scalar's wanted value.
fn wanted(doc: &SettingsDoc) -> BTreeMap<&'static str, toml::Value> {
    let l = &doc.limits;
    let int = |n: u64| toml::Value::Integer(n as i64);
    BTreeMap::from([
        ("orchestrator.max_writers", int(l.max_writers.into())),
        ("orchestrator.max_readers", int(l.max_readers.into())),
        ("orchestrator.max_bounces", int(l.max_bounces.into())),
        ("orchestrator.stall_after_secs", int(l.stall_after_secs)),
        (
            "orchestrator.budget.s.tool_calls",
            int(l.budget_s.tool_calls.into()),
        ),
        (
            "orchestrator.budget.s.minutes",
            int(l.budget_s.minutes.into()),
        ),
        (
            "orchestrator.budget.m.tool_calls",
            int(l.budget_m.tool_calls.into()),
        ),
        (
            "orchestrator.budget.m.minutes",
            int(l.budget_m.minutes.into()),
        ),
        (
            "orchestrator.budget.l.tool_calls",
            int(l.budget_l.tool_calls.into()),
        ),
        (
            "orchestrator.budget.l.minutes",
            int(l.budget_l.minutes.into()),
        ),
    ])
}
