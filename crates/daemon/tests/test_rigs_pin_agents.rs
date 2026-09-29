//! M9.13 review, item 0: no test rig may build a `ManagerConfig` with
//! `ManagerConfig::new`'s defaults, whose `claude_bin` and `codex_bin` are the real
//! `claude` and `codex` on `PATH`. Every rig starts from `ManagerConfig::for_tests`,
//! which pins all three agent programs to paths that do not exist. (A daemon test once
//! started a real `claude` orchestrator this way.) Production code builds its
//! configuration with `from_env`, so `new` has no caller outside the manager's config.
//!
//! The re-review widened it to the other two ways a test reaches an agent program: a
//! `ManagerConfig::from_vars` built from a variable map, and an `anthrex` process (a
//! daemon it starts, or one a client command starts for it). These are file-level
//! checks on the source text, so their limits are recorded with them: a test file that
//! names the three variables passes whether or not every command in it sets them, and a
//! command built by a helper in another file is judged by that helper's file.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_rig_builds_a_manager_config_with_the_real_agent_programs() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let allowed = crates.join("daemon/src/manager/config.rs");
    let this = Path::new(file!()).file_name().unwrap().to_owned();
    let mut files = Vec::new();
    rust_files(crates, &mut files);
    assert!(files.len() > 100, "the walk found {} files", files.len());
    // Built so this file does not name the call itself.
    let call = ["ManagerConfig", "::new("].concat();
    let offenders: Vec<String> = files
        .iter()
        .filter(|f| **f != allowed && f.file_name() != Some(this.as_os_str()))
        .filter(|f| std::fs::read_to_string(f).unwrap().contains(&call))
        .map(|f| f.strip_prefix(crates).unwrap().display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "use ManagerConfig::for_tests, which pins claude, codex and the decider: {offenders:#?}"
    );
}

/// The sources a test is built from: anything under a `tests` directory, and the
/// in-crate test modules (`*_tests.rs`, `tests_*.rs`, `tests.rs`, `test_support.rs`).
fn is_test_source(path: &Path) -> bool {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    path.components().any(|c| c.as_os_str() == "tests")
        || name.ends_with("_tests.rs")
        || name.starts_with("tests_")
        || name == "tests.rs"
        || name == "test_support.rs"
}

const AGENT_VARS: [&str; 3] = [
    "ANTHREX_CLAUDE_BIN",
    "ANTHREX_CODEX_BIN",
    "ANTHREX_DECIDER_BIN",
];

#[test]
fn every_test_that_starts_anthrex_or_reads_variables_pins_the_agents() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let this = Path::new(file!()).file_name().unwrap().to_owned();
    let mut files = Vec::new();
    rust_files(crates, &mut files);
    let from_vars = ["ManagerConfig", "::from_vars("].concat();
    let spawns = [
        ["Command::new(", "ANTHREX)"].concat(),
        ["CARGO_BIN_EXE_", "anthrex\")"].concat(),
    ];
    let daemon_start = ["\"daemon\", ", "\"start\""].concat();
    let mut offenders = Vec::new();
    for f in files.iter().filter(|f| is_test_source(f)) {
        if f.file_name() == Some(this.as_os_str()) {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap();
        let rel = f.strip_prefix(crates).unwrap().display().to_string();
        // The CLI tests' `pin_agents` helper sets all three.
        let helper = text.contains("pin_agents(");
        let missing: Vec<&str> = AGENT_VARS
            .iter()
            .copied()
            .filter(|v| !helper && !text.contains(v))
            .collect();
        // `from_vars` needs the claude and codex variables in its map.
        if text.contains(&from_vars) && missing.iter().any(|v| *v != "ANTHREX_DECIDER_BIN") {
            offenders.push(format!("{rel}: from_vars without {missing:?}"));
        }
        // A file that builds an `anthrex` command sets all three.
        if spawns.iter().any(|s| text.contains(s)) && !missing.is_empty() {
            offenders.push(format!("{rel}: starts anthrex without {missing:?}"));
        }
        // A daemon is started only through the helpers that pin them.
        for (n, line) in text.lines().enumerate() {
            let pinned = line.contains("isolated_command(") || line.contains("self.command(");
            if line.contains(&daemon_start) && !pinned {
                offenders.push(format!(
                    "{rel}:{}: daemon start outside the pinned helpers",
                    n + 1
                ));
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}
