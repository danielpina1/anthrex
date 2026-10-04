//! What changed between two versions of a document: the gate's summary (DF §6.1's
//! Review panel) and decision 36's line diff (`ShowDoc`'s `diff` and the TUI's `d`
//! view; carry M-4). No diff crate is in the workspace, so the diff is a longest common
//! subsequence over lines, written here. Pure.

use std::fmt::Write as _;

use super::requirements::scan;
use super::template::{heading, lines};

/// Decision 36 (carry M-4): either input past 64 KiB gets no diff.
pub const DIFF_CAP: usize = 64 * 1024;

/// Lines of context around a change, as `diff -u`.
const CONTEXT: usize = 3;

/// The LCS table's bound, in cells (16 MB of `u32`). Past it, the lines between the
/// common head and tail are diffed as one removal and one addition: still a correct
/// diff, only not a minimal one. Two 64 KiB documents of ordinary lines stay under it.
const MAX_CELLS: usize = 4_000_000;

/// The changes from `old` to `new`, one line each, in this order: added, changed and
/// removed requirements (`+ R4, R5`, `~ R2`, `- R3`); then each section of `new` that
/// is new (`+ Interfaces`) or changed (`~ Testing: 2 lines`, the lines removed plus
/// added); then each section `new` dropped (`- Risks`). A section is a `##` heading and
/// its body; what comes before the first is `Title`.
pub fn summary(old: &str, new: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (old_reqs, new_reqs) = (scan(old), scan(new));
    let text_of = |reqs: &[super::requirements::Requirement], id: &str| {
        reqs.iter().find(|r| r.id == id).map(|r| r.text.clone())
    };
    let added: Vec<&str> = new_reqs
        .iter()
        .filter(|r| text_of(&old_reqs, &r.id).is_none())
        .map(|r| r.id.as_str())
        .collect();
    let changed: Vec<&str> = new_reqs
        .iter()
        .filter(|r| text_of(&old_reqs, &r.id).is_some_and(|t| t != r.text))
        .map(|r| r.id.as_str())
        .collect();
    let removed: Vec<&str> = old_reqs
        .iter()
        .filter(|r| text_of(&new_reqs, &r.id).is_none())
        .map(|r| r.id.as_str())
        .collect();
    for (sign, ids) in [("+", added), ("~", changed), ("-", removed)] {
        if !ids.is_empty() {
            out.push(format!("{sign} {}", ids.join(", ")));
        }
    }

    let (old_secs, new_secs) = (sections(old), sections(new));
    for (key, body) in &new_secs {
        match old_secs.iter().find(|(k, _)| k == key) {
            None => out.push(format!("+ {}", key.0)),
            Some((_, before)) => {
                let n = ops(before, body).iter().filter(|o| **o != Op::Keep).count();
                if n > 0 {
                    let unit = if n == 1 { "line" } else { "lines" };
                    out.push(format!("~ {}: {n} {unit}", key.0));
                }
            }
        }
    }
    for (key, _) in &old_secs {
        if !new_secs.iter().any(|(k, _)| k == key) {
            out.push(format!("- {}", key.0));
        }
    }
    out
}

/// A section's key: its heading, and which occurrence of that heading it is.
type Key = (String, usize);

fn sections(text: &str) -> Vec<(Key, Vec<&str>)> {
    let mut out: Vec<(Key, Vec<&str>)> = vec![(("Title".to_string(), 0), Vec::new())];
    for line in lines(text) {
        match heading(&line) {
            Some((2, title)) => {
                let seen = out.iter().filter(|((k, _), _)| k == title).count();
                out.push(((title.to_string(), seen), Vec::new()));
            }
            _ => out
                .last_mut()
                .expect("starts with the title")
                .1
                .push(line.text),
        }
    }
    out
}

/// The unified-style line diff of `old` to `new`: `@@ -a,b +c,d @@` hunk headers with
/// three lines of context, and ` `, `-` and `+` prefixed lines, each ending in `\n`.
/// Empty when the texts have the same lines. Either input past [`DIFF_CAP`] bytes gives
/// `diff too large (<n> lines)`, `n` both inputs' lines together.
pub fn line_diff(old: &str, new: &str) -> String {
    if old.len() > DIFF_CAP || new.len() > DIFF_CAP {
        let n = old.lines().count() + new.lines().count();
        return format!("diff too large ({n} lines)");
    }
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let ops = ops(&a, &b);
    let mut out = String::new();
    for (start, end) in hunks(&ops) {
        // The old and new line numbers where the hunk starts.
        let before = &ops[..start];
        let old_at = before.iter().filter(|o| **o != Op::Add).count();
        let new_at = before.iter().filter(|o| **o != Op::Del).count();
        let span = &ops[start..end];
        let old_n = span.iter().filter(|o| **o != Op::Add).count();
        let new_n = span.iter().filter(|o| **o != Op::Del).count();
        let from = |at: usize, n: usize| if n == 0 { at } else { at + 1 };
        let _ = writeln!(
            out,
            "@@ -{},{old_n} +{},{new_n} @@",
            from(old_at, old_n),
            from(new_at, new_n)
        );
        let (mut i, mut j) = (old_at, new_at);
        for op in span {
            match op {
                Op::Keep => {
                    let _ = writeln!(out, " {}", a[i]);
                    i += 1;
                    j += 1;
                }
                Op::Del => {
                    let _ = writeln!(out, "-{}", a[i]);
                    i += 1;
                }
                Op::Add => {
                    let _ = writeln!(out, "+{}", b[j]);
                    j += 1;
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Keep,
    Del,
    Add,
}

/// The edit script from `a` to `b`: the common head and tail kept, the middle by LCS
/// (removals before additions where both are equally short), or past [`MAX_CELLS`] as
/// one removal and one addition.
fn ops(a: &[&str], b: &[&str]) -> Vec<Op> {
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut out = vec![Op::Keep; head];
    let (n, m) = (am.len(), bm.len());
    if n.saturating_mul(m) > MAX_CELLS {
        out.extend(std::iter::repeat_n(Op::Del, n));
        out.extend(std::iter::repeat_n(Op::Add, m));
    } else {
        // lcs[i * (m + 1) + j]: the LCS length of am[i..] and bm[j..].
        let width = m + 1;
        let mut lcs = vec![0u32; (n + 1) * width];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * width + j] = if am[i] == bm[j] {
                    lcs[(i + 1) * width + j + 1] + 1
                } else {
                    lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                out.push(Op::Keep);
                i += 1;
                j += 1;
            } else if j == m || (i < n && lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
                out.push(Op::Del);
                i += 1;
            } else {
                out.push(Op::Add);
                j += 1;
            }
        }
    }
    out.extend(std::iter::repeat_n(Op::Keep, tail));
    out
}

/// The `[start, end)` ranges of `ops` each hunk shows: every change with [`CONTEXT`]
/// kept lines around it, and changes closer than twice that in one hunk.
fn hunks(ops: &[Op]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (k, _) in ops.iter().enumerate().filter(|(_, o)| **o != Op::Keep) {
        let start = k.saturating_sub(CONTEXT);
        let end = (k + 1 + CONTEXT).min(ops.len());
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => out.push((start, end)),
        }
    }
    out
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
