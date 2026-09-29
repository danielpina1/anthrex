//! Decision 7's placeholders: parsing a tier command into text and placeholders, and
//! substituting them. Pure.
//!
//! Only these are placeholders: `{module}`, `{modules}`, `{modules:<t>}`,
//! `{filter:<t>}`, `{shard}`, `{shards}` and `{test}`. Any other text, `${VAR}` and
//! shell brace expansion included, is left alone. Inside a template `%` is the value
//! and `%%` a literal `%`; the first `}` ends the template.

use super::{Scope, TierProfile};
use crate::launch::shell_quote;

/// One part of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Module,
    Modules,
    /// `{modules:<t>}`, holding `<t>`.
    ModulesEach(&'a str),
    /// `{filter:<t>}`, holding `<t>`.
    Filter(&'a str),
    Shard,
    Shards,
    Test,
}

impl Piece<'_> {
    /// How validation names it (decision 8's `{<name>} is not allowed here`).
    pub fn name(&self) -> &'static str {
        match self {
            Piece::Text(_) => "",
            Piece::Module => "{module}",
            Piece::Modules => "{modules}",
            Piece::ModulesEach(_) => "{modules:<template>}",
            Piece::Filter(_) => "{filter:<template>}",
            Piece::Shard => "{shard}",
            Piece::Shards => "{shards}",
            Piece::Test => "{test}",
        }
    }
}

const SIMPLE: [(&str, Piece<'static>); 5] = [
    ("{module}", Piece::Module),
    ("{modules}", Piece::Modules),
    ("{shard}", Piece::Shard),
    ("{shards}", Piece::Shards),
    ("{test}", Piece::Test),
];

/// `command` split into text and placeholders, in order.
pub fn pieces(command: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    while let Some(offset) = command[i..].find('{') {
        let at = i + offset;
        let rest = &command[at..];
        let simple = SIMPLE
            .iter()
            .find(|(spelling, _)| rest.starts_with(spelling));
        let templated = [("{modules:", true), ("{filter:", false)]
            .into_iter()
            .find_map(|(open, each)| {
                let body = rest.strip_prefix(open)?;
                let end = body.find('}')?;
                let template = &body[..end];
                let piece = if each {
                    Piece::ModulesEach(template)
                } else {
                    Piece::Filter(template)
                };
                Some((piece, open.len() + end + 1))
            });
        let found = match (simple, templated) {
            (Some((spelling, piece)), _) => Some((piece.clone(), spelling.len())),
            (None, Some(found)) => Some(found),
            (None, None) => None,
        };
        match found {
            Some((piece, len)) => {
                if text_start < at {
                    out.push(Piece::Text(&command[text_start..at]));
                }
                out.push(piece);
                i = at + len;
                text_start = i;
            }
            None => i = at + 1,
        }
    }
    if text_start < command.len() {
        out.push(Piece::Text(&command[text_start..]));
    }
    out
}

/// The values a command's placeholders take. A placeholder whose value is `None` is
/// left as written (tier 0's prompt keeps `{module}`), except `{filter:…}`, which is
/// empty when no filter applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placeholders {
    pub module: Option<String>,
    /// Sorted by [`substitute`].
    pub modules: Option<Vec<String>>,
    pub filter: Option<String>,
    /// `(shard, shards)`, the shard 1-based.
    pub shard: Option<(u8, u8)>,
    pub test: Option<String>,
}

/// `template` with `%` replaced by the shell-quoted `value` and `%%` by `%`.
fn expand(template: &str, value: &str) -> String {
    let quoted = shell_quote(value);
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' if chars.peek() == Some(&'%') => {
                chars.next();
                out.push('%');
            }
            '%' => out.push_str(&quoted),
            c => out.push(c),
        }
    }
    out
}

/// Decision 7: `template` with its placeholders substituted, every value shell-quoted
/// with M8a's quoting.
pub fn substitute(template: &str, values: &Placeholders) -> String {
    let mut modules = values.modules.clone();
    if let Some(list) = &mut modules {
        list.sort();
    }
    let mut out = String::new();
    for piece in pieces(template) {
        let text = match (&piece, values) {
            (Piece::Text(text), _) => text.to_string(),
            (
                Piece::Module,
                Placeholders {
                    module: Some(m), ..
                },
            ) => shell_quote(m),
            (Piece::Modules, _) if modules.is_some() => modules
                .iter()
                .flatten()
                .map(|m| shell_quote(m))
                .collect::<Vec<_>>()
                .join(" "),
            (Piece::ModulesEach(t), _) if modules.is_some() => modules
                .iter()
                .flatten()
                .map(|m| expand(t, m))
                .collect::<Vec<_>>()
                .join(" "),
            (Piece::Filter(t), _) => values
                .filter
                .as_deref()
                .map(|f| expand(t, f))
                .unwrap_or_default(),
            (
                Piece::Shard,
                Placeholders {
                    shard: Some((k, _)),
                    ..
                },
            ) => k.to_string(),
            (
                Piece::Shards,
                Placeholders {
                    shard: Some((_, n)),
                    ..
                },
            ) => n.to_string(),
            (Piece::Test, Placeholders { test: Some(t), .. }) => shell_quote(t),
            (Piece::ModulesEach(t), _) => format!("{{modules:{t}}}"),
            (piece, _) => piece.name().to_string(),
        };
        out.push_str(&text);
    }
    out
}

/// Decision 25's filter expression for a step of `scope` (`timing`: the timing step),
/// or `None` when no clause applies.
pub fn filter_expr(tiers: &TierProfile, scope: Scope, timing: bool) -> Option<String> {
    let slow = tiers.slow_tests.as_deref();
    let time = tiers.timing_tests.as_deref();
    let clauses: Vec<String> = match (scope, timing) {
        (Scope::Gate, false) => [slow, time]
            .into_iter()
            .flatten()
            .map(|f| format!("not ({f})"))
            .collect(),
        (Scope::Gate, true) => time
            .map(|t| format!("({t})"))
            .into_iter()
            .chain(slow.map(|s| format!("not ({s})")))
            .collect(),
        (Scope::Full, false) => time.map(|t| format!("not ({t})")).into_iter().collect(),
        (Scope::Full, true) => time.map(|t| format!("({t})")).into_iter().collect(),
    };
    (!clauses.is_empty()).then(|| clauses.join(" and "))
}
