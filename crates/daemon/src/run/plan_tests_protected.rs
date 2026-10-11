//! Milestone 9.11 decision 33 (D6): the agents' override and local instruction files and
//! the shared `.agents/` directory are protected like `CLAUDE.md` and `AGENTS.md`. Its own
//! file because `plan_tests.rs` is at the 600-line rule (pre-flight P8).

use super::*;
use crate::run::globs::ProtectedMatcher;

#[test]
fn the_agents_override_files_are_protected() {
    let builtins: Vec<String> = BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect();
    let matcher = ProtectedMatcher::new(&builtins).unwrap();
    for path in [
        "docs/AGENTS.override.md",
        "AGENTS.override.md",
        "CLAUDE.local.md",
        "crates/x/CLAUDE.local.md",
        ".agents/skills/x",
    ] {
        assert!(matcher.matches(path), "{path} is not protected");
    }
    // A neighbour that only looks alike stays writable.
    for path in ["docs/AGENTS.md.txt", "src/agents/x.rs"] {
        assert!(!matcher.matches(path), "{path} is protected");
    }
}
