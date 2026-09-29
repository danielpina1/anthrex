//! Decision 40: test-weakening signals (TT §3.8) from the claim's
//! `git diff -U0 --no-renames --no-color` and the changed `.rs` files that hold
//! `#[cfg(test)]` at the head. Pure.

use super::{SIGNALS_MAX, Signal};
use crate::run::globs::OwnsMatcher;

/// The assertion markers by file extension; the last entry is every other file's.
pub const ASSERTION_MARKERS: &[(&[&str], &[&str])] = &[
    (&["rs"], &["assert"]),
    (&["py"], &["assert", "pytest.raises"]),
    (
        &["js", "jsx", "ts", "tsx"],
        &["expect(", "assert", "should"],
    ),
    (&["go"], &["t.Error", "t.Fatal", "assert.", "require."]),
    (&[], &["assert", "expect(", "should"]),
];

fn assertion_markers(path: &str) -> &'static [&'static str] {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    ASSERTION_MARKERS
        .iter()
        .find(|(exts, _)| exts.contains(&ext))
        .map(|(_, markers)| *markers)
        .unwrap_or(ASSERTION_MARKERS[ASSERTION_MARKERS.len() - 1].1)
}

/// At most [`SIGNALS_MAX`] signals, deleted test files first.
pub fn signals(
    diff: &str,
    cfg_test_files: &[String],
    test_paths: &[String],
    skip_markers: &[String],
) -> Vec<Signal> {
    signals_and_rest(diff, cfg_test_files, test_paths, skip_markers).0
}

/// [`signals`], and how many more there were (the prompt's `… and <n> more`).
pub fn signals_and_rest(
    diff: &str,
    cfg_test_files: &[String],
    test_paths: &[String],
    skip_markers: &[String],
) -> (Vec<Signal>, usize) {
    let tests = (!test_paths.is_empty())
        .then(|| OwnsMatcher::new(test_paths).ok())
        .flatten();
    let is_test = |path: &str| {
        tests.as_ref().is_some_and(|m| m.matches(path)) || cfg_test_files.iter().any(|f| f == path)
    };
    let mut deleted = Vec::new();
    let mut other = Vec::new();
    for file in parse(diff) {
        if file.deleted {
            if tests.as_ref().is_some_and(|m| m.matches(&file.old)) {
                deleted.push(Signal::DeletedTestFile { path: file.old });
            }
            continue;
        }
        let path = file.new.clone();
        for (line, text) in &file.added {
            if let Some(marker) = skip_markers.iter().find(|m| text.contains(m.as_str())) {
                other.push(Signal::SkipMarker {
                    path: path.clone(),
                    line: *line,
                    marker: marker.clone(),
                });
            }
        }
        if is_test(&path) {
            let markers = assertion_markers(&path);
            let asserts = |text: &String| markers.iter().any(|m| text.contains(m));
            let removed: Vec<u32> = file
                .removed
                .iter()
                .filter(|(_, text)| asserts(text))
                .map(|(line, _)| *line)
                .collect();
            let added = file.added.iter().filter(|(_, text)| asserts(text)).count() as u32;
            if removed.len() as u32 > added {
                other.push(Signal::AssertionLoss {
                    path,
                    line: removed[0],
                    removed: removed.len() as u32,
                    added,
                });
            }
        }
    }
    deleted.extend(other);
    let rest = deleted.len().saturating_sub(SIGNALS_MAX);
    deleted.truncate(SIGNALS_MAX);
    (deleted, rest)
}

/// One file of a `-U0` diff: its paths, whether it was deleted, and its removed
/// (old-side line) and added (new-side line) lines.
#[derive(Default)]
struct FileDiff {
    old: String,
    new: String,
    deleted: bool,
    removed: Vec<(u32, String)>,
    added: Vec<(u32, String)>,
}

fn parse(diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let (mut old_line, mut new_line) = (0u32, 0u32);
    let mut in_hunk = false;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = git_line_path(rest).unwrap_or_default();
            files.push(FileDiff {
                old: path.clone(),
                new: path,
                ..FileDiff::default()
            });
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if !in_hunk {
            if line.starts_with("deleted file mode") {
                file.deleted = true;
            } else if let Some(path) = line.strip_prefix("--- ") {
                if let Some(path) = side_path(path, "a/") {
                    file.old = path;
                }
            } else if let Some(path) = line.strip_prefix("+++ ") {
                match side_path(path, "b/") {
                    Some(path) => file.new = path,
                    None => file.deleted = true,
                }
            }
        }
        if let Some(header) = line.strip_prefix("@@ ") {
            if let Some((old, new)) = hunk_starts(header) {
                (old_line, new_line) = (old, new);
                in_hunk = true;
            }
            continue;
        }
        if !in_hunk {
            continue;
        }
        if let Some(text) = line.strip_prefix('-') {
            file.removed.push((old_line, text.to_string()));
            old_line += 1;
        } else if let Some(text) = line.strip_prefix('+') {
            file.added.push((new_line, text.to_string()));
            new_line += 1;
        }
    }
    files
}

/// The path of `a/<p> b/<p>` (with `--no-renames` both sides are one path), for a file
/// with no `---`/`+++` lines, such as a binary one.
fn git_line_path(rest: &str) -> Option<String> {
    let rest = rest.strip_prefix("a/")?;
    let len = rest.len().checked_sub(3)?;
    if len % 2 != 0 {
        return None;
    }
    let (path, other) = rest.split_at(len / 2);
    (other.strip_prefix(" b/")? == path).then(|| path.to_string())
}

/// A `---`/`+++` line's path without its side prefix; `None` for `/dev/null`. A quoted
/// path (git quotes unusual characters, `core.quotePath`) is unquoted.
fn side_path(path: &str, prefix: &str) -> Option<String> {
    let path = path.trim_end_matches('\t');
    if path == "/dev/null" {
        return None;
    }
    let path = match path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) {
        Some(quoted) => unquote(quoted),
        None => path.to_string(),
    };
    Some(path.strip_prefix(prefix).unwrap_or(&path).to_string())
}

/// C-style unquoting: `\\`, `\"`, `\t`, `\n` and three-digit octal bytes.
fn unquote(quoted: &str) -> String {
    let bytes = quoted.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 == bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let octal = bytes
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)));
        match (octal, bytes[i + 1]) {
            (Some(d), _) => {
                out.push(d.iter().fold(0u8, |n, b| n.wrapping_mul(8) + (b - b'0')));
                i += 4;
            }
            (None, b't') => {
                out.push(b'\t');
                i += 2;
            }
            (None, b'n') => {
                out.push(b'\n');
                i += 2;
            }
            (None, other) => {
                out.push(other);
                i += 2;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `-<a>[,<b>] +<c>[,<d>] @@…` → `(a, c)`.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split(' ');
    let start = |part: Option<&str>, sign: char| -> Option<u32> {
        part?.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    let old = start(parts.next(), '-')?;
    let new = start(parts.next(), '+')?;
    Some((old, new))
}
