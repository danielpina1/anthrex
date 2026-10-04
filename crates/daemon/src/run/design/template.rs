//! The design documents' templates, checked mechanically (brief decision 14 and
//! "Messages (exact)"; DF §3.3, §3.4 and §4.1): the sections each document must have,
//! its size cap, the spec's placeholders and open questions, and the merged brainstorm
//! report's tags, recommendation and single-brainstorm line.
//!
//! Review focus 4: a submitted text is capped on its raw bytes first, so a 10 MB text is
//! refused before anything reads it, then cleaned by `safe_text::multi_line`; every check
//! runs on the cleaned text, which is what is stored. Requirement numbering is
//! `requirements::parse`'s, called by the submit beside this check.
//!
//! Also the markdown reading the other pure modules share: lines that know whether they
//! sit in a fenced code block, headings, and a section's body. Pure.

use proto::{DocKind, safe_text};

/// What a check needs beyond the text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateCtx {
    /// The brainstormers' labels: `claude` and `codex`, or the lenses `A` and `B`. A
    /// merged report's approach is tagged with one of them or `[both]`.
    pub labels: Vec<String>,
    /// A brainstormer that failed, `(label, reason)`: the merged report must begin
    /// `single brainstorm: <label> failed: <reason>` (DF §3.5).
    pub failed: Option<(String, String)>,
    /// The spec's `ready`: its Open questions must then be empty.
    pub ready: bool,
}

const DRAFT_SECTIONS: &[&str] = &[
    "## Understanding",
    "## Assumptions",
    "## Constraints found",
    "## Approaches",
    "## Recommendation",
    "## Questions for you",
];

pub(crate) const REPORT_SECTIONS: &[&str] = &[
    "## Where they agree",
    "## Where they disagree",
    "## Approaches",
    "## Recommendation",
    "## Questions for you",
];

const SPEC_SECTIONS: &[&str] = &[
    "## Goal and success criteria",
    "## Non-goals",
    "## Approach",
    "## Design",
    "## Requirements",
    "## Interfaces",
    "## Errors and edge cases",
    "## Testing",
    "## Risks",
    "## Open questions",
];

/// The spec's title, a level-1 heading, named as the template writes it.
const TITLE: &str = "# <title>";

/// How a refusal names a document.
pub fn kind_name(kind: DocKind) -> &'static str {
    match kind {
        DocKind::BrainstormDraft => "brainstorm draft",
        DocKind::Brainstorm => "brainstorm",
        DocKind::Spec => "spec",
        DocKind::Plan => "plan",
    }
}

/// A document's cap in bytes: 12 KiB for a draft, 32 for the merged report, 64 for the
/// spec (DF §3.3, §3.4, §4.1). The plan is rendered by the engine, never submitted; it
/// takes `get_doc`'s 64 KiB.
pub fn cap_bytes(kind: DocKind) -> usize {
    1024 * match kind {
        DocKind::BrainstormDraft => 12,
        DocKind::Brainstorm => 32,
        DocKind::Spec | DocKind::Plan => 64,
    }
}

fn within_cap(kind: DocKind, bytes: usize) -> Result<(), String> {
    let cap = cap_bytes(kind);
    if bytes > cap {
        return Err(format!(
            "the {} is over its {} KiB cap",
            kind_name(kind),
            cap / 1024
        ));
    }
    Ok(())
}

/// Review focus 4: the raw text's size is checked first, then it is cleaned. Cleaning
/// never makes a text longer, so the cleaned text is within the cap too.
pub fn clean(kind: DocKind, raw: &str) -> Result<String, String> {
    within_cap(kind, raw.len())?;
    Ok(safe_text::multi_line(raw))
}

/// [`clean`], then [`check`]: the text to store, or the refusal.
pub fn admit(kind: DocKind, raw: &str, ctx: &TemplateCtx) -> Result<String, String> {
    let text = clean(kind, raw)?;
    check(kind, &text, ctx)?;
    Ok(text)
}

/// Checks a cleaned text against its kind's template. The first failure is the refusal.
pub fn check(kind: DocKind, text: &str, ctx: &TemplateCtx) -> Result<(), String> {
    within_cap(kind, text.len())?;
    let lines = lines(text);
    match kind {
        DocKind::BrainstormDraft => sections(kind, &lines, DRAFT_SECTIONS),
        DocKind::Brainstorm => {
            single_line(&lines, ctx)?;
            sections(kind, &lines, REPORT_SECTIONS)?;
            approaches(&lines, ctx)
        }
        DocKind::Spec => {
            if !lines
                .iter()
                .any(|l| heading(l).is_some_and(|(n, t)| n == 1 && !t.is_empty()))
            {
                return Err(missing(kind, TITLE));
            }
            sections(kind, &lines, SPEC_SECTIONS)?;
            placeholders(&lines)?;
            open_questions(&lines, ctx)
        }
        DocKind::Plan => Ok(()),
    }
}

fn missing(kind: DocKind, heading: &str) -> String {
    format!(
        "the {} is missing the section \"{heading}\"",
        kind_name(kind)
    )
}

fn sections(kind: DocKind, lines: &[DocLine<'_>], wanted: &[&str]) -> Result<(), String> {
    match wanted.iter().find(|h| section(lines, h).is_none()) {
        Some(h) => Err(missing(kind, h)),
        None => Ok(()),
    }
}

/// `TBD` or `TODO` as a word, outside fenced blocks and inline code.
fn placeholders(lines: &[DocLine<'_>]) -> Result<(), String> {
    for line in lines.iter().filter(|l| !l.code) {
        let prose = without_inline_code(line.text);
        if has_word(&prose, "TBD") || has_word(&prose, "TODO") {
            return Err(format!(
                "the spec still says TBD or TODO at line {}",
                line.n
            ));
        }
    }
    Ok(())
}

fn open_questions(lines: &[DocLine<'_>], ctx: &TemplateCtx) -> Result<(), String> {
    let body = section(lines, "## Open questions").unwrap_or_default();
    if ctx.ready && body.iter().any(|l| !l.text.trim().is_empty()) {
        return Err("Open questions must be empty when ready is true".to_string());
    }
    Ok(())
}

/// DF §3.5: with one brainstormer failed, the report's first non-blank line names it.
fn single_line(lines: &[DocLine<'_>], ctx: &TemplateCtx) -> Result<(), String> {
    let Some((label, reason)) = &ctx.failed else {
        return Ok(());
    };
    let first = lines.iter().map(|l| l.text.trim()).find(|t| !t.is_empty());
    let prefix = format!("single brainstorm: {label} failed:");
    if first.is_some_and(|t| t.starts_with(&prefix)) {
        return Ok(());
    }
    Err(format!(
        "a single brainstorm must begin \"single brainstorm: {label} failed: {}\"",
        safe_text::one_line(reason)
    ))
}

/// DF §3.4: every approach heading (`###` under Approaches) carries `[<label>]` or
/// `[both]`, and the recommendation names a listed approach: it holds the approach's
/// name as a whole word or phrase, ignoring case, or exactly as written for a name
/// under three characters (ruling T4-3).
fn approaches(lines: &[DocLine<'_>], ctx: &TemplateCtx) -> Result<(), String> {
    let tags = tags(&ctx.labels);
    let lower_tags: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
    let body = section(lines, "## Approaches").unwrap_or_default();
    let mut names = Vec::new();
    for line in body {
        let Some((3, title)) = heading(&line) else {
            continue;
        };
        let lower = title.to_lowercase();
        if !lower_tags.iter().any(|t| lower.contains(t.as_str())) {
            let (last, rest) = tags.split_last().expect("[both] is always there");
            let named = match rest {
                [] => last.clone(),
                _ => format!("{} or {last}", rest.join(", ")),
            };
            return Err(format!("approach \"{title}\" has no {named} tag"));
        }
        names.push(approach_name(title, &tags));
    }
    let recommendation: String = section(lines, "## Recommendation")
        .unwrap_or_default()
        .iter()
        .map(|l| l.text.to_string() + "\n")
        .collect();
    let lower = recommendation.to_lowercase();
    let named = |name: &String| match name.chars().count() {
        0 => false,
        // Ruling T4-3: a name under three characters (`A`) only as written.
        1 | 2 => has_word(&recommendation, name),
        _ => has_word(&lower, &name.to_lowercase()),
    };
    if names.iter().any(named) {
        return Ok(());
    }
    Err("the recommendation must name one of the listed approaches".to_string())
}

/// The tags an approach heading may carry: each label's, then `[both]`.
pub(crate) fn tags(labels: &[String]) -> Vec<String> {
    let mut tags: Vec<String> = labels.iter().map(|l| format!("[{l}]")).collect();
    tags.push("[both]".to_string());
    tags
}

/// An approach heading without its tags (matched ignoring ASCII case) and its leading
/// number (`2.` or `2)`), in the heading's own case. Task 4's carry: the title is
/// lower-cased once and the name rebuilt in one pass, so the check stays linear in the
/// capped report however many tags a heading repeats; a tag that removing another
/// would join is text.
pub(crate) fn approach_name(title: &str, tags: &[String]) -> String {
    let tags: Vec<String> = tags.iter().map(|t| t.to_ascii_lowercase()).collect();
    // ASCII lower-casing keeps every byte offset, so a tag found in the lowered copy is
    // the same range of `title`; a tag is ASCII, so its end is a character boundary.
    let lower = title.to_ascii_lowercase();
    let mut name = String::with_capacity(title.len());
    let mut at = 0;
    while let Some(c) = title[at..].chars().next() {
        match tags.iter().find(|t| lower[at..].starts_with(t.as_str())) {
            Some(tag) => at += tag.len(),
            None => {
                name.push(c);
                at += c.len_utf8();
            }
        }
    }
    let name = name.trim();
    let digits = name.len() - name.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let name = match name[digits..].strip_prefix(['.', ')']) {
        Some(rest) if digits > 0 => rest,
        _ => name,
    };
    name.trim().to_string()
}

/// One line of a document, numbered from 1, and whether it is in a fenced code block
/// (a fence line itself counts as code).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DocLine<'a> {
    pub n: usize,
    pub text: &'a str,
    pub code: bool,
}

/// The lines of `text`, each knowing whether it sits in a ```` ``` ```` or `~~~` fence
/// (review m1, after CommonMark): a fence opens on a line starting, after at most three
/// spaces, with three or more of one marker character whose info string, for
/// backticks, holds no backtick (so ```` ```cargo test``` ```` is inline code, not a
/// fence); it closes only on a bare run of the same character at least as long as the
/// opener's, also after at most three spaces. A fence that never closes runs to the end
/// of the document (task 4's carry).
pub(crate) fn lines(text: &str) -> Vec<DocLine<'_>> {
    scan(text).0
}

/// The closing line a document whose last fence never closes needs (its marker
/// repeated to the opener's length), so that text appended after it is not code.
pub(crate) fn open_fence(text: &str) -> Option<String> {
    let (c, len) = scan(text).1?;
    Some(c.to_string().repeat(len))
}

/// [`lines`], and the fence still open at the end: its character and its run's length.
fn scan(text: &str) -> (Vec<DocLine<'_>>, Option<(char, usize)>) {
    let mut fence: Option<(char, usize)> = None;
    let mut out = Vec::new();
    for (i, text) in text.lines().enumerate() {
        let run = fence_run(text);
        let code = match (fence, run) {
            (None, Some((c, len, info))) if c == '~' || !info.contains('`') => {
                fence = Some((c, len));
                true
            }
            (Some((open, at_least)), Some((c, len, info)))
                if c == open && len >= at_least && info.trim().is_empty() =>
            {
                fence = None;
                true
            }
            (open, _) => open.is_some(),
        };
        out.push(DocLine {
            n: i + 1,
            text,
            code,
        });
    }
    (out, fence)
}

/// CommonMark: a fence or a heading sits after at most three spaces; four or more make
/// an indented code line, which is neither (task 4's carry). The line after them.
fn unindented(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ');
    (line.len() - rest.len() <= 3).then_some(rest)
}

/// A line that starts (after at most three spaces) with three or more backticks or
/// tildes: the character, the run's length and the rest of the line.
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = unindented(line)?;
    let c = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = trimmed.len() - trimmed.trim_start_matches(c).len();
    (len >= 3).then(|| (c, len, &trimmed[len..]))
}

/// A markdown heading outside code: its level and its trimmed text.
pub(crate) fn heading<'a>(line: &DocLine<'a>) -> Option<(usize, &'a str)> {
    if line.code {
        return None;
    }
    let text = unindented(line.text)?;
    let level = text.len() - text.trim_start_matches('#').len();
    let rest = &text[level..];
    let spaced = rest.is_empty() || rest.starts_with([' ', '\t']);
    (level > 0 && spaced).then(|| (level, rest.trim()))
}

/// The body of the first section whose heading line is `wanted` (`## Requirements`),
/// up to three leading and any trailing spaces aside: its lines up to the next heading of the same or a higher
/// level. `None` when there is no such heading.
pub(crate) fn section<'a>(lines: &[DocLine<'a>], wanted: &str) -> Option<Vec<DocLine<'a>>> {
    let level = wanted.len() - wanted.trim_start_matches('#').len();
    let start = lines
        .iter()
        .position(|l| !l.code && unindented(l.text).is_some_and(|t| t.trim_end() == wanted))?;
    let body = lines[start + 1..]
        .iter()
        .take_while(|l| heading(l).is_none_or(|(n, _)| n > level))
        .copied()
        .collect();
    Some(body)
}

/// `line` with each inline code span blanked out (review m7): a run of backticks opens
/// a span that the next run of exactly the same length closes. A run with no closer is
/// plain text, so a lone backtick hides nothing. CommonMark (task 4's carry): outside a
/// span, a backslash makes the backtick after it text, and the rest of its run is still
/// a run; inside a span a backslash is text, so a closer is never escaped. Each run's
/// next run of a length is looked up, not searched for, so the masking is linear.
fn without_inline_code(line: &str) -> String {
    let runs = backtick_runs(line);
    // For each run: the nearest later run as long as it, and as long as it less one
    // (the closer of an escaped opener).
    let mut next: Vec<(Option<usize>, Option<usize>)> = vec![(None, None); runs.len()];
    let mut nearest: std::collections::HashMap<usize, usize> = Default::default();
    for (k, &(_, len, _)) in runs.iter().enumerate().rev() {
        next[k] = (nearest.get(&len).copied(), nearest.get(&(len - 1)).copied());
        nearest.insert(len, k);
    }
    let mut out = String::with_capacity(line.len());
    let mut copied = 0;
    let mut k = 0;
    while k < runs.len() {
        let (start, len, escaped) = runs[k];
        let (open, close) = match escaped {
            false => (start, next[k].0),
            true if len > 1 => (start + 1, next[k].1),
            true => (start, None),
        };
        match close {
            Some(j) => {
                let (close, close_len, _) = runs[j];
                out.push_str(&line[copied..open]);
                out.push(' ');
                copied = close + close_len;
                k = j + 1;
            }
            None => k += 1,
        }
    }
    out.push_str(&line[copied..]);
    out
}

/// Each run of backticks in `line`: its byte offset, its length, and whether an odd
/// number of backslashes stands right before it (its first backtick is escaped).
fn backtick_runs(line: &str) -> Vec<(usize, usize, bool)> {
    let mut runs: Vec<(usize, usize, bool)> = Vec::new();
    let mut slashes = 0;
    for (at, c) in line.char_indices() {
        if c != '`' {
            slashes = if c == '\\' { slashes + 1 } else { 0 };
            continue;
        }
        match runs.last_mut() {
            Some((start, len, _)) if *start + *len == at => *len += 1,
            _ => runs.push((at, 1, slashes % 2 == 1)),
        }
        slashes = 0;
    }
    runs
}

/// `word` in `text` with no letter, digit or `_` on either side.
fn has_word(text: &str, word: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

#[cfg(test)]
#[path = "template_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "template_tests_text.rs"]
mod tests_text;

#[cfg(test)]
#[path = "template_tests_linear.rs"]
mod tests_linear;
