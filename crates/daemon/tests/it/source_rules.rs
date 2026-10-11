//! Source rules over the merged integration-test binaries (milestone 9.12, decision 14).
//!
//! The merged binary runs its tests on threads of one process under `cargo test`, so a
//! test that changes the process's environment, working directory or umask races every
//! other test in it: a write to `environ` is a data race with any concurrent reader, and
//! every process spawn reads it (`crates/daemon/tests/git_env.rs`). Such a file is kept
//! alone as its own `[[test]]`; this rule fails when one lands under a merged `tests/it/`.

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

/// Every hit of a process-state call outside a comment line in `crates/<crate>/tests/it`,
/// one Exact-text line each. A crate without a merged binary yet contributes none.
fn hits(crates: &Path) -> Vec<String> {
    // Built so this file does not name the calls itself.
    let calls = [
        ["env::", "set_var"].concat(),
        ["env::", "remove_var"].concat(),
        ["set_current", "_dir"].concat(),
        ["libc::", "umask"].concat(),
    ];
    let mut files = Vec::new();
    for merged in ["daemon/tests/it", "cli/tests/it"] {
        rust_files(&crates.join(merged), &mut files);
    }
    files.sort();
    let mut hits = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let rel = file.strip_prefix(crates).unwrap().display().to_string();
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for call in calls.iter().filter(|c| line.contains(c.as_str())) {
                hits.push(format!(
                    "{rel}:{}: {call} in a merged test binary; move the file to a binary of its own (decision 14)",
                    n + 1
                ));
            }
        }
    }
    hits
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
