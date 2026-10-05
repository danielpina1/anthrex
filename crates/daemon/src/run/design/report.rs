//! The merged brainstorm report (DF §3.4), beyond its template check: the appendix the
//! engine attaches to its written file (decision 13), and what the brainstorm gate's
//! Review panel reads from it (DF §6.1). Pure.
//!
//! **The appendix.** `## Appendix: the drafts`, then each brainstormer's draft under
//! `### <label>`, its own headings demoted three levels so that, inside the appendix,
//! the labels are the only headings at their level or above, and a draft's sections
//! never pass for the report's. A fence the report or a draft leaves open is closed
//! first, so nothing the engine writes after it is code. The appendix is the last
//! `## Appendix: the drafts` line outside code ([`split`]).

use proto::{ApproachTag, DocKind, ReportSummary, safe_text};

use super::template::{DocLine, approach_name, heading, lines, open_fence, section, tags};

/// The appendix's heading (decision 13).
pub const APPENDIX: &str = "## Appendix: the drafts";
/// How many levels a draft's headings go down, below its `### <label>`.
const DEMOTE: usize = 3;
/// Markdown's deepest heading.
const DEEPEST: usize = 6;

/// `report` with the appendix attached: each `(label, draft)` in order, a draft that
/// is missing named with why (`(no draft: <reason>)`). What the engine writes.
pub fn attach(report: &str, drafts: &[(String, Result<String, String>)]) -> String {
    let mut out = closed(report.trim_end());
    out.push_str("\n\n");
    out.push_str(APPENDIX);
    out.push('\n');
    for (label, draft) in drafts {
        out.push_str(&format!("\n### {label}\n\n"));
        match draft {
            Ok(text) => out.push_str(&closed(demoted(text).trim_end())),
            Err(reason) => out.push_str(&format!("(no draft: {})", safe_text::one_line(reason))),
        }
        out.push('\n');
    }
    out
}

/// `text` split at the engine's appendix: the report before it, and the appendix.
/// `(text, None)` without one.
pub fn split(text: &str) -> (&str, Option<&str>) {
    let doc = lines(text);
    let found = (doc.iter().rev()).find(|l| !l.code && l.text.trim_end() == APPENDIX);
    match found {
        Some(line) => {
            let at = line.text.as_ptr() as usize - text.as_ptr() as usize;
            (&text[..at], Some(&text[at..]))
        }
        None => (text, None),
    }
}

/// What a version of `kind` says without the engine's appendix: a brainstorm report's
/// own text, any other document whole. Change summaries compare this.
pub fn body(kind: DocKind, text: &str) -> &str {
    match kind {
        DocKind::Brainstorm => split(text).0,
        _ => text,
    }
}

/// DF §6.1: the report's agree and disagree counts and each approach's tag, read from
/// its own text (the appendix left out). `labels` are the brainstormers'.
pub fn summary(text: &str, labels: &[String]) -> ReportSummary {
    let doc = lines(split(text).0);
    let count = |wanted: &str| section(&doc, wanted).map_or(0, |b| points(&b));
    let tags = tags(labels);
    let lower: Vec<String> = tags.iter().map(|t| t.to_ascii_lowercase()).collect();
    let body = section(&doc, "## Approaches").unwrap_or_default();
    let approaches = (body.iter())
        .filter_map(|l| heading(l).filter(|(n, _)| *n == 3).map(|(_, t)| t))
        .map(|title| {
            let low = title.to_ascii_lowercase();
            let first = (lower.iter().enumerate())
                .filter_map(|(k, t)| low.find(t.as_str()).map(|at| (at, k)))
                .min();
            let tag = first.map_or_else(String::new, |(_, k)| {
                tags[k].trim_matches(['[', ']']).to_string()
            });
            ApproachTag {
                name: approach_name(title, &tags),
                tag,
            }
        })
        .collect();
    ReportSummary {
        agree: count("## Where they agree"),
        disagree: count("## Where they disagree"),
        approaches,
    }
}

/// A section's points: its sub-headings, else its list items at the line's start, else
/// its paragraphs.
fn points(body: &[DocLine<'_>]) -> u32 {
    let subs = body.iter().filter(|l| heading(l).is_some()).count();
    if subs > 0 {
        return subs as u32;
    }
    let items = (body.iter())
        .filter(|l| !l.code && list_item(l.text))
        .count();
    if items > 0 {
        return items as u32;
    }
    let mut paragraphs = 0;
    let mut blank = true;
    for line in body {
        let empty = line.text.trim().is_empty();
        if blank && !empty {
            paragraphs += 1;
        }
        blank = empty;
    }
    paragraphs
}

/// A list item at the line's start: `- `, `* `, `+ `, `1. ` or `1) `.
fn list_item(line: &str) -> bool {
    if ["- ", "* ", "+ "].iter().any(|m| line.starts_with(m)) {
        return true;
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    digits > 0 && [". ", ") "].iter().any(|m| line[digits..].starts_with(m))
}

/// `text` with a fence it leaves open closed on a line of its own.
fn closed(text: &str) -> String {
    match open_fence(text) {
        Some(fence) => format!("{text}\n{fence}"),
        None => text.to_string(),
    }
}

/// `text`'s headings each [`DEMOTE`] levels down, to [`DEEPEST`] at most; code is left
/// as written.
fn demoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    for line in lines(text) {
        if let Some((level, _)) = heading(&line) {
            let add = (level + DEMOTE).min(DEEPEST).saturating_sub(level);
            let at = line.text.len() - line.text.trim_start_matches(' ').len();
            out.push_str(&line.text[..at]);
            out.push_str(&"#".repeat(add));
            out.push_str(&line.text[at..]);
        } else {
            out.push_str(line.text);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
