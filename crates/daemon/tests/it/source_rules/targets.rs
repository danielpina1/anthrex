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

/// Each out-of-line `mod <name>;` in `code` (already blanked by `code_only`), as (offset
/// of its `mod`, name, the inline `mod a { … }` bodies it sits in, outermost first). The
/// declarations are `mod_decls`'s, found by the same test and in the same order.
fn decl_sites(code: &str) -> Vec<(usize, String, Vec<String>)> {
    let b = code.as_bytes();
    let mut sites = Vec::new();
    // Each open inline module, with the brace depth inside its body.
    let mut inline: Vec<(String, usize)> = Vec::new();
    let mut pending: Option<String> = None;
    let mut depth = 0usize;
    for (at, &c) in b.iter().enumerate() {
        match c {
            b'{' => {
                depth += 1;
                if let Some(name) = pending.take() {
                    inline.push((name, depth));
                }
            }
            b'}' => {
                while inline.last().is_some_and(|(_, d)| *d == depth) {
                    inline.pop();
                }
                depth = depth.saturating_sub(1);
            }
            b'm' if code[at..].starts_with("mod") && (at == 0 || !is_ident(b[at - 1])) => {
                let rest = &code[at + 3..];
                if !rest.starts_with(|c: char| c.is_whitespace()) {
                    continue;
                }
                let rest = rest.trim_start();
                let len = rest.bytes().take_while(|&c| is_ident(c)).count();
                let after = rest[len..].trim_start();
                if len == 0 {
                    continue;
                }
                let name = rest[..len].to_string();
                if after.starts_with(';') {
                    let within = inline.iter().map(|(n, _)| n.clone()).collect();
                    sites.push((at, name, within));
                } else if after.starts_with('{') {
                    pending = Some(name);
                }
            }
            _ => {}
        }
    }
    sites
}

/// The index of the `open` byte that matches the `close` byte at `at`, walking back.
fn matching_open(b: &[u8], at: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0usize;
    for k in (0..=at).rev() {
        if b[k] == close {
            depth += 1;
        } else if b[k] == open {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

/// The predicate of each `#[cfg(…)]` on the item whose `mod` is at `at` in `code`, with
/// its whitespace removed (so a blanked `target_os = "…"` reads `target_os=""`). The
/// attributes are read back from `mod` past a visibility (`pub`, `pub(crate)`, …), each
/// matched by its brackets, so one may span lines.
fn cfgs_before(code: &str, at: usize) -> Vec<String> {
    let b = code.as_bytes();
    let mut head = code[..at].trim_end();
    if head.ends_with(')')
        && let Some(open) = matching_open(b, head.len() - 1, b'(', b')')
        && code[..open].trim_end().ends_with("pub")
    {
        head = code[..open].trim_end();
    }
    if let Some(before) = head.strip_suffix("pub")
        && before.bytes().last().is_none_or(|c| !is_ident(c))
    {
        head = before;
    }
    let mut preds = Vec::new();
    loop {
        head = head.trim_end();
        if !head.ends_with(']') {
            break;
        }
        let Some(open) = matching_open(b, head.len() - 1, b'[', b']') else {
            break;
        };
        if open == 0 || b[open - 1] != b'#' {
            break;
        }
        let attr: String = code[open + 1..head.len() - 1]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if let Some(pred) = attr.strip_prefix("cfg(").and_then(|p| p.strip_suffix(')')) {
            preds.push(pred.to_string());
        }
        head = &code[..open - 1];
    }
    preds
}

/// The file `mod name;` with `path` mounts from `file`, inside the inline modules
/// `inline` (decision 15(e): `mod a { mod b; }` in `x.rs` is `x/a/b.rs`), and whether
/// that file owns its directory (a crate root, a `mod.rs`, or any `#[path]` file, as
/// rustc treats it).
fn mounted(
    file: &Path,
    owner: bool,
    inline: &[String],
    name: &str,
    path: Option<&str>,
) -> Option<(PathBuf, bool)> {
    let dir = file.parent().unwrap();
    if let Some(path) = path
        && inline.is_empty()
    {
        return Some((dir.join(path), true));
    }
    let mut base = if owner {
        dir.to_path_buf()
    } else {
        dir.join(file.file_stem().unwrap())
    };
    base.extend(inline);
    if let Some(path) = path {
        return Some((base.join(path), true));
    }
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
        let decls = mod_decls(&text);
        let sites = decl_sites(&code_only(&text));
        assert_eq!(decls.len(), sites.len(), "{}", canon.display());
        for ((name, path), (_, _, inline)) in decls.into_iter().zip(sites) {
            if let Some(next) = mounted(&canon, owner, &inline, &name, path.as_deref()) {
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

/// Every `.rs` file under `crate_dir/tests` that no `[[test]]` target compiles, and every
/// `mod` line in a compiled one under a cfg that may be off (`cfg_disabled_mods`), one
/// Exact-text line each, named relative to `crates`. The crate's own `tests/fixtures/`
/// holds data, not code (decision 15(a): no other `fixtures` directory is exempt).
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
        if rel_tests
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == "fixtures")
        {
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
        } else {
            let text = std::fs::read_to_string(&file).unwrap();
            for line in cfg_disabled_mods(&text) {
                hits.push(format!(
                    "{rel}:{line}: this `mod` is under a #[cfg] that may be off when the tests \
                     build, so the file it mounts may never compile; review it (decision 15(d))"
                ));
            }
        }
    }
    hits
}

/// Decision 15(d): the line of each out-of-line `mod x;` whose `#[cfg(…)]` may be off
/// when the tests build. A file mounted only under such a cfg is reached by `reachable`
/// but may never compile, so the rule fails on its line for someone to review.
///
/// Allowed: `test`, `unix` and `target_os = …`, and `not(test)` when a `#[cfg(test)]`
/// twin of the same name sits in the same inline module.
pub(super) fn cfg_disabled_mods(text: &str) -> Vec<usize> {
    let code = code_only(text);
    let sites = decl_sites(&code);
    let cfgs: Vec<Vec<String>> = sites
        .iter()
        .map(|(at, ..)| cfgs_before(&code, *at))
        .collect();
    let has_test_twin = |name: &str, inline: &[String]| {
        sites
            .iter()
            .zip(&cfgs)
            .any(|((_, n, i), preds)| n == name && i == inline && preds.iter().any(|p| p == "test"))
    };
    sites
        .iter()
        .zip(&cfgs)
        .filter(|((_, name, inline), preds)| {
            preds.iter().any(|pred| match pred.as_str() {
                "test" | "unix" | "target_os=\"\"" => false,
                "not(test)" => !has_test_twin(name, inline),
                _ => true,
            })
        })
        .map(|((at, ..), _)| code[..*at].matches('\n').count() + 1)
        .collect()
}

/// Decision 15(b): every crate under `crates` whose `Cargo.toml` sets `autotests = false`,
/// by directory name, sorted. Cargo builds only such a crate's declared `[[test]]`
/// targets, so these are the crates `target_hits` must walk.
pub(super) fn autotests_off_crates(crates: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(crates).unwrap() {
        let dir = entry.unwrap().path();
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let manifest: toml::Table = manifest.parse().unwrap();
        let autotests = manifest.get("package").and_then(|p| p.get("autotests"));
        if autotests.and_then(|a| a.as_bool()) == Some(false) {
            found.push(dir.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    found.sort();
    found
}

/// A crate `k` under a temporary `crates` directory, with `tests/it/main.rs` as its one
/// `[[test]]` target.
fn temp_crate() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let krate = dir.path().join("k");
    std::fs::create_dir_all(&krate).unwrap();
    std::fs::write(
        krate.join("Cargo.toml"),
        "[package]\nname = \"k\"\nautotests = false\n\
         [[test]]\nname = \"it\"\npath = \"tests/it/main.rs\"\n",
    )
    .unwrap();
    (dir, krate)
}

/// Writes `text` to `rel` under `krate`, making its directories.
fn put(krate: &Path, rel: &str, text: &str) {
    let path = krate.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The file each hit names, relative to `crates`.
fn named(hits: &[String]) -> Vec<&str> {
    hits.iter().map(|h| h.split(':').next().unwrap()).collect()
}

#[test]
fn a_cfg_disabled_mod_is_flagged() {
    let text = "#[cfg(any())] mod x;\n\
                #[cfg(feature = \"off\")]\nmod y;\n\
                #[cfg(test)] mod t;\n\
                #[cfg(target_os = \"linux\")]\npub(crate) mod l;\n\
                #[cfg(unix)]\nmod u;\n\
                #[cfg(test)]\n#[path = \"p.rs\"]\nmod p;\n\
                #[cfg(not(test))]\nmod alone;\n\
                #[cfg(not(test))]\nmod twin;\n#[cfg(test)]\n#[path = \"twin_test.rs\"]\nmod twin;\n\
                #[cfg(\n    windows\n)]\nmod w;\n\
                #[cfg(feature = \"x\")]\nmod inline {\n}\n\
                // #[cfg(any())] mod c;\n\
                mod plain;\n";
    assert_eq!(cfg_disabled_mods(text), [1, 3, 13, 22]);
    for blinder in super::BLINDERS {
        let text = format!("{blinder}#[cfg(any())]\nmod x;\n");
        assert_eq!(cfg_disabled_mods(&text), [3], "after {blinder:?}");
    }

    // `target_hits` reports it, though the file it mounts is reached.
    let (dir, krate) = temp_crate();
    put(
        &krate,
        "tests/it/main.rs",
        "mod a;\n#[cfg(feature = \"off\")]\nmod b;\n",
    );
    put(&krate, "tests/it/a.rs", "");
    put(&krate, "tests/it/b.rs", "");
    let hits = target_hits(dir.path(), &krate);
    assert_eq!(named(&hits), ["k/tests/it/main.rs"], "{hits:#?}");
    assert!(hits[0].starts_with("k/tests/it/main.rs:3: "), "{}", hits[0]);
}

#[test]
fn an_inline_modules_child_resolves_under_it() {
    let (dir, krate) = temp_crate();
    put(
        &krate,
        "tests/it/main.rs",
        "mod x;\nmod top {\n    mod m;\n}\n",
    );
    put(
        &krate,
        "tests/it/x.rs",
        "mod a {\n    mod b;\n    mod c {\n        mod d;\n    }\n}\nmod e;\n",
    );
    for reached in ["x/a/b.rs", "x/a/c/d.rs", "x/e.rs", "top/m.rs"] {
        put(&krate, &format!("tests/it/{reached}"), "");
    }
    for stray in ["x/b.rs", "x/d.rs", "m.rs"] {
        put(&krate, &format!("tests/it/{stray}"), "");
    }
    let hits = target_hits(dir.path(), &krate);
    assert_eq!(
        named(&hits),
        ["k/tests/it/m.rs", "k/tests/it/x/b.rs", "k/tests/it/x/d.rs"],
        "{hits:#?}"
    );
}

#[test]
fn only_the_crate_root_fixtures_directory_is_exempt() {
    let (dir, krate) = temp_crate();
    put(&krate, "tests/it/main.rs", "");
    put(&krate, "tests/fixtures/data.rs", "");
    put(&krate, "tests/fixtures/deep/more.rs", "");
    put(&krate, "tests/it/fixtures/real_test.rs", "");
    let hits = target_hits(dir.path(), &krate);
    assert_eq!(
        named(&hits),
        ["k/tests/it/fixtures/real_test.rs"],
        "{hits:#?}"
    );
}

#[test]
fn every_autotests_off_crate_is_walked() {
    let dir = tempfile::tempdir().unwrap();
    for (name, manifest) in [
        ("off", "[package]\nname = \"off\"\nautotests = false\n"),
        ("on", "[package]\nname = \"on\"\nautotests = true\n"),
        ("unset", "[package]\nname = \"unset\"\n"),
        (
            "elsewhere",
            "[package]\nname = \"elsewhere\"\n[lib]\nautotests = false\n",
        ),
    ] {
        put(&dir.path().join(name), "Cargo.toml", manifest);
    }
    put(dir.path(), "not_a_crate/readme.txt", "");
    assert_eq!(autotests_off_crates(dir.path()), ["off"]);

    let package = testexec::checkout_dir(env!("CARGO_MANIFEST_DIR"));
    let crates = package.parent().unwrap();
    let found = autotests_off_crates(crates);
    for krate in ["cli", "daemon"] {
        assert!(found.iter().any(|k| k == krate), "{krate} in {found:?}");
    }
}
