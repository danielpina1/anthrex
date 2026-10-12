//! The engine-purity scanner (milestone 9.12, decisions 16 and 17): which names the run
//! engine's production code may not use, and which of its lines are production code.

use super::{code_only, is_ident, rust_files};
use std::path::{Path, PathBuf};

/// The module declarations and test-only line ranges of one file (decision 17).
struct Parsed {
    /// Per line: inside a `#[cfg(test)]` item (attribute lines included).
    skip: Vec<bool>,
    /// Each `#[cfg(test)] mod <name>;`, with its `#[path = "…"]` if it has one.
    mounts: Vec<(String, Option<String>)>,
}

/// The text of the first double-quoted string on `line`.
fn quoted(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let len = line[start..].find('"')?;
    Some(line[start..start + len].to_string())
}

/// The first two words of an item, after its visibility (`pub`, `pub(crate)`, …).
fn item_words(code: &str) -> Vec<&str> {
    let mut words = item_words_all(code);
    words.truncate(2);
    words
}

/// The position just after the attribute whose `#` is at (`line`, `col`): the `]` that
/// closes its `[`, matched across lines. Panics when the file ends first.
fn attr_end(code: &[&str], line: usize, col: usize) -> (usize, usize) {
    let mut depth = 0i32;
    let mut from = col;
    for (l, text) in code.iter().enumerate().skip(line) {
        for (k, c) in text.bytes().enumerate().skip(from) {
            match c {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return (l, k + 1);
                    }
                }
                _ => {}
            }
        }
        from = 0;
    }
    panic!("the attribute on line {} never closes", line + 1)
}

/// The last line of the item that starts at (`from`, `col`) of `code`: a `use`, `const`,
/// `static` or `type` ends at its `;`; any other item at its `;` or at the `}` that closes
/// its first brace (so a `{` inside a signature or a body never ends it early). Panics
/// when a bracket closes one it never opened or the file ends inside the item: the parse
/// has lost its place, and carrying on would leave the rest of the file unscanned.
fn item_end(code: &[&str], from: usize, col: usize) -> usize {
    let words = item_words(&code[from][col..]);
    let semicolon_only = match words.as_slice() {
        ["use" | "static" | "type", ..] => true,
        ["const", next] => !matches!(*next, "fn" | "unsafe" | "async"),
        _ => false,
    };
    let (mut depth, mut braced) = (0i32, false);
    let mut skip = col;
    for (line, text) in code.iter().enumerate().skip(from) {
        for c in text.bytes().skip(skip) {
            match c {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b'{' => {
                    depth += 1;
                    braced = true;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 && braced && !semicolon_only {
                        return line;
                    }
                }
                b';' if depth == 0 => return line,
                _ => {}
            }
            assert!(
                depth >= 0,
                "the item from line {} closes a bracket it never opened on line {}",
                from + 1,
                line + 1
            );
        }
        skip = 0;
    }
    panic!("the item from line {} never ends", from + 1)
}

/// `header` (the code before a `{`) with its leading attributes removed.
fn without_attributes(mut header: &str) -> &str {
    loop {
        header = header.trim_start();
        if !header.starts_with('#') {
            return header;
        }
        let Some(open) = header.find('[') else {
            return header;
        };
        let mut depth = 0;
        let mut close = None;
        for (k, c) in header.bytes().enumerate().skip(open) {
            match c {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(k);
                        break;
                    }
                }
                _ => {}
            }
        }
        match close {
            Some(k) => header = &header[k + 1..],
            None => return header,
        }
    }
}

/// Whether the `{` after `header` opens a body that holds items: a `mod`, an `impl`, a
/// `trait` or an `extern` block. A struct, enum or union body holds fields or variants,
/// and a function or expression body holds statements.
fn opens_items(header: &str) -> bool {
    let mut words = item_words_all(without_attributes(header)).into_iter();
    let mut first = words.next();
    while matches!(first, Some("unsafe" | "default" | "auto")) {
        first = words.next();
    }
    matches!(first, Some("mod" | "impl" | "trait" | "extern"))
}

/// Every word of an item's head, after its visibility.
fn item_words_all(code: &str) -> Vec<&str> {
    let mut rest = code.trim_start();
    if let Some(r) = rest.strip_prefix("pub") {
        rest = r.trim_start();
        if rest.starts_with('(') {
            rest = rest.find(')').map_or("", |i| &rest[i + 1..]);
        }
    }
    rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Per line, whether it starts in a body that holds items (the file, or a `mod`, `impl`,
/// `trait` or `extern` body), where `#[cfg(test)]` marks a whole item. Anywhere else it
/// marks a field, a variant or a statement, which the scan keeps rather than skip and
/// lose its place. Panics on an unbalanced brace.
fn item_bodies(code: &[&str]) -> Vec<bool> {
    let mut stack = vec![true];
    let mut header = String::new();
    let mut at_line = Vec::with_capacity(code.len());
    for (line, text) in code.iter().enumerate() {
        at_line.push(*stack.last().unwrap());
        for c in text.chars() {
            match c {
                '{' => {
                    let items = *stack.last().unwrap() && opens_items(&header);
                    stack.push(items);
                    header.clear();
                }
                '}' => {
                    stack.pop();
                    assert!(
                        !stack.is_empty(),
                        "line {} closes a brace it never opened",
                        line + 1
                    );
                    header.clear();
                }
                ';' => header.clear(),
                c => header.push(c),
            }
        }
        header.push(' ');
    }
    assert!(
        stack.len() == 1,
        "the file ends inside {} brace(s)",
        stack.len() - 1
    );
    at_line
}

fn parse(text: &str) -> Parsed {
    let code_text = code_only(text);
    let code: Vec<&str> = code_text.lines().collect();
    let orig: Vec<&str> = text.lines().collect();
    let bodies = item_bodies(&code);
    let mut skip = vec![false; code.len()];
    let mut mounts = Vec::new();
    let mut i = 0;
    while i < code.len() {
        if code[i].trim() != "#[cfg(test)]" || !bodies[i] {
            i += 1;
            continue;
        }
        skip[i] = true;
        let mut path = None;
        // The item's other attributes, each matched to its closing `]`, so one that spans
        // lines is passed whole; the item may start on the line an attribute ends.
        let (mut j, mut col) = (i + 1, 0);
        loop {
            while j < code.len() && code[j][col..].trim().is_empty() {
                skip[j] = true;
                (j, col) = (j + 1, 0);
            }
            assert!(
                j < code.len(),
                "the #[cfg(test)] on line {} has no item",
                i + 1
            );
            let rest = &code[j][col..];
            if !rest.trim_start().starts_with("#[") {
                break;
            }
            let at = col + rest.len() - rest.trim_start().len();
            if rest.trim_start().starts_with("#[path") {
                path = quoted(&orig[j][at..]);
            }
            let (end, after) = attr_end(&code, j, at);
            end_skip(&mut skip, j, end);
            (j, col) = (end, after);
        }
        let end = item_end(&code, j, col);
        let mut head = code[j][col..].to_string();
        for line in &code[j + 1..=end] {
            head.push(' ');
            head.push_str(line);
        }
        let before_brace = head.split('{').next().unwrap_or("");
        if let ["mod", name] = item_words(&code[j][col..]).as_slice()
            && before_brace.contains(';')
        {
            mounts.push((name.to_string(), path));
            end_skip(&mut skip, j, j);
            i = j + 1;
            continue;
        }
        end_skip(&mut skip, j, end);
        i = end + 1;
    }
    Parsed { skip, mounts }
}

fn end_skip(skip: &mut [bool], from: usize, to: usize) {
    for s in &mut skip[from..=to] {
        *s = true;
    }
}

/// The production lines of one file: not comment lines, not blank, not inside a
/// `#[cfg(test)]` item. Strings and comments are judged on the blanked text, so a
/// `#[cfg(test)]` inside a string starts nothing.
pub(super) fn production_lines(text: &str) -> Vec<(usize, &str)> {
    let parsed = parse(text);
    let code = code_only(text);
    text.lines()
        .zip(code.lines())
        .enumerate()
        .filter(|(i, (_, c))| !parsed.skip[*i] && !c.trim().is_empty())
        .map(|(i, (line, _))| (i + 1, line))
        .collect()
}

/// `std` paths decision 16 forbids.
const PATHS: [&str; 4] = ["std::fs", "std::process", "std::thread", "std::env"];
/// Names decision 16 forbids wherever they appear as a word.
const WORDS: [&str; 5] = ["tokio", "SystemTime", "UNIX_EPOCH", "libc", "portable_pty"];

/// `line` with the whitespace around each `::` removed, so `std :: fs` is `std::fs`.
fn squeezed(line: &str) -> String {
    let mut out = line.to_string();
    while out.contains(" ::") || out.contains(":: ") {
        out = out.replace(" ::", "::").replace(":: ", "::");
    }
    out
}

/// Every offset of `pat` in `line` as a whole word (no identifier byte on either side).
fn word_at(line: &str, pat: &str) -> bool {
    let b = line.as_bytes();
    line.match_indices(pat).any(|(at, _)| {
        (at == 0 || !is_ident(b[at - 1])) && b.get(at + pat.len()).is_none_or(|&c| !is_ident(c))
    })
}

/// The forbidden name on one blanked code line, if any (grouped imports excluded).
fn line_pattern(code: &str) -> Option<&'static str> {
    let flat = squeezed(code);
    for pat in PATHS.into_iter().chain(["Instant::now"]) {
        if word_at(&flat, pat) {
            return Some(pat);
        }
    }
    WORDS.into_iter().find(|w| word_at(code, w))
}

/// Forbidden names inside `use std::{ … }` groups, as (line index, pattern): the first
/// word of each direct child (`fs`, `process::Command`, `thread`, `env::{self}`).
fn group_patterns(code: &[&str]) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    let (mut depth, mut child_start) = (0i32, false);
    for (line, text) in code.iter().enumerate() {
        let mut chars: Vec<char> = text.chars().collect();
        if depth == 0 {
            // Resume at the `{` that follows `std::`.
            let open = text
                .match_indices('{')
                .find(|(i, _)| ends_with_std_path(&text[..*i]))
                .map(|(i, _)| i);
            let Some(open) = open else { continue };
            chars = text[open..].chars().collect();
        }
        let mut k = 0;
        while k < chars.len() {
            let c = chars[k];
            match c {
                '{' => {
                    depth += 1;
                    child_start = depth == 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                ',' => child_start = depth == 1,
                c if c.is_whitespace() => {}
                c if is_ident(c as u8) && c.is_ascii() => {
                    let start = k;
                    while k < chars.len() && chars[k].is_ascii() && is_ident(chars[k] as u8) {
                        k += 1;
                    }
                    if child_start && depth == 1 {
                        let word: String = chars[start..k].iter().collect();
                        if let Some(p) = ["fs", "process", "thread", "env"]
                            .into_iter()
                            .position(|w| w == word)
                        {
                            found.push((line, PATHS[p]));
                        }
                    }
                    child_start = false;
                    continue;
                }
                _ => child_start = false,
            }
            k += 1;
        }
    }
    found
}

/// Whether `prefix` (the text before a `{`) ends in the path `std::`.
fn ends_with_std_path(prefix: &str) -> bool {
    squeezed(prefix.trim_end())
        .strip_suffix("std::")
        .is_some_and(|before| {
            before
                .bytes()
                .last()
                .is_none_or(|c| !is_ident(c) && c != b':')
        })
}

/// The forbidden-name scan of one file's production code, as (line, pattern).
pub(super) fn engine_scan(text: &str) -> Vec<(usize, &'static str)> {
    let parsed = parse(text);
    let code_text = code_only(text);
    let code: Vec<&str> = code_text.lines().collect();
    let mut found: Vec<(usize, &'static str)> = group_patterns(&code)
        .into_iter()
        .filter(|(i, _)| !parsed.skip[*i])
        .map(|(i, p)| (i + 1, p))
        .collect();
    for (i, line) in code.iter().enumerate() {
        if !parsed.skip[i]
            && let Some(p) = line_pattern(line)
        {
            found.push((i + 1, p));
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Where the module `name`, declared in `file`, lives: `name.rs` or `name/mod.rs` in the
/// module directory, or the `#[path]` file beside the declaring file.
fn mount_targets(file: &Path, name: &str, path: Option<&str>) -> Vec<PathBuf> {
    let dir = file.parent().unwrap();
    if let Some(path) = path {
        return vec![dir.join(path)];
    }
    let owns_dir = file.file_name().is_some_and(|f| f == "mod.rs");
    let base = if owns_dir {
        dir.to_path_buf()
    } else {
        dir.join(file.file_stem().unwrap())
    };
    vec![
        base.join(format!("{name}.rs")),
        base.join(name).join("mod.rs"),
    ]
}

/// Whether `file` is test code: under `<root>/tests`, mounted only under `#[cfg(test)]`,
/// or inside the directory of such a module.
fn test_only(root: &Path, file: &Path, mounted: &[PathBuf]) -> bool {
    if file
        .strip_prefix(root)
        .is_ok_and(|r| r.starts_with("tests"))
    {
        return true;
    }
    mounted.iter().any(|m| {
        let dir = if m.file_name().is_some_and(|f| f == "mod.rs") {
            m.parent().unwrap().to_path_buf()
        } else {
            m.with_extension("")
        };
        file == m || file.starts_with(&dir)
    })
}

/// Every forbidden name in the production code under `root` (the engine directory), one
/// Exact-text line each.
pub(super) fn engine_hits(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    rust_files(root, &mut files);
    files.sort();
    let texts: Vec<String> = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect();
    let mut mounted = Vec::new();
    for (file, text) in files.iter().zip(&texts) {
        for (name, path) in parse(text).mounts {
            mounted.extend(mount_targets(file, &name, path.as_deref()));
        }
    }
    let mut hits = Vec::new();
    for (file, text) in files.iter().zip(&texts) {
        if test_only(root, file, &mounted) {
            continue;
        }
        let rel = file.strip_prefix(root).unwrap().display().to_string();
        let lines: Vec<&str> = text.lines().collect();
        for (line, pattern) in engine_scan(text) {
            hits.push(format!(
                "run/engine/{rel}:{line}: {pattern} in engine code (the reducer is pure: decision 16); {}",
                lines[line - 1].trim()
            ));
        }
    }
    hits
}
