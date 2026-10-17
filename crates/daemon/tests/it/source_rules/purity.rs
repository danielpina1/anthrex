//! The target pool's purity rule (milestone 9.18, decision 15; the 9.14 follow-up "a
//! purity check for the pool's pure files"). `run/target_pool/{book,book_file,clean}.rs`
//! decide from what they are given and do no I/O and read no clock; the pool's I/O lives
//! in its other files. Milestone 9.14's Verification checked this with a grep by hand.
//! Milestone 9.19 (decision 1, ruling R19) applies the same scan to `common`'s number
//! modules, which format what they are given and read no clock, file or environment.

use super::engine::production_lines;
use super::{code_only, is_ident};
use std::collections::BTreeSet;
use std::path::Path;

/// The pool's pure files, relative to `crates/daemon/src/run/target_pool`.
const PURE_FILES: [&str; 3] = ["book.rs", "book_file.rs", "clean.rs"];

/// The `std` modules a pure file may not name: `std::<m>`, or `<m>` in `use std::{…}`.
const STD_MODULES: [&str; 4] = ["fs", "process", "net", "thread"];
/// The names a pure file may not use, each after no identifier byte (the paths may take
/// spaces around `::`). `std::<m>` for each of `STD_MODULES` is checked beside them.
const NAMES: [&str; 4] = ["tokio::", "Command::new", "SystemTime", "Instant::now"];

/// Every forbidden name in `text`'s production code (not its comments, literals or
/// `#[cfg(test)]` items), as (line, name), sorted.
pub(super) fn pure_scan(text: &str) -> Vec<(usize, &'static str)> {
    let production: BTreeSet<usize> = production_lines(text).iter().map(|l| l.0).collect();
    let code = code_only(text);
    let mut found = Vec::new();
    for (k, line) in code.lines().enumerate() {
        if !production.contains(&(k + 1)) {
            continue;
        }
        let flat = squeezed(line);
        for (module, path) in STD_MODULES.iter().zip(STD_PATHS) {
            if word_at(&flat, &format!("std::{module}"), true) {
                found.push((k + 1, path));
            }
        }
        for name in NAMES {
            // `tokio::` is followed by a path; the others are whole words.
            if word_at(&flat, name, name != "tokio::") {
                found.push((k + 1, name));
            }
        }
    }
    found.extend(
        grouped(&code)
            .into_iter()
            .filter(|(line, _)| production.contains(line)),
    );
    found.sort();
    found.dedup();
    found
}

/// `std::<m>` for each of `STD_MODULES`, as the hits name them.
const STD_PATHS: [&str; 4] = ["std::fs", "std::process", "std::net", "std::thread"];

/// `line` with the whitespace around each `::` removed, so `std :: fs` is `std::fs`.
fn squeezed(line: &str) -> String {
    let mut out = line.to_string();
    while out.contains(" ::") || out.contains(":: ") {
        out = out.replace(" ::", "::").replace(":: ", "::");
    }
    out
}

/// Whether `pat` is in `line` with no identifier byte before it (`::` may be), and,
/// when `whole`, none after it.
fn word_at(line: &str, pat: &str, whole: bool) -> bool {
    let b = line.as_bytes();
    line.match_indices(pat).any(|(at, _)| {
        let before = at == 0 || !is_ident(b[at - 1]);
        let after = !whole || b.get(at + pat.len()).is_none_or(|&c| !is_ident(c));
        before && after
    })
}

/// Each `STD_MODULES` name that is a direct child of a `std::{ … }` group in `code`, as
/// (line, `std::<m>`): `use std::{collections::BTreeMap, fs};` names `std::fs`.
fn grouped(code: &str) -> Vec<(usize, &'static str)> {
    let b = code.as_bytes();
    let mut found = Vec::new();
    for (open, _) in code.match_indices('{') {
        let before = squeezed(code[..open].trim_end());
        let Some(prefix) = before.strip_suffix("std::") else {
            continue;
        };
        if prefix
            .bytes()
            .last()
            .is_some_and(|c| is_ident(c) || c == b':')
        {
            continue;
        }
        let (mut depth, mut child_start) = (0usize, false);
        let mut k = open;
        while k < b.len() {
            match b[k] {
                b'{' => {
                    depth += 1;
                    child_start = depth == 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                b',' => child_start = depth == 1,
                c if c.is_ascii_whitespace() => {}
                c if is_ident(c) => {
                    let len = b[k..].iter().take_while(|&&c| is_ident(c)).count();
                    if child_start
                        && let Some(m) = STD_MODULES.iter().position(|m| *m == &code[k..k + len])
                    {
                        found.push((code[..k].matches('\n').count() + 1, STD_PATHS[m]));
                    }
                    child_start = false;
                    k += len;
                    continue;
                }
                _ => child_start = false,
            }
            k += 1;
        }
    }
    found
}

#[test]
fn the_purity_scan_finds_each_forbidden_form() {
    let forbidden = [
        ("let t = std::fs::read(p)?;", "std::fs"),
        ("use std::fs;", "std::fs"),
        ("use std :: process;", "std::process"),
        ("let s = std::net::TcpStream::connect(a);", "std::net"),
        ("std::thread::sleep(d);", "std::thread"),
        ("tokio::spawn(f);", "tokio::"),
        ("use tokio::sync::Mutex;", "tokio::"),
        ("let c = Command::new(\"git\");", "Command::new"),
        (
            "let c = std::process::Command::new(\"git\");",
            "std::process",
        ),
        ("let t = SystemTime::now();", "SystemTime"),
        ("let t: Option<SystemTime> = None;", "SystemTime"),
        ("let t = Instant::now();", "Instant::now"),
        ("let t = std::time::Instant::now();", "Instant::now"),
        ("use std::{collections::BTreeMap, fs};", "std::fs"),
        ("use std::{\n    io,\n    net::TcpStream,\n};", "std::net"),
    ];
    for (text, name) in forbidden {
        let found = pure_scan(text);
        assert!(
            found.iter().any(|(_, n)| *n == name),
            "{name} not found in {text:?}: {found:?}"
        );
    }
    // The line of a grouped import's offending name, not of the `use`.
    assert_eq!(
        pure_scan("use std::{\n    io,\n    thread,\n};"),
        [(3, "std::thread")]
    );
    let allowed = [
        "use std::time::{Duration, Instant};",
        "fn f(now: Instant) -> Instant { now }",
        "use std::path::{Path, PathBuf};",
        "use std::{collections::BTreeMap, io};",
        "// std::fs::read is done by the caller",
        "/// tokio::spawn and Instant::now() are not used here",
        "let s = \"std::process and SystemTime\";",
        "fn my_tokio() {} struct MyCommand; MyCommand::new_one();",
        "#[cfg(test)]\nmod tests {\n    use std::fs;\n}\n",
        "#[cfg(test)]\nfn now() -> std::time::Instant { std::time::Instant::now() }\n",
    ];
    for text in allowed {
        assert_eq!(pure_scan(text), [], "found in: {text:?}");
    }
    for blinder in super::BLINDERS {
        let text = format!("{blinder}let t = std::fs::read(p);\n");
        assert_eq!(pure_scan(&text), [(2, "std::fs")], "after {blinder:?}");
    }
}

#[test]
fn the_target_pool_pure_files_are_pure() {
    let package = testexec::checkout_dir(env!("CARGO_MANIFEST_DIR"));
    let pool = package.join("src/run/target_pool");
    let mut hits = Vec::new();
    for name in PURE_FILES {
        let file = pool.join(name);
        assert!(file.is_file(), "{} is missing", file.display());
        hits.extend(file_hits(&file, name));
    }
    assert!(hits.is_empty(), "{}", hits.join("\n"));

    // The same check on a constructed pure file with a read in it.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("book.rs");
    std::fs::write(
        &file,
        "use std::path::Path;\n\npub fn load(p: &Path) -> Vec<u8> {\n    std::fs::read(p).unwrap()\n}\n",
    )
    .unwrap();
    let hits = file_hits(&file, "book.rs");
    assert_eq!(hits.len(), 1, "{hits:#?}");
    assert!(
        hits[0].starts_with("run/target_pool/book.rs:4: std::fs in a pure pool file"),
        "{}",
        hits[0]
    );
}

/// One Exact-text line per forbidden name in `file`, named as `run/target_pool/<name>`.
fn file_hits(file: &Path, name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(file).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    pure_scan(&text)
        .into_iter()
        .map(|(line, what)| {
            format!(
                "run/target_pool/{name}:{line}: {what} in a pure pool file (decision 15): {}",
                lines[line - 1].trim()
            )
        })
        .collect()
}

/// `common`'s pure number modules, relative to `crates/common/src`. `glossary.rs` joins in
/// milestone 9.19.3.
const NUMBER_FILES: [&str; 2] = ["numbers.rs", "progress.rs"];

#[test]
fn the_common_number_modules_are_pure() {
    let package = testexec::checkout_dir(env!("CARGO_MANIFEST_DIR"));
    let common = package.join("../common/src");
    let mut hits = Vec::new();
    for name in NUMBER_FILES {
        let file = common.join(name);
        assert!(file.is_file(), "{} is missing", file.display());
        hits.extend(number_hits(&file, name));
    }
    assert!(hits.is_empty(), "{}", hits.join("\n"));

    // The same check on a constructed number file with a clock read in it.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("numbers.rs");
    std::fs::write(
        &file,
        "pub fn now_text() -> String {\n    format!(\"{:?}\", std::time::SystemTime::now())\n}\n",
    )
    .unwrap();
    let hits = number_hits(&file, "numbers.rs");
    assert_eq!(hits.len(), 1, "{hits:#?}");
    assert!(
        hits[0].starts_with("common/src/numbers.rs:2: SystemTime in a pure number module"),
        "{}",
        hits[0]
    );
}

/// One line per forbidden name in `file`, named as `common/src/<name>`.
fn number_hits(file: &Path, name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(file).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    pure_scan(&text)
        .into_iter()
        .map(|(line, what)| {
            format!(
                "common/src/{name}:{line}: {what} in a pure number module (milestone 9.19, decision 1): {}",
                lines[line - 1].trim()
            )
        })
        .collect()
}
