//! The test-target rule (milestone 9.12, final review I1). The daemon and the CLI set
//! `autotests = false`, so Cargo builds only their declared `[[test]]` targets: a
//! `tests/<f>.rs` with no `[[test]]` entry, or a `tests/it/<f>.rs` no `mod` line reaches,
//! is never compiled, and nothing says so. This rule names each such file.

use super::{code_only, is_ident, rust_files};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Each out-of-line `mod <name>;` in `text`'s code, with its `#[path = "…"]` if one of
/// the attributes before it (on its own line or the lines just above) has one.
pub(super) fn mod_decls(text: &str) -> Vec<(String, Option<String>)> {
    let code = code_only(text);
    let b = code.as_bytes();
    let mut decls = Vec::new();
    for (at, _) in code.match_indices("mod") {
        if at > 0 && is_ident(b[at - 1]) {
            continue;
        }
        let rest = &code[at + 3..];
        if !rest.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let rest = rest.trim_start();
        let len = rest.bytes().take_while(|&c| is_ident(c)).count();
        if len == 0 || !rest[len..].trim_start().starts_with(';') {
            continue;
        }
        let name = rest[..len].to_string();
        // The attributes: this line up to `mod`, and the attribute lines just above.
        let mut start = code[..at].rfind('\n').map_or(0, |i| i + 1);
        while start > 0 {
            let prev = code[..start - 1].rfind('\n').map_or(0, |i| i + 1);
            let line = code[prev..start - 1].trim();
            if !line.starts_with("#[") {
                break;
            }
            start = prev;
        }
        let path = code[start..at].find("#[path").and_then(|p| {
            let from = start + p;
            let open = text[from..].find('"')? + from + 1;
            let len = text[open..].find('"')?;
            Some(text[open..open + len].to_string())
        });
        decls.push((name, path));
    }
    decls
}

/// The file `mod name;` with `path` mounts from `file`, and whether that file owns its
/// directory (a crate root, a `mod.rs`, or any `#[path]` file, as rustc treats it).
fn mounted(file: &Path, owner: bool, name: &str, path: Option<&str>) -> Option<(PathBuf, bool)> {
    let dir = file.parent().unwrap();
    if let Some(path) = path {
        return Some((dir.join(path), true));
    }
    let base = if owner {
        dir.to_path_buf()
    } else {
        dir.join(file.file_stem().unwrap())
    };
    let flat = base.join(format!("{name}.rs"));
    let nested = base.join(name).join("mod.rs");
    if flat.is_file() {
        Some((flat, false))
    } else if nested.is_file() {
        Some((nested, true))
    } else {
        None
    }
}

/// Every file reachable through `mod` declarations from `roots` (crate roots), canonical.
fn reachable(roots: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut work: Vec<(PathBuf, bool)> = roots.iter().map(|r| (r.clone(), true)).collect();
    while let Some((file, owner)) = work.pop() {
        let Ok(canon) = file.canonicalize() else {
            continue;
        };
        if !seen.insert(canon.clone()) {
            continue;
        }
        let text = std::fs::read_to_string(&canon).unwrap();
        for (name, path) in mod_decls(&text) {
            if let Some(next) = mounted(&canon, owner, &name, path.as_deref()) {
                work.push(next);
            }
        }
    }
    seen
}

/// The `[[test]]` paths of `crate_dir`'s `Cargo.toml`, absolute.
fn test_targets(crate_dir: &Path) -> Vec<PathBuf> {
    let manifest = std::fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();
    let manifest: toml::Table = manifest.parse().unwrap();
    manifest
        .get("test")
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("path")?.as_str())
        .map(|p| crate_dir.join(p))
        .collect()
}

/// Every `.rs` file under `crate_dir/tests` that no `[[test]]` target compiles, one
/// Exact-text line each, named relative to `crates`. `fixtures/` holds data, not code.
pub(super) fn target_hits(crates: &Path, crate_dir: &Path) -> Vec<String> {
    let targets = test_targets(crate_dir);
    let listed: BTreeSet<PathBuf> = targets
        .iter()
        .filter_map(|t| t.canonicalize().ok())
        .collect();
    let reached = reachable(&targets);
    let tests = crate_dir.join("tests");
    let mut files = Vec::new();
    rust_files(&tests, &mut files);
    files.sort();
    let mut hits = Vec::new();
    for file in files {
        let rel_tests = file.strip_prefix(&tests).unwrap();
        if rel_tests.components().any(|c| c.as_os_str() == "fixtures") {
            continue;
        }
        let canon = file.canonicalize().unwrap();
        let rel = file
            .strip_prefix(crates)
            .unwrap_or(&file)
            .display()
            .to_string();
        if rel_tests.components().count() == 1 && !listed.contains(&canon) {
            hits.push(format!(
                "{rel}: no [[test]] entry in Cargo.toml, so it is never compiled (autotests = false); \
                 put it in tests/it/ with a `mod` line in tests/it/main.rs"
            ));
        } else if !reached.contains(&canon) {
            hits.push(format!(
                "{rel}: no `mod` line reaches it from a [[test]] target, so it is never compiled; \
                 mount it (tests/it/main.rs or its parent module)"
            ));
        }
    }
    hits
}
