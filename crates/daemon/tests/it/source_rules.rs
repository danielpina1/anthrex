//! Source rules over the merged integration-test binaries (milestone 9.12, decision 14).
//!
//! The merged binary runs its tests on threads of one process under `cargo test`, so a
//! test that changes the process's environment, working directory or umask races every
//! other test in it: a write to `environ` is a data race with any concurrent reader, and
//! every process spawn reads it (`crates/daemon/tests/git_env.rs`). Such a file is kept
//! alone as its own `[[test]]`; this rule fails when one lands under a merged `tests/it/`
//! or in the `tests/support/` code that binary mounts.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The calls that change the process's environment, working directory or umask, by the
/// name they are called with, so `env::set_var(…)`, `std::env::set_var(…)` and a bare
/// `set_var(…)` after `use std::env::set_var;` are all caught.
const CALLS: [&str; 4] = ["set_var", "remove_var", "set_current_dir", "umask"];

/// `text` with every comment (`//` to the end of the line, `/* */` nested) and every
/// string or character literal's contents blanked to spaces; newlines are kept, so a
/// byte's line number is unchanged.
fn code_only(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, i: usize| {
        if out[i] != b'\n' {
            out[i] = b' ';
        }
    };
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                blank(&mut out, i);
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            let mut depth = 0;
            while i < b.len() {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, i);
                    i += 1;
                }
            }
        } else if b[i] == b'r'
            && (b[i + 1..].starts_with(b"\"") || b[i + 1..].starts_with(b"#"))
            && (i == 0 || !is_ident(b[i - 1]))
        {
            // A raw string: r"…", r#"…"#, …
            let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
            if b.get(i + 1 + hashes) != Some(&b'"') {
                i += 1;
                continue;
            }
            let close: Vec<u8> = std::iter::once(b'"')
                .chain(std::iter::repeat_n(b'#', hashes))
                .collect();
            i += 2 + hashes;
            while i < b.len() && !b[i..].starts_with(&close) {
                blank(&mut out, i);
                i += 1;
            }
            i += close.len();
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    blank(&mut out, i);
                    i += 1;
                }
                if i < b.len() {
                    blank(&mut out, i);
                }
                i += 1;
            }
            i += 1;
        } else if b[i] == b'\'' && (b.get(i + 2) == Some(&b'\'') || b.get(i + 1) == Some(&b'\\')) {
            // A character literal ('x' or '\…'); a lifetime has no closing quote.
            i += 1;
            while i < b.len() && b[i] != b'\'' {
                if b[i] == b'\\' {
                    blank(&mut out, i);
                    i += 1;
                }
                blank(&mut out, i);
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Every call of one of [`CALLS`] in `text`'s code (not its comments or literals), as
/// (line, name). A call may be split across lines before its `(`.
fn calls_in(text: &str) -> Vec<(usize, &'static str)> {
    let code = code_only(text);
    let b = code.as_bytes();
    let mut found = Vec::new();
    for name in CALLS {
        for (at, _) in code.match_indices(name) {
            let before_ok = at == 0 || !is_ident(b[at - 1]);
            let rest = &code[at + name.len()..];
            if before_ok && rest.trim_start().starts_with('(') {
                found.push((code[..at].matches('\n').count() + 1, name));
            }
        }
    }
    found.sort();
    found
}

/// Every hit in the merged binaries' sources, `crates/<crate>/tests/it` and the
/// `crates/<crate>/tests/support` they mount, one Exact-text line each. A crate without
/// a merged binary yet contributes none.
fn hits(crates: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for dir in [
        "daemon/tests/it",
        "daemon/tests/support",
        "cli/tests/it",
        "cli/tests/support",
    ] {
        rust_files(&crates.join(dir), &mut files);
    }
    files.sort();
    let mut hits = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let rel = file.strip_prefix(crates).unwrap().display().to_string();
        for (line, call) in calls_in(&text) {
            hits.push(format!(
                "{rel}:{line}: {call} in a merged test binary; move the file to a binary of its own (decision 14)"
            ));
        }
    }
    hits
}

/// The scanner itself: the forms a call takes, and the places a name is not a call.
#[test]
fn the_environment_scan_finds_calls_however_imported() {
    let text = "use std::env::set_var;\n\
                fn f() {\n\
                    unsafe { set_var(\"A\", \"1\") };\n\
                    unsafe { std::env::remove_var\n(\"A\") };\n\
                    let _ = std::env::set_current_dir(\"/\");\n\
                    unsafe { libc::umask(0o22) };\n\
                }\n";
    assert_eq!(
        calls_in(text),
        [
            (3, "set_var"),
            (4, "remove_var"),
            (6, "set_current_dir"),
            (7, "umask")
        ]
    );
    let quiet = "// set_var(x)\n\
                 /* remove_var(x)\n /* nested */ umask(0) */\n\
                 let s = \"set_current_dir(x)\";\n\
                 let r = r#\"umask(\"0\")\"#;\n\
                 let c = '\"'; fn my_set_var() {} my_set_var();\n";
    assert_eq!(calls_in(quiet), []);
}

#[test]
fn no_merged_test_mutates_the_process_environment() {
    // M9.12.11 passes this through `testexec::checkout_dir` (decision 25).
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert!(
        crates.join("daemon/tests/it/main.rs").is_file(),
        "the walk starts at {}",
        crates.display()
    );
    let hits = hits(crates);
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}

// ---------------------------------------------------------------------------------
// The engine-purity rule (milestone 9.12, decisions 16 and 17); the scanner is `engine`.

mod engine;
use engine::{engine_hits, engine_scan, production_lines};

#[test]
fn the_scanner_finds_each_forbidden_form() {
    let forbidden = [
        "use tokio::sync::mpsc;",
        "tokio::spawn(f)",
        "use std::fs;",
        "let t = std::fs::read(p)?;",
        "use std::{collections::BTreeMap, process::Command};",
        "use std::{\n    collections::BTreeMap,\n    thread,\n};",
        "let v = std::env::var(\"X\");",
        "let t = SystemTime::now();",
        "let e = UNIX_EPOCH;",
        "let t = Instant::now();",
        "unsafe { libc::kill(1, 0) };",
        "let s = portable_pty::native_pty_system();",
    ];
    for text in forbidden {
        assert!(!engine_scan(text).is_empty(), "not found: {text}");
    }
    let allowed = [
        "use std::time::Duration;",
        "use std::path::PathBuf;",
        "use std::{collections::BTreeMap, time::Duration};",
        "// tokio is not used here",
        "/// no std::fs",
        "let s = \"std::fs and tokio\";",
    ];
    for text in allowed {
        assert_eq!(engine_scan(text), [], "found in: {text}");
    }
    // The line of a grouped import's offending name, not of the `use`.
    assert_eq!(
        engine_scan("use std::{\n    collections::BTreeMap,\n    thread,\n};"),
        [(3, "std::thread")]
    );
}

#[test]
fn the_scanner_skips_test_code() {
    let dir = tempfile::tempdir().unwrap();
    let engine = dir.path().join("engine");
    let put = |rel: &str, text: &str| {
        let path = engine.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    put(
        "mod.rs",
        "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;\n\
         #[cfg(test)]\nmod more {\n    use std::fs;\n}\n\
         pub(crate) fn f() {}\n\
         #[cfg(test)]\npub(crate) use ended::REBASELINED;\n\
         #[cfg(test)]\npub(super) fn waits(run: &Run) -> bool {\n    let x = { 1 };\n    std::thread::yield_now();\n    x == 1\n}\n\
         #[cfg(test)]\n#[allow(dead_code)]\nstruct S {\n    t: std::time::SystemTime,\n}\n",
    );
    put("x_tests.rs", "use std::fs;\n");
    put("tests/a.rs", "use tokio::time;\n");
    put(
        "y.rs",
        "let s = \"#[cfg(test)] use std::fs\";\nuse std::fs;\n",
    );
    let hits = engine_hits(&engine);
    assert_eq!(hits.len(), 1, "{hits:#?}");
    assert!(hits[0].starts_with("run/engine/y.rs:2: std::fs in engine code"));
    assert!(hits[0].ends_with("use std::fs;"));

    // The production code after a skipped item is still scanned.
    let text = "#[cfg(test)]\npub(super) fn waits() {\n    use std::fs;\n}\nuse std::fs;\n\
                #[cfg(test)]\npub(crate) use a::B;\nuse std::process;\n";
    assert_eq!(engine_scan(text), [(5, "std::fs"), (8, "std::process")]);
    let kept: Vec<usize> = production_lines(text).iter().map(|l| l.0).collect();
    assert_eq!(kept, [5, 8]);
}

#[test]
fn the_engine_imports_no_runtime_io_or_clock() {
    // M9.12.11 passes this through `testexec::checkout_dir` (decision 25).
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let engine = crates.join("daemon/src/run/engine");
    assert!(
        engine.join("mod.rs").is_file(),
        "the walk starts at {}",
        engine.display()
    );
    let hits = engine_hits(&engine);
    assert!(
        hits.is_empty(),
        "the engine imports runtime, I/O or clock APIs:\n{}",
        hits.join("\n")
    );
}
