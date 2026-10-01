//! Decision 30: the line-based writer. `toml_edit` is not in the tree, so the writer
//! edits the text line by line and proves its own output by reading it back.
//!
//! It knows exactly three shapes: an owned scalar as a single-line `key = value` under its
//! own `[table]` header, a `[[orchestrator.models]]` block, and every other line, which it
//! never touches. Headers and keys are recognised after leading whitespace (preflight
//! F17). An owned key or table written any other way refuses the whole save with
//! [`UNSUPPORTED_FORM`].

use std::collections::{BTreeMap, BTreeSet};

use proto::{ModelEntry, SettingsDoc};

use super::lines::{Kind, classify, indent_of};
use super::origin::lookup;
use super::validate::strength_name;
use crate::Orchestrator;

/// Decision 30's refusal; `{key}` is the key or table as written, `{table}` where it
/// belongs.
pub const UNSUPPORTED_FORM: &str = "{key} is written in a form the settings screen does not edit (a dotted key, an inline table or a multi-line value); edit config.toml by hand, or move it under [{table}]";

/// The owned scalars, `(table, key)`, in the order a missing key is inserted.
const SCALARS: [(&str, &str); 11] = [
    ("orchestrator", "builtin_models"),
    ("orchestrator", "max_writers"),
    ("orchestrator", "max_readers"),
    ("orchestrator", "max_bounces"),
    ("orchestrator", "stall_after_secs"),
    ("orchestrator.agent", "runtime"),
    ("orchestrator.agent", "model"),
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
const CONTAINERS: [&str; 7] = [
    "orchestrator",
    "orchestrator.agent",
    "orchestrator.budget",
    "orchestrator.budget.s",
    "orchestrator.budget.m",
    "orchestrator.budget.l",
    "orchestrator.models",
];
const MODELS: &str = "orchestrator.models";

/// Pure: `text` with every owned key set as `doc` says (decision 30). Refuses with
/// problems on an unsupported form, on invalid TOML, or on an output that does not read
/// back as `doc` with every other key unchanged.
pub fn edit_text(text: &str, doc: &SettingsDoc) -> Result<String, Vec<String>> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| vec![format!("config.toml is not valid TOML: {e}")])?;
    let scan = Scan::new(text);
    scan.check(&table)?;
    let current = crate::parse(text).0.orchestrator;
    let out = scan.edit(&table, doc, &current);
    super::save::read_back(&table, &out, doc)?;
    Ok(out)
}

fn scalars() -> impl Iterator<Item = (&'static str, &'static str)> {
    SCALARS.into_iter().chain(BUDGET_L)
}

fn is(path: &[String], dotted: &str) -> bool {
    path.iter().map(String::as_str).eq(dotted.split('.'))
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
        (Some("models"), _) => format!("[{MODELS}]"),
        (Some("budget"), Some(r @ ("s" | "m" | "l"))) => format!("orchestrator.budget.{r}"),
        (Some("budget"), _) => "orchestrator.budget.<s|m|l>".to_string(),
        (Some("agent"), _) => "orchestrator.agent".to_string(),
        _ => "orchestrator".to_string(),
    }
}

fn refusal(path: &[String]) -> String {
    UNSUPPORTED_FORM
        .replace("{key}", &path.join("."))
        .replace("{table}", &home(path))
}

struct Section {
    header: Option<usize>,
    path: Vec<String>,
    array: bool,
    /// Indices of the key lines in this section.
    keys: Vec<usize>,
}

struct Scan {
    lines: Vec<(String, &'static str)>,
    kinds: Vec<Kind>,
    sections: Vec<Section>,
    eol: &'static str,
    final_newline: bool,
}

impl Scan {
    fn new(text: &str) -> Scan {
        let mut lines = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            match rest.find('\n') {
                Some(n) => {
                    let line = &rest[..n];
                    match line.strip_suffix('\r') {
                        Some(l) => lines.push((l.to_string(), "\r\n")),
                        None => lines.push((line.to_string(), "\n")),
                    }
                    rest = &rest[n + 1..];
                }
                None => {
                    lines.push((rest.to_string(), ""));
                    rest = "";
                }
            }
        }
        let eol = lines
            .iter()
            .map(|(_, e)| *e)
            .find(|e| !e.is_empty())
            .unwrap_or("\n");
        let final_newline = text.is_empty() || text.ends_with('\n');
        let mut kinds = Vec::with_capacity(lines.len());
        let mut i = 0;
        while i < lines.len() {
            let kind = classify(&lines, i);
            let next = match &kind {
                Kind::KeyVal { last, .. } => *last + 1,
                _ => i + 1,
            };
            kinds.push(kind);
            kinds.extend((i + 1..next).map(|_| Kind::Continuation));
            i = next;
        }
        let mut sections = vec![Section {
            header: None,
            path: Vec::new(),
            array: false,
            keys: Vec::new(),
        }];
        for (i, kind) in kinds.iter().enumerate() {
            match kind {
                Kind::Header { path, array } => sections.push(Section {
                    header: Some(i),
                    path: path.clone(),
                    array: *array,
                    keys: Vec::new(),
                }),
                Kind::KeyVal { .. } => sections.last_mut().unwrap().keys.push(i),
                _ => {}
            }
        }
        Scan {
            lines,
            kinds,
            sections,
            eol,
            final_newline,
        }
    }

    fn key_of(&self, i: usize) -> (&[String], usize, Option<(usize, usize)>) {
        match &self.kinds[i] {
            Kind::KeyVal { key, last, span } => (key, *last, *span),
            _ => unreachable!("only key lines are recorded as keys"),
        }
    }

    /// The supported line of an owned scalar, if the file has one.
    fn scalar_line(&self, table: &str, key: &str) -> Option<usize> {
        let section = self.table_section(table)?;
        section.keys.iter().copied().find(|&i| {
            let (k, last, _) = self.key_of(i);
            k.len() == 1 && k[0] == key && last == i
        })
    }

    fn table_section(&self, table: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.header.is_some() && !s.array && is(&s.path, table))
    }

    /// Decision 30's supported forms, checked on the lines and against the parsed table.
    fn check(&self, table: &toml::Table) -> Result<(), Vec<String>> {
        let mut refused = self.refused_lines();
        if refused.is_empty() {
            // Whatever the lines missed, the parsed table still names.
            for (t, k) in scalars() {
                let path = format!("{t}.{k}");
                if lookup(table, &path).is_some() && self.scalar_line(t, k).is_none() {
                    refused.push(split(&path));
                }
            }
            let has_blocks = self.sections.iter().any(|s| s.array && is(&s.path, MODELS));
            if lookup(table, MODELS).is_some() && !has_blocks {
                refused.push(models());
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

    /// Every owned key or table whose header or key line has an unsupported form. A
    /// refused header's own keys are not reported again.
    fn refused_lines(&self) -> Vec<Vec<String>> {
        let mut refused = Vec::new();
        for s in &self.sections {
            if s.header.is_some() {
                let h = &s.path;
                let bad = if s.array {
                    !is(h, MODELS) && (is_container(h) || is_scalar(h) || h.starts_with(&models()))
                } else {
                    is(h, "orchestrator.budget")
                        || is(h, MODELS)
                        || is_scalar(h)
                        || (h.len() > 2 && h.starts_with(&models()))
                };
                if bad {
                    refused.push(h.clone());
                    continue;
                }
            }
            for &i in &s.keys {
                let (key, last, _) = self.key_of(i);
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
        }
        refused
    }

    fn edit(&self, table: &toml::Table, doc: &SettingsDoc, current: &Orchestrator) -> String {
        let mut e = Edits::default();
        let defaults = wanted(&super::doc_of(&Orchestrator::default()), true);
        let roster_changed = doc.models != current.models;
        let mut want = wanted(doc, false);
        if !roster_changed {
            want.remove("orchestrator.builtin_models");
        }
        let mut missing: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (t, k) in scalars() {
            let path = format!("{t}.{k}");
            let Some(new) = want.get(path.as_str()) else {
                continue;
            };
            match (self.scalar_line(t, k), new) {
                (Some(i), Some(v)) if lookup(table, &path) != Some(v) => {
                    let (_, _, span) = self.key_of(i);
                    let (a, b) = span.expect("a supported line has a single-line value");
                    let line = &self.lines[i].0;
                    e.replace
                        .insert(i, format!("{}{v}{}", &line[..a], &line[b..]));
                }
                (Some(i), None) => {
                    e.delete.insert(i);
                }
                (None, Some(v)) if defaults.get(path.as_str()) != Some(&Some(v.clone())) => {
                    missing.entry(t).or_default().push(format!("{k} = {v}"));
                }
                _ => {}
            }
        }
        let mut appended: Vec<Vec<String>> = Vec::new();
        for t in CONTAINERS {
            let Some(keys) = missing.remove(t) else {
                if t == MODELS && roster_changed {
                    self.place_roster(&mut e, &doc.models, &mut appended);
                }
                continue;
            };
            match self.table_section(t) {
                Some(s) => {
                    let anchor = match s.keys.last() {
                        Some(&i) => self.key_of(i).1,
                        None => s.header.unwrap(),
                    };
                    let indent = indent_of(&self.lines[anchor].0);
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
        self.emit(&e, appended)
    }

    /// Removes every `[[orchestrator.models]]` block (header through its last key line,
    /// preflight F15) and writes `models` where the first one was, or at the end.
    fn place_roster(&self, e: &mut Edits, models: &[ModelEntry], appended: &mut Vec<Vec<String>>) {
        let mut blocks: Vec<String> = Vec::new();
        for (n, m) in models.iter().enumerate() {
            if n > 0 {
                blocks.push(String::new());
            }
            blocks.extend(model_block(m));
        }
        let mut first = None;
        for s in self
            .sections
            .iter()
            .filter(|s| s.array && is(&s.path, MODELS))
        {
            let header = s.header.unwrap();
            let end = s.keys.last().map_or(header, |&i| self.key_of(i).1);
            for i in header..=end {
                e.delete.insert(i);
            }
            first.get_or_insert(header);
        }
        match first {
            Some(i) => e.before.entry(i).or_default().extend(blocks),
            None => appended.push(blocks),
        }
    }

    fn emit(&self, e: &Edits, appended: Vec<Vec<String>>) -> String {
        // (content, its own line ending; `None` for a new line)
        let mut out: Vec<(String, Option<&str>)> = Vec::new();
        for (i, (line, eol)) in self.lines.iter().enumerate() {
            if let Some(new) = e.before.get(&i) {
                out.extend(new.iter().map(|l| (l.clone(), None)));
            }
            if !e.delete.contains(&i) {
                let line = e.replace.get(&i).unwrap_or(line);
                out.push((line.clone(), Some(*eol)));
            }
            if let Some(new) = e.after.get(&i) {
                out.extend(new.iter().map(|l| (l.clone(), None)));
            }
        }
        for block in appended {
            if out.last().is_some_and(|(l, _)| !l.trim().is_empty()) {
                out.push((String::new(), None));
            }
            out.extend(block.into_iter().map(|l| (l, None)));
        }
        let mut text = String::new();
        let n = out.len();
        for (k, (line, eol)) in out.into_iter().enumerate() {
            text.push_str(&line);
            let own = eol.filter(|e| !e.is_empty()).unwrap_or(self.eol);
            if k + 1 < n || self.final_newline {
                text.push_str(own);
            }
        }
        text
    }
}

#[derive(Default)]
struct Edits {
    replace: BTreeMap<usize, String>,
    delete: BTreeSet<usize>,
    before: BTreeMap<usize, Vec<String>>,
    after: BTreeMap<usize, Vec<String>>,
}

/// Each owned scalar's wanted value; `None` means "absent" (a runtime left to the default).
fn wanted(doc: &SettingsDoc, builtin: bool) -> BTreeMap<&'static str, Option<toml::Value>> {
    use toml::Value::{Boolean, Integer, String as Str};
    let l = &doc.limits;
    let int = |n: u64| Some(Integer(n as i64));
    BTreeMap::from([
        ("orchestrator.builtin_models", Some(Boolean(builtin))),
        ("orchestrator.max_writers", int(l.max_writers.into())),
        ("orchestrator.max_readers", int(l.max_readers.into())),
        ("orchestrator.max_bounces", int(l.max_bounces.into())),
        ("orchestrator.stall_after_secs", int(l.stall_after_secs)),
        (
            "orchestrator.agent.runtime",
            doc.orchestrator.runtime.map(|r| Str(r.to_string())),
        ),
        (
            "orchestrator.agent.model",
            Some(Str(doc.orchestrator.model.clone())),
        ),
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

fn model_block(m: &ModelEntry) -> Vec<String> {
    let s = |v: &str| toml::Value::String(v.to_string());
    let mut lines = vec![
        format!("[[{MODELS}]]"),
        format!("runtime = {}", s(&m.runtime.to_string())),
        format!("model = {}", s(&m.model)),
        format!("strength = {}", s(strength_name(m.strength))),
    ];
    if !m.note.is_empty() {
        lines.push(format!("note = {}", s(&m.note)));
    }
    lines
}

fn models() -> Vec<String> {
    split(MODELS)
}

fn split(dotted: &str) -> Vec<String> {
    dotted.split('.').map(str::to_string).collect()
}
