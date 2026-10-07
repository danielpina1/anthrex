use super::*;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
use proto::{AgentRole, Effort, Runtime, Strength};
use std::path::PathBuf;

#[test]
fn reviewer_spec_is_read_only() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    let task = &run.tasks[0];
    let review = PathBuf::from("/tmp/wt/runs/add-password-reset-3f9a/t1.review");
    for runtime in [Runtime::Claude, Runtime::Codex] {
        let route = Route {
            runtime,
            model: "m".into(),
            strength: Strength::Standard,
            effort: Effort::MEDIUM,
        };
        let spec = reviewer_spec(&run, task, &route);
        assert_eq!(spec.cwd, review);
        assert_eq!(spec.runtime, runtime);
        assert_eq!(spec.effort, Effort::MEDIUM);
        assert_eq!(spec.instructions, crate::run::contract::REVIEWER_CONTRACT);
        assert_eq!(
            spec.allowed_tools,
            [
                "mcp__anthrex__submit_review",
                "Read",
                "Glob",
                "Grep",
                "Bash(git diff:*)",
                "Bash(git log:*)",
                "Bash(git show:*)"
            ]
        );
        assert_eq!(spec.mcp.as_ref().unwrap().role, AgentRole::Reviewer);
        assert_eq!(spec.codex_sandbox, "read-only");
        assert!(spec.codex_writable_roots.is_empty());
        if runtime == Runtime::Claude {
            // F1c round 3 (N4): a read-only sandbox, so `git diff --output` cannot
            // write.
            let sandbox = spec.claude_sandbox.as_ref().expect("read-only sandbox");
            assert!(sandbox.writable_roots.is_empty());
            assert_eq!(spec.claude_permission_mode.as_deref(), Some("dontAsk"));
            assert_eq!(
                spec.claude_disallowed_tools,
                ["Edit", "Write", "NotebookEdit"]
            );
        }
    }
    let worker = worker_spec(&run, task);
    let sandbox = worker.claude_sandbox.expect("workers run sandboxed");
    // Final fix batch F1b and F1c: nothing of the git common dir, only the task's
    // checkout's private object directory and its temporary directory, under the
    // run's data directory. The checkout's repository is self-describing: no
    // object-directory variable is set.
    let objects = PathBuf::from(format!("/tmp/data/runs/{}/tasks/t1/git/objects", run.id));
    // F1d: the temporary directory is short, under the daemon's own root.
    let tmp = crate::run::git::task_tmp(&PathBuf::from(format!(
        "/tmp/data/runs/{}/tasks/t1",
        run.id
    )));
    assert!(tmp.starts_with(crate::run::git::tmp_root()), "{tmp:?}");
    assert_eq!(sandbox.writable_roots, [objects, tmp.clone()]);
    // F1d (R5): `TMPDIR` is that directory, last, whatever the profile set.
    assert_eq!(
        worker.env.last(),
        Some(&("TMPDIR".to_string(), tmp.display().to_string()))
    );
    assert_eq!(worker.env.iter().filter(|(k, _)| k == "TMPDIR").count(), 1);
    assert!(
        !sandbox
            .writable_roots
            .iter()
            .any(|root| root.starts_with("/tmp/p/.git"))
    );
    assert!(
        !worker
            .env
            .iter()
            .any(|(key, _)| key.starts_with("GIT_OBJECT_DIRECTORY")
                || key == "GIT_ALTERNATE_OBJECT_DIRECTORIES")
    );
    // Fix round 5: the worker's git writes no reflog.
    assert!(worker.env.contains(&(
        "GIT_CONFIG_PARAMETERS".to_string(),
        "'core.logAllRefUpdates'='false'".to_string()
    )));
    // A profile's own entries are kept, ours after them.
    let env = with_worker_git_config(vec![(
        "GIT_CONFIG_PARAMETERS".to_string(),
        "'a.b'='c'".to_string(),
    )]);
    assert_eq!(
        env,
        [(
            "GIT_CONFIG_PARAMETERS".to_string(),
            "'a.b'='c' 'core.logAllRefUpdates'='false'".to_string()
        )]
    );
    assert_eq!(
        worker.claude_permission_mode.as_deref(),
        Some("acceptEdits")
    );
}

/// Final fix batch F2 (C-I1): every Codex session (worker and reviewer) carries the
/// base's `.codex` as its guard, unless the run's Codex CLI does not load project
/// config or is told not to; Claude sessions never do.
#[test]
fn codex_sessions_carry_the_base_codex_config_guard() {
    use crate::headless::argv::CodexProjectConfig;
    use crate::headless::codex_guard::{EntryKind, GuardEntry, ObjectFormat};
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    let entry = GuardEntry {
        path: ".codex/config.toml".into(),
        kind: EntryKind::File,
        oid: "a".repeat(40),
    };
    run.codex_config_base = vec![entry.clone()];
    let route = |runtime| Route {
        runtime,
        model: "m".into(),
        strength: Strength::Standard,
        effort: Effort::MEDIUM,
    };
    let mut task = run.tasks[0].clone();
    for branch in [None, Some(CodexProjectConfig::Loaded)] {
        run.codex_project_config = branch;
        task.route = route(Runtime::Codex);
        let guard = worker_spec(&run, &task)
            .codex_config_guard
            .expect("guarded");
        assert_eq!(guard.format, ObjectFormat::Sha1);
        assert_eq!(guard.entries, std::slice::from_ref(&entry));
        let review = reviewer_spec(&run, &task, &route(Runtime::Codex));
        assert_eq!(review.codex_config_guard, Some(guard));
        task.route = route(Runtime::Claude);
        assert_eq!(worker_spec(&run, &task).codex_config_guard, None);
        let review = reviewer_spec(&run, &task, &route(Runtime::Claude));
        assert_eq!(review.codex_config_guard, None);
    }
    task.route = route(Runtime::Codex);
    for branch in [CodexProjectConfig::NotLoaded, CodexProjectConfig::Excluded] {
        run.codex_project_config = Some(branch);
        assert_eq!(worker_spec(&run, &task).codex_config_guard, None);
    }
}

/// F2 round 2 (item 2): a Claude worker's sandbox denies writes to the protected
/// agent-config paths of its checkout, minus those its `owns` names exactly; a Claude
/// reviewer's denies them all. F4 (the F2 re-review's I2): no any-depth entry, since
/// Claude's globs cannot leave `node_modules` and the like out; (M1) a literal
/// `.claude` in `owns` names no file under it, so it keeps the directory denied.
#[test]
fn claude_sessions_deny_writes_to_protected_agent_config() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    let task = run.tasks[0].clone();
    let at = |rel: &str| task.worktree.join(rel);
    let all = [
        at(".claude"),
        at(".codex"),
        at(".mcp.json"),
        at("CLAUDE.md"),
        at("AGENTS.md"),
    ];
    let denied = |run: &Run, task: &Task| worker_spec(run, task).claude_sandbox.unwrap().deny_write;
    assert_eq!(denied(&run, &task), all);

    let mut owner = task.clone();
    owner.spec.owns = vec![
        "./.claude/settings.json".into(),
        "docs/AGENTS.md".into(),
        "CLAUDE.md".into(),
        ".codex/**".into(),
    ];
    assert_eq!(
        denied(&run, &owner),
        // A glob in `owns` (`.codex/**`) never counts (decision 56).
        [at(".codex"), at(".mcp.json"), at("AGENTS.md")]
    );
    owner.spec.owns = vec![".claude".into(), ".codex/".into()];
    assert_eq!(denied(&run, &owner), all);

    let route = Route {
        runtime: Runtime::Claude,
        model: "m".into(),
        strength: Strength::Standard,
        effort: Effort::MEDIUM,
    };
    let review = reviewer_spec(&run, &owner, &route).claude_sandbox.unwrap();
    let path = run.review_path(owner.id());
    assert_eq!(review.deny_write.len(), 5);
    assert!(review.deny_write.iter().all(|p| p.starts_with(&path)));
    run.limits.worker_sandbox = false;
    assert!(worker_spec(&run, &task).claude_sandbox.is_none());
}
