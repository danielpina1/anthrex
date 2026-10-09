//! The text the writers edit (decision 30), split out of `write.rs` by M9.8.12 so the
//! limits writer (`write.rs`) and the models writer (`write_models.rs`, `write_old.rs`)
//! share it: the lines with their own endings, each line's kind, the sections they
//! fall in, and the edits that turn them back into text. Pure.

use std::collections::{BTreeMap, BTreeSet};

use super::lines::{Kind, classify};

pub(super) fn is(path: &[String], dotted: &str) -> bool {
    path.iter().map(String::as_str).eq(dotted.split('.'))
}

pub(super) fn split(dotted: &str) -> Vec<String> {
    dotted.split('.').map(str::to_string).collect()
}

pub(super) struct Section {
    pub header: Option<usize>,
    pub path: Vec<String>,
    pub array: bool,
    /// Indices of the key lines in this section.
    pub keys: Vec<usize>,
}

pub(super) struct Scan {
    pub lines: Vec<(String, &'static str)>,
    pub kinds: Vec<Kind>,
    pub sections: Vec<Section>,
    eol: &'static str,
    final_newline: bool,
}

#[derive(Default)]
pub(super) struct Edits {
    pub replace: BTreeMap<usize, String>,
    pub delete: BTreeSet<usize>,
    pub before: BTreeMap<usize, Vec<String>>,
    pub after: BTreeMap<usize, Vec<String>>,
}

impl Scan {
    pub fn new(text: &str) -> Scan {
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

    pub fn key_of(&self, i: usize) -> (&[String], usize, Option<(usize, usize)>) {
        match &self.kinds[i] {
            Kind::KeyVal { key, last, span } => (key, *last, *span),
            _ => unreachable!("only key lines are recorded as keys"),
        }
    }

    /// The supported line of `key` in `section`: a single segment on one line.
    pub fn line_in(&self, section: &Section, key: &str) -> Option<usize> {
        section.keys.iter().copied().find(|&i| {
            let (k, last, _) = self.key_of(i);
            k.len() == 1 && k[0] == key && last == i
        })
    }

    /// The supported line of an owned scalar, if the file has one.
    pub fn scalar_line(&self, table: &str, key: &str) -> Option<usize> {
        self.line_in(self.table_section(table)?, key)
    }

    pub fn table_section(&self, table: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.header.is_some() && !s.array && is(&s.path, table))
    }

    /// The last line of `section`: its last key's last line, or its header.
    pub fn end_of(&self, section: &Section) -> Option<usize> {
        match section.keys.last() {
            Some(&i) => Some(self.key_of(i).1),
            None => section.header,
        }
    }

    /// Marks `section` (header through its last key's last line) deleted.
    pub fn delete_section(&self, e: &mut Edits, section: &Section) {
        if let (Some(header), Some(end)) = (section.header, self.end_of(section)) {
            e.delete.extend(header..=end);
        }
    }

    /// Marks key line `i` (and its value's continuation lines) deleted.
    pub fn delete_key(&self, e: &mut Edits, i: usize) {
        let (_, last, _) = self.key_of(i);
        e.delete.extend(i..=last);
    }

    /// The edited text. A blank line a deletion leaves doubled (or leading, or trailing)
    /// goes too (M9.8.12); `appended` blocks follow the last line, one blank line apart.
    pub fn emit(&self, e: &Edits, appended: Vec<Vec<String>>) -> String {
        // (content, its own line ending; `None` for a new line)
        let mut out: Vec<(String, Option<&str>)> = Vec::new();
        let blank =
            |out: &[(String, Option<&str>)]| out.last().is_some_and(|(l, _)| l.trim().is_empty());
        // A new blank line never follows another, nor opens the file.
        let add = |out: &mut Vec<(String, Option<&str>)>, new: &[String]| {
            for l in new {
                if !(l.trim().is_empty() && (out.is_empty() || blank(out))) {
                    out.push((l.clone(), None));
                }
            }
        };
        let mut gap = false;
        for (i, (line, eol)) in self.lines.iter().enumerate() {
            if let Some(new) = e.before.get(&i) {
                add(&mut out, new);
                gap = false;
            }
            if e.delete.contains(&i) {
                gap = true;
            } else {
                let line = e.replace.get(&i).unwrap_or(line);
                let doubled = gap && line.trim().is_empty() && (out.is_empty() || blank(&out));
                if !doubled {
                    out.push((line.clone(), Some(*eol)));
                }
                gap = false;
            }
            if let Some(new) = e.after.get(&i) {
                add(&mut out, new);
                gap = false;
            }
        }
        if gap && blank(&out) {
            out.pop();
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
