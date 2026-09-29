//! M9.13 review, item 0: no test rig may build a `ManagerConfig` with
//! `ManagerConfig::new`'s defaults, whose `claude_bin` and `codex_bin` are the real
//! `claude` and `codex` on `PATH`. Every rig starts from `ManagerConfig::for_tests`,
//! which pins all three agent programs to paths that do not exist. (A daemon test once
//! started a real `claude` orchestrator this way.) Production code builds its
//! configuration with `from_env`, so `new` has no caller outside the manager's config.

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
