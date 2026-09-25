//! M8a final fix batch F2 round 2 (the F2 review's recommendation, item 2): a confined
//! check, proof or `setup` runs worker-written code with its checkout writable, so it
//! could plant agent config there, including from a child that outlives it (a `setsid`
//! process escapes the command's group kill, but never its sandbox). The profile denies
//! every write to the protected agent-config paths of its checkout, unconditionally:
//! `.claude` and `.codex` (the directories themselves too), `.mcp.json`, and `CLAUDE.md`
//! and `AGENTS.md` at any depth. Real `sandbox-exec`, harmless payloads in the test's
//! own temporary directories.

#![cfg(target_os = "macos")]

mod support;

use daemon::run::exec::run_confined;
use std::time::Duration;
use support::confine::world;

const LONG: Duration = Duration::from_secs(60);

#[test]
fn a_confined_command_cannot_write_protected_agent_config() {
    let w = world();
    let task = w.task();
    std::fs::create_dir_all(task.join("sub")).unwrap();
    std::fs::create_dir_all(task.join(".claude")).unwrap();
    std::fs::write(task.join(".claude/tracked.json"), "{}").unwrap();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let attempts = [
        "mkdir .codex",
        "printf x > .claude/settings.json",
        "printf x > .mcp.json",
        "printf x > CLAUDE.md",
        "printf x > AGENTS.md",
        "printf x > sub/CLAUDE.md",
        "printf x > sub/AGENTS.md",
        "ln -s /tmp .codex",
        "mv .claude .claude-moved",
        "ln -s /tmp sub/.claude-link && mv sub/.claude-link .codex",
        // A hard link to a protected file, then a write through it.
        "ln .claude/tracked.json hard && printf x >> hard",
    ];
    let mut command = String::new();
    for attempt in attempts {
        command.push_str(&format!(
            "({attempt}) 2>/dev/null && echo 'WROTE: {attempt}'; "
        ));
    }
    command.push_str("printf ok > built.txt && printf ok > sub/notes.md && echo done");

    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    assert!(outcome.ok, "{outcome:?}");
    assert!(!outcome.tail.contains("WROTE"), "{}", outcome.tail);
    assert!(outcome.tail.ends_with("done"), "{}", outcome.tail);
    for path in [
        ".codex",
        ".claude/settings.json",
        ".mcp.json",
        "CLAUDE.md",
        "AGENTS.md",
        "sub/CLAUDE.md",
        "sub/AGENTS.md",
        ".claude-moved",
    ] {
        assert!(
            std::fs::symlink_metadata(task.join(path)).is_err(),
            "{path} was written"
        );
    }
    assert!(task.join(".claude").is_dir());
    assert_eq!(
        std::fs::read_to_string(task.join(".claude/tracked.json")).unwrap(),
        "{}"
    );
    // Other files of the checkout stay writable.
    assert!(task.join("built.txt").is_file());
    assert!(task.join("sub/notes.md").is_file());

    // Unconfined, the same writes land: the payloads are live.
    let outcome = run_confined(
        &task,
        "printf x > CLAUDE.md && mkdir .codex",
        &[],
        LONG,
        None,
    );
    assert!(outcome.ok, "{outcome:?}");
    assert!(task.join("CLAUDE.md").is_file() && task.join(".codex").is_dir());
}

/// F4 (the F2 re-review's I1): macOS's default volume ignores case, so `.CODEX/`,
/// `agents.md` or `sub/Claude.md` is the same file Codex and Claude open as `.codex/`,
/// `AGENTS.md` or `sub/CLAUDE.md`. The deny ignores case too. (On macOS 26.2 the
/// kernel's matching already folds case on such a volume, so this passed before the
/// profile spelled the names with bracket classes; it pins the property either way.)
#[test]
fn a_confined_command_cannot_write_protected_agent_config_in_another_case() {
    let w = world();
    let task = w.task();
    std::fs::create_dir_all(task.join("sub")).unwrap();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let attempts = [
        "mkdir .CODEX",
        "mkdir .Codex",
        "mkdir .CLAUDE",
        "printf x > .MCP.json",
        "printf x > .Mcp.Json",
        "printf x > agents.md",
        "printf x > Claude.MD",
        "printf x > sub/Claude.md",
        "printf x > sub/agents.MD",
        "ln -s /tmp .Claude",
    ];
    let mut command = String::new();
    for attempt in attempts {
        command.push_str(&format!(
            "({attempt}) 2>/dev/null && echo 'WROTE: {attempt}'; "
        ));
    }
    command.push_str("printf ok > built.txt && echo done");

    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    assert!(outcome.ok, "{outcome:?}");
    assert!(!outcome.tail.contains("WROTE"), "{}", outcome.tail);
    assert!(outcome.tail.ends_with("done"), "{}", outcome.tail);
    let names: Vec<String> = std::fs::read_dir(&task)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_lowercase())
        .collect();
    for name in [".codex", ".claude", ".mcp.json", "agents.md", "claude.md"] {
        assert!(!names.iter().any(|n| n == name), "{name}: {names:?}");
    }
    assert!(names.iter().any(|n| n == "built.txt"), "{names:?}");
    assert_eq!(std::fs::read_dir(task.join("sub")).unwrap().count(), 0);
}

/// F4 (the F2 re-review's I2): published packages ship `AGENTS.md` and `CLAUDE.md`
/// (recharts does), so the nested deny leaves dependency and vendored trees alone at
/// any depth: an install and `rm -rf node_modules` work. The root files and nested ones
/// in source directories stay denied.
#[test]
fn a_confined_command_may_install_and_remove_dependencies_that_ship_agent_files() {
    let w = world();
    let task = w.task();
    std::fs::create_dir_all(task.join("src")).unwrap();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let dependency_dirs = [
        "node_modules/recharts",
        "packages/a/node_modules/@scope/x",
        ".venv/lib/site-packages/p",
        "venv/p",
        "vendor/github.com/x",
        "target/doc",
        "bower_components/x",
        ".yarn/cache/x",
        ".pnpm-store/v3/x",
    ];
    let mut command = String::from("set -e; ");
    for dir in dependency_dirs {
        command.push_str(&format!(
            "mkdir -p {dir}; printf x > {dir}/AGENTS.md; printf x > {dir}/CLAUDE.md; \
             test -f {dir}/AGENTS.md; "
        ));
    }
    command.push_str(
        "rm -rf node_modules packages .venv venv vendor target bower_components .yarn .pnpm-store; \
         set +e; mkdir -p src/node_modules_notes; ",
    );
    for attempt in [
        "printf x > src/AGENTS.md",
        "printf x > src/node_modules_notes/CLAUDE.md",
        "printf x > AGENTS.md",
        "printf x > CLAUDE.md",
    ] {
        command.push_str(&format!(
            "({attempt}) 2>/dev/null && echo 'WROTE: {attempt}'; "
        ));
    }
    command.push_str("echo done");

    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    assert!(outcome.ok, "{outcome:?}");
    assert!(!outcome.tail.contains("WROTE"), "{}", outcome.tail);
    assert!(outcome.tail.ends_with("done"), "{}", outcome.tail);
    assert!(
        !task.join("node_modules").exists(),
        "rm -rf node_modules failed"
    );
    for path in ["src/AGENTS.md", "AGENTS.md", "CLAUDE.md"] {
        assert!(!task.join(path).exists(), "{path} was written");
    }
}
