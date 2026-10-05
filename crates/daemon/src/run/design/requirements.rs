//! A spec's requirements (brief decision 14, DF §4.1): within its `## Requirements`
//! section, a line matching `^R(\d+)\b` starts a requirement, whose text runs to the
//! next such line or heading, trimmed and capped at 2 KiB. Lines in fenced code blocks
//! neither start nor end one. Pure.

use serde::{Deserialize, Serialize};

use super::template::{DocLine, heading, lines, section};

/// Decision 14: a requirement's text is capped at 2 KiB.
pub const TEXT_CAP: usize = 2 * 1024;

/// Ruling T4-1: a spec whose Requirements section has no R-line.
pub const NO_REQUIREMENTS: &str = "the spec has no requirements; write them as lines R1, R2, …";

/// Decision 12: the approved spec's Goal section is kept to 8 KiB.
pub const GOAL_CAP: usize = 8 * 1024;

/// How many ids a numbering refusal lists before it stops.
const LISTED: usize = 40;

/// One requirement of a spec. `id` is kept exactly as written (`R4`), never
/// normalised (ruling T4-2): `R01` is not `R1`, and fails the numbering check.
/// (Brief "Interfaces" places it in `state.rs`; it is defined here, beside its parser,
/// and the design state holds it.)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub text: String,
}

/// A requirement line as written: its digits, its number (`None` with a leading zero
/// or past `u32`), and its text so far.
struct Found {
    digits: String,
    number: Option<u32>,
    text: String,
}

/// `digits` as a number, only when they are its canonical spelling: no leading zero.
fn number(digits: &str) -> Option<u32> {
    digits
        .parse::<u32>()
        .ok()
        .filter(|n| n.to_string() == digits)
}

impl Found {
    fn shown(&self) -> String {
        format!("R{}", self.digits)
    }

    fn requirement(&self) -> Requirement {
        Requirement {
            id: self.shown(),
            text: capped(self.text.trim()),
        }
    }
}

/// The spec's requirements, numbered `R1..Rn` in order without gaps or repeats, or the
/// exact refusal `requirements must be numbered R1 to R<n> without gaps; found <list>`.
/// A Requirements section without a single R-line is refused with exactly
/// [`NO_REQUIREMENTS`] (ruling T4-1).
pub fn parse(spec: &str) -> Result<Vec<Requirement>, String> {
    let found = find(spec);
    if found.is_empty() {
        return Err(NO_REQUIREMENTS.to_string());
    }
    let in_order = found
        .iter()
        .enumerate()
        .all(|(i, f)| f.number.is_some_and(|n| n as usize == i + 1));
    if in_order {
        return Ok(found.iter().map(Found::requirement).collect());
    }
    let shown: Vec<String> = found.iter().take(LISTED).map(Found::shown).collect();
    let list = if found.len() > LISTED {
        format!("{}, …", shown.join(", "))
    } else {
        shown.join(", ")
    };
    Err(format!(
        "requirements must be numbered R1 to R{} without gaps; found {list}",
        found.len()
    ))
}

/// Decision 14 and DF §8.1: a round's amendment. Its requirements numbered at or below
/// the approved spec's last are changes (each at most once); the rest must continue
/// from the last number, in order, without gaps. Anything else is refused with exactly
/// `the amendment must continue from R<n+1>`.
pub fn parse_amendment(text: &str, approved: &[Requirement]) -> Result<Vec<Requirement>, String> {
    let last = approved
        .iter()
        .filter_map(|r| number(r.id.strip_prefix('R')?))
        .max()
        .unwrap_or(0);
    let refused = || format!("the amendment must continue from R{}", u64::from(last) + 1);
    let found = find(text);
    let mut changed = Vec::new();
    let mut next = u64::from(last) + 1;
    for f in &found {
        match f.number.map(u64::from) {
            Some(n) if n >= 1 && n <= u64::from(last) && !changed.contains(&n) => changed.push(n),
            Some(n) if n == next => next += 1,
            _ => return Err(refused()),
        }
    }
    if found.is_empty() {
        return Err(refused());
    }
    Ok(found.iter().map(Found::requirement).collect())
}

/// Every requirement in the section as written, the first of a repeated id kept, with
/// no numbering check: what a change summary compares.
pub fn scan(spec: &str) -> Vec<Requirement> {
    let mut out: Vec<Requirement> = Vec::new();
    for r in find(spec).iter().map(Found::requirement) {
        if !out.iter().any(|o| o.id == r.id) {
            out.push(r);
        }
    }
    out
}

/// Decision 12: the spec's `## Goal and success criteria` section, trimmed and cut to
/// [`GOAL_CAP`] bytes: what workers and reviewers get beside their requirements.
pub fn goal_section(spec: &str) -> String {
    let lines = lines(spec);
    let body = section(&lines, "## Goal and success criteria").unwrap_or_default();
    let text: Vec<&str> = body.iter().map(|l| l.text).collect();
    cut(text.join("\n").trim(), GOAL_CAP)
}

/// Decision 22 (task M9.6.11): the spec's `## Interfaces` section, trimmed and cut to
/// [`GOAL_CAP`] bytes: what a sub-planner gets beside the Goal and its requirements.
pub fn interfaces_section(spec: &str) -> String {
    let lines = lines(spec);
    let body = section(&lines, "## Interfaces").unwrap_or_default();
    let text: Vec<&str> = body.iter().map(|l| l.text).collect();
    cut(text.join("\n").trim(), GOAL_CAP)
}

fn find(spec: &str) -> Vec<Found> {
    let lines = lines(spec);
    let Some(body) = section(&lines, "## Requirements") else {
        return Vec::new();
    };
    let mut found: Vec<Found> = Vec::new();
    // Whether the text of the last requirement is still running.
    let mut open = false;
    for line in &body {
        if let Some((digits, rest)) = start(line) {
            found.push(Found {
                number: number(digits),
                digits: digits.to_string(),
                text: rest.to_string(),
            });
            open = true;
        } else if heading(line).is_some() {
            open = false;
        } else if open && let Some(last) = found.last_mut() {
            // Past the cap nothing more is kept; the final cut is on a char boundary.
            if last.text.len() <= TEXT_CAP {
                last.text.push('\n');
                last.text.push_str(line.text);
            }
        }
    }
    found
}

/// `^R(\d+)\b` outside code: the digits and the text after them, without the
/// separator a writer puts there (`R1:`, `R1.`, `R1 -`).
fn start<'a>(line: &DocLine<'a>) -> Option<(&'a str, &'a str)> {
    if line.code {
        return None;
    }
    let rest = line.text.strip_prefix('R')?;
    let digits_len = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits_len == 0 {
        return None;
    }
    let (digits, after) = rest.split_at(digits_len);
    if after
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }
    let text = after
        .trim_start()
        .trim_start_matches([':', '.', ')', '-', '—', '–'])
        .trim_start();
    Some((digits, text))
}

/// `text` cut to [`TEXT_CAP`] bytes on a char boundary.
fn capped(text: &str) -> String {
    cut(text, TEXT_CAP)
}

/// `text` cut to `cap` bytes on a char boundary.
fn cut(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[cfg(test)]
#[path = "requirements_tests.rs"]
mod tests;
