//! The one-local-clock rule (milestone 9.19, decision 3, rulings R2 and R4). The local
//! zone is read in one file, `common/src/localtime.rs`: nothing else calls `localtime_r`
//! or reads `tm_gmtoff`. And `common::localtime` itself is called only where reading
//! the zone belongs: the TUI's event loop (`tui/src/lib.rs`), the CLI's command entry
//! points, `common` and `fake-agent`. Never the client's pure state (`tui/src/app/`,
//! `tui/src/ui/`), and never the daemon, whose text is UTC.

use super::{code_only, is_ident, rust_files};
use std::path::Path;

/// The one file that reads the zone, relative to `crates/`.
const READER: &str = "common/src/localtime.rs";

/// The libc names only [`READER`] may use: the reentrant clock, its non-reentrant twin,
/// the reverse conversion, the struct and its offset field (review round 1, Minor 2).
const LIBC_NAMES: [&str; 5] = [
    "localtime_r",
    "tm_gmtoff",
    "libc::localtime",
    "libc::mktime",
    "libc::tm",
];

/// The client's pure state, where no `libc` path may appear at all (hard rule 5).
const PURE_DIRS: [&str; 2] = ["tui/src/app/", "tui/src/ui/"];

/// Where `localtime` (the module) may be named, relative to `crates/`: a file, or a
/// directory ending in `/`.
const CALLERS: [&str; 6] = [
    "common/src/",
    "fake-agent/src/",
    "tui/src/lib.rs",
    "cli/src/run_cmd.rs",
    "cli/src/hold_cmd.rs",
    "cli/src/profile_cmd.rs",
];

/// Each whole-word `name` in `text`'s code (not its comments or literals), as lines.
fn word_lines(text: &str, name: &str) -> Vec<usize> {
    let code = code_only(text);
    let b = code.as_bytes();
    code.match_indices(name)
        .filter(|(at, _)| {
            let before = *at == 0 || !is_ident(b[at - 1]);
            let after = b.get(at + name.len()).is_none_or(|&c| !is_ident(c));
            before && after
        })
        .map(|(at, _)| code[..at].matches('\n').count() + 1)
        .collect()
}

/// One line per use out of place in the file `rel` (relative to `crates/`).
fn clock_hits(rel: &str, text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    if rel != READER {
        for name in LIBC_NAMES {
            for line in word_lines(text, name) {
                hits.push(format!(
                    "{rel}:{line}: {name} outside {READER} (milestone 9.19, decision 3)"
                ));
            }
        }
    }
    if PURE_DIRS.iter().any(|d| rel.starts_with(d)) {
        for line in word_lines(text, "libc") {
            hits.push(format!(
                "{rel}:{line}: libc in the client's pure state (hard rule 5)"
            ));
        }
    }
    let allowed = CALLERS.iter().any(|c| match c.strip_suffix('/') {
        Some(dir) => rel.starts_with(&format!("{dir}/")),
        None => rel == *c,
    });
    if !allowed {
        for line in word_lines(text, "localtime") {
            hits.push(format!(
                "{rel}:{line}: localtime named outside the event loop and the CLI's entry points (ruling R4)"
            ));
        }
    }
    hits
}

#[test]
fn one_local_clock() {
    let package = testexec::checkout_dir(env!("CARGO_MANIFEST_DIR"));
    let crates = package.join("..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).unwrap() {
        rust_files(&entry.unwrap().path().join("src"), &mut files);
    }
    files.sort();
    assert!(
        files.iter().any(|f| f.ends_with(READER)),
        "{READER} is missing"
    );
    let mut hits = Vec::new();
    for file in &files {
        let rel = rel_of(&crates, file);
        hits.extend(clock_hits(&rel, &std::fs::read_to_string(file).unwrap()));
    }
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}

fn rel_of(crates: &Path, file: &Path) -> String {
    file.strip_prefix(crates).unwrap().display().to_string()
}

/// The scanner itself, on constructed files.
#[test]
fn the_clock_scan_finds_each_form() {
    let libc = "fn f() {\n    unsafe { libc::localtime_r(&t, &mut tm) };\n    tm.tm_gmtoff\n}\n";
    assert!(clock_hits(READER, libc).is_empty());
    let hits = clock_hits("tui/src/lib.rs", libc);
    assert_eq!(hits.len(), 2, "{hits:#?}");
    assert!(
        hits[0].starts_with("tui/src/lib.rs:2: localtime_r outside"),
        "{hits:#?}"
    );
    assert!(
        hits[1].starts_with("tui/src/lib.rs:3: tm_gmtoff outside"),
        "{hits:#?}"
    );

    let call = "fn f() -> i32 {\n    common::localtime::offset_secs(0)\n}\n";
    for rel in [
        "tui/src/lib.rs",
        "cli/src/run_cmd.rs",
        "common/src/numbers.rs",
        "fake-agent/src/quota_steps.rs",
    ] {
        assert!(clock_hits(rel, call).is_empty(), "{rel}");
    }
    for rel in [
        "tui/src/app/tick.rs",
        "tui/src/ui/conversation.rs",
        "tui/src/inspector/run.rs",
        "daemon/src/run/engine/brakes.rs",
        "cli/src/run_cmd/status.rs",
    ] {
        let hits = clock_hits(rel, call);
        assert_eq!(hits.len(), 1, "{rel}: {hits:#?}");
        assert!(
            hits[0].starts_with(&format!("{rel}:2: localtime named")),
            "{hits:#?}"
        );
    }
    // Review round 1: the non-reentrant clock and its kin are the reader's alone, even in
    // a file that may call `localtime::`; and the pure state names no `libc` at all.
    let other = "fn f() {\n    unsafe { libc::localtime(&t) };\n    let tm: libc::tm = z;\n    libc::mktime(&mut tm)\n}\n";
    assert!(clock_hits(READER, other).is_empty());
    let hits = clock_hits("cli/src/run_cmd.rs", other);
    assert_eq!(hits.len(), 3, "{hits:#?}");
    assert!(
        hits[0].starts_with("cli/src/run_cmd.rs:2: libc::localtime outside"),
        "{hits:#?}"
    );
    let time = "fn f() -> i64 {\n    unsafe { libc::time(std::ptr::null_mut()) }\n}\n";
    for rel in ["tui/src/app/tick.rs", "tui/src/ui/settings.rs"] {
        let hits = clock_hits(rel, time);
        assert_eq!(hits.len(), 1, "{rel}: {hits:#?}");
        assert!(hits[0].starts_with(&format!("{rel}:2: libc in the client's pure state")));
    }
    assert!(clock_hits("tui/src/lib.rs", time).is_empty());
    // An import is a use; a comment, a literal and a longer name are not.
    let import = "use common::{localtime, numbers};\n";
    assert_eq!(clock_hits("daemon/src/lib.rs", import).len(), 1);
    let quiet =
        "// localtime::offset_secs\nconst A: &str = \"localtime_r\";\nfn localtime_ok() {}\n";
    assert!(clock_hits("daemon/src/lib.rs", quiet).is_empty());
}
