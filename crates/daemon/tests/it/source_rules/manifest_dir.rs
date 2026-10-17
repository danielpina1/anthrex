//! The compile-time manifest-directory rule (split from `source_rules.rs`): every
//! `env!("CARGO_MANIFEST_DIR")` goes through `testexec::checkout_dir`.

use super::{BLINDERS, code_only, is_ident, rust_files};

/// Whether a macro's arguments start with the literal `CARGO_MANIFEST_DIR`, plain
/// (`"…"`) or raw (`r"…"`, `r#"…"#`, …).
fn names_manifest_dir(args: &str) -> bool {
    let args = args.trim_start();
    let (hashes, quoted) = match args.strip_prefix('r') {
        Some(raw) => {
            let hashes = raw.bytes().take_while(|&c| c == b'#').count();
            (hashes, &raw[hashes..])
        }
        None => (0, args),
    };
    quoted
        .strip_prefix("\"CARGO_MANIFEST_DIR\"")
        .is_some_and(|after| after.bytes().take(hashes).filter(|&c| c == b'#').count() == hashes)
}

/// Every `env!("CARGO_MANIFEST_DIR")` and `option_env!("CARGO_MANIFEST_DIR")` in `text`'s
/// code that is not directly the argument of `checkout_dir(` (the name itself, after
/// `::` or a non-identifier byte), as line numbers.
fn unguarded_manifest_dirs(text: &str) -> Vec<usize> {
    let code = code_only(text);
    let mut lines = Vec::new();
    for (at, _) in code.match_indices("env!") {
        let b = code.as_bytes();
        let start = if code[..at].ends_with("option_") {
            at - "option_".len()
        } else {
            at
        };
        if start > 0 && is_ident(b[start - 1]) {
            continue;
        }
        let args = text[at + "env!".len()..].trim_start();
        let Some(args) = args.strip_prefix('(') else {
            continue;
        };
        if !names_manifest_dir(args) {
            continue;
        }
        // Decision 15(c): `checkout_dir` itself, alone or after `::`, not `my_checkout_dir`.
        let guarded = text[..start]
            .trim_end()
            .strip_suffix('(')
            .and_then(|before| before.trim_end().strip_suffix("checkout_dir"))
            .is_some_and(|before| before.bytes().last().is_none_or(|c| !is_ident(c)));
        if !guarded {
            lines.push(code[..at].matches('\n').count() + 1);
        }
    }
    lines
}

#[test]
fn the_manifest_scan_wants_checkout_dir_around_each_use() {
    let text = "let a = testexec::checkout_dir(env!(\"CARGO_MANIFEST_DIR\"));\n\
                let b = checkout_dir(\n    option_env!(\"CARGO_MANIFEST_DIR\").unwrap(),\n);\n\
                let c = Path::new(env!(\"CARGO_MANIFEST_DIR\"));\n\
                // env!(\"CARGO_MANIFEST_DIR\")\n\
                let d = \"env!(\\\"CARGO_MANIFEST_DIR\\\")\";\n\
                let e = env!(\"CARGO_PKG_NAME\");\n\
                let f = std::env::var(\"CARGO_MANIFEST_DIR\");\n\
                let g = Path::new(env!(r\"CARGO_MANIFEST_DIR\"));\n\
                let h = Path::new(env!(r#\"CARGO_MANIFEST_DIR\"#));\n\
                let i = testexec::checkout_dir(env!(r##\"CARGO_MANIFEST_DIR\"##));\n";
    assert_eq!(unguarded_manifest_dirs(text), [5, 10, 11]);
    for blinder in BLINDERS {
        let text = format!("{blinder}let c = Path::new(env!(\"CARGO_MANIFEST_DIR\"));\n");
        assert_eq!(unguarded_manifest_dirs(&text), [2], "after {blinder:?}");
    }
}

#[test]
fn every_compile_time_manifest_dir_is_guarded() {
    let crates = testexec::checkout_dir(env!("CARGO_MANIFEST_DIR"));
    let crates = crates.parent().unwrap();
    let mut files = Vec::new();
    rust_files(crates, &mut files);
    files.sort();
    let mut hits = Vec::new();
    for file in files {
        let rel = file.strip_prefix(crates).unwrap();
        if rel.components().any(|c| c.as_os_str() == "target") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for line in unguarded_manifest_dirs(&text) {
            hits.push(format!(
                "{}:{line}: env!(\"CARGO_MANIFEST_DIR\") is not passed through testexec::checkout_dir",
                rel.display()
            ));
        }
    }
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}

/// Decision 15(c): the guard is `checkout_dir(` itself, alone or after a path's `::`,
/// not any function whose name ends in it.
#[test]
fn my_checkout_dir_is_not_the_guard() {
    let text = "let a = my_checkout_dir(env!(\"CARGO_MANIFEST_DIR\"));\n\
                let b = testexec::checkout_dir(env!(\"CARGO_MANIFEST_DIR\"));\n\
                let c = checkout_dir(env!(\"CARGO_MANIFEST_DIR\"));\n\
                let d = testexec :: checkout_dir(env!(\"CARGO_MANIFEST_DIR\"));\n\
                let e = x.my_checkout_dir(env!(\"CARGO_MANIFEST_DIR\"));\n\
                let f = &checkout_dir(env!(\"CARGO_MANIFEST_DIR\"));\n";
    assert_eq!(unguarded_manifest_dirs(text), [1, 5]);
}
