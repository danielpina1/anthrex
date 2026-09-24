//! The launch specs of run sessions (decisions 24–26, 50, 54) and their session ids
//! (decision 24). Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

use std::path::{Path, PathBuf};

use proto::{AgentRole, Route, RunRef, Runtime};

use super::contract::{REVIEWER_CONTRACT, WORKER_CONTRACT};
use super::env::profile_env;
use super::model::{OpId, Run, Task};
use crate::headless::{ClaudeSandbox, HeadlessSpec, McpTarget};

/// Every worker's first two allowed tools; `worker_allowed_tools` follows (decision 24).
pub const WORKER_MCP_TOOLS: [&str; 2] = ["mcp__anthrex__task_done", "mcp__anthrex__task_blocked"];

/// A reviewer's allowed tools (decision 24, ruling Q4): its verdict, reading, and three
/// read-only git commands.
pub const REVIEWER_TOOLS: [&str; 7] = [
    "mcp__anthrex__submit_review",
    "Read",
    "Glob",
    "Grep",
    "Bash(git diff:*)",
    "Bash(git log:*)",
    "Bash(git show:*)",
];

/// A Claude reviewer's permission mode and denied tools (M8a.1: plan mode blocks the
/// reviewer's MCP call in `-p`, so `dontAsk` with the write tools disallowed).
pub const REVIEWER_PERMISSION_MODE: &str = "dontAsk";
pub const REVIEWER_DISALLOWED_TOOLS: [&str; 3] = ["Edit", "Write", "NotebookEdit"];

/// A reviewer's Codex sandbox, always (decision 25).
pub const REVIEWER_CODEX_SANDBOX: &str = "read-only";

/// The parts of the repository's git common directory a worker's sandbox may write
/// (decisions 25 and 54, narrowed by final fix batch F1, findings C-C1 and D-5, and its
/// fix round 1, N3): the object store, and the task's own branch
/// `refs/heads/anthrex/<run>/<task>`, its `.lock` and its reflog. Not `config`,
/// `hooks/`, `info/`, `packed-refs` or any other branch, the base branch, the run branch
/// and sibling tasks' branches included. The driver adds the files of the worktree's own
/// git directory a commit needs ([`crate::run::git::worker_git_dirs`]) at launch.
pub fn worker_git_roots(git_common_dir: &Path, run_id: &str, task_id: &str) -> Vec<PathBuf> {
    let branch = Path::new("refs/heads/anthrex").join(run_id).join(task_id);
    let mut lock = branch.clone().into_os_string();
    lock.push(".lock");
    vec![
        git_common_dir.join("objects"),
        git_common_dir.join(&branch),
        git_common_dir.join(lock),
        git_common_dir.join("logs").join(&branch),
    ]
}

/// The worker session of `task`'s current session number, in its worktree.
pub fn worker_spec(run: &Run, task: &Task) -> HeadlessSpec {
    let route = &task.route;
    let limits = &run.limits;
    let claude = route.runtime == Runtime::Claude;
    let mut allowed: Vec<String> = WORKER_MCP_TOOLS.iter().map(|s| s.to_string()).collect();
    allowed.extend(limits.worker_allowed_tools.iter().cloned());
    HeadlessSpec {
        runtime: route.runtime,
        model: route.model.clone(),
        effort: route.effort,
        cwd: task.worktree.clone(),
        instructions: WORKER_CONTRACT.to_string(),
        mcp: Some(McpTarget {
            role: AgentRole::Worker,
            run_id: run.id.clone(),
            task_id: Some(task.spec.id.clone()),
        }),
        allowed_tools: allowed,
        claude_permission_mode: claude.then(|| limits.worker_permission_mode.clone()),
        claude_disallowed_tools: Vec::new(),
        claude_sandbox: (claude && limits.worker_sandbox).then(|| ClaudeSandbox {
            writable_roots: worker_git_roots(&run.git_common_dir, &run.id, task.id()),
        }),
        codex_sandbox: limits.worker_codex_sandbox.clone(),
        codex_writable_roots: if claude {
            Vec::new()
        } else {
            worker_git_roots(&run.git_common_dir, &run.id, task.id())
        },
        env: profile_env(&run.profile, &task.worktree),
        claude_auth: limits.claude_auth.into(),
        api_key_helper: limits.api_key_helper.clone(),
        run_ref: Some(RunRef {
            run_id: run.id.clone(),
            task_id: Some(task.spec.id.clone()),
            role: AgentRole::Worker,
            session: task.session,
        }),
    }
}

/// A fresh reviewer session on `route`, read-only, in the task's review worktree. Its
/// `RunRef.session` is the review round.
pub fn reviewer_spec(run: &Run, task: &Task, route: &Route) -> HeadlessSpec {
    let claude = route.runtime == Runtime::Claude;
    let path = run.review_path(task.id());
    let round = task
        .rounds
        .iter()
        .filter(|r| r.role == AgentRole::Reviewer)
        .count() as u32
        + 1;
    HeadlessSpec {
        runtime: route.runtime,
        model: route.model.clone(),
        effort: route.effort,
        cwd: path.clone(),
        instructions: REVIEWER_CONTRACT.to_string(),
        mcp: Some(McpTarget {
            role: AgentRole::Reviewer,
            run_id: run.id.clone(),
            task_id: Some(task.spec.id.clone()),
        }),
        allowed_tools: REVIEWER_TOOLS.iter().map(|s| s.to_string()).collect(),
        claude_permission_mode: claude.then(|| REVIEWER_PERMISSION_MODE.to_string()),
        claude_disallowed_tools: if claude {
            REVIEWER_DISALLOWED_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        },
        claude_sandbox: None,
        codex_sandbox: REVIEWER_CODEX_SANDBOX.to_string(),
        codex_writable_roots: Vec::new(),
        env: profile_env(&run.profile, &path),
        claude_auth: run.limits.claude_auth.into(),
        api_key_helper: run.limits.api_key_helper.clone(),
        run_ref: Some(RunRef {
            run_id: run.id.clone(),
            task_id: Some(task.spec.id.clone()),
            role: AgentRole::Reviewer,
            session: round,
        }),
    }
}

/// 64-bit FNV-1a over `parts`, each followed by a 0xff separator (a byte no UTF-8
/// string contains), so `("ab", "c")` and `("a", "bc")` differ.
pub fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.iter().chain(std::iter::once(&0xff)) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// Decision 24's `--session-id`: a version-4-shaped UUID built from two FNV-1a hashes of
/// the run id and the `CreateWindow` op id. Deterministic, so the reducer stays pure; a
/// re-issued op has a new id, so it never collides with a half-created session.
pub fn session_uuid(run_id: &str, op: OpId) -> String {
    let op = op.to_le_bytes();
    let high = fnv1a(&[b"anthrex-session", run_id.as_bytes(), &op]);
    let low = fnv1a(&[&op, run_id.as_bytes(), b"anthrex-session"]);
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&high.to_be_bytes());
    bytes[8..].copy_from_slice(&low.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// [`session_uuid`] for `run`'s op `op`, with the run's random `session_nonce` mixed
/// into the run id (M8a.21's carry): two daemons that drew the same run id never share a
/// session. A nonce of 0 (a run from before M8a.22) gives `session_uuid(run.id, op)`.
pub fn session_uuid_of(run: &Run, op: OpId) -> String {
    if run.session_nonce == 0 {
        return session_uuid(&run.id, op);
    }
    session_uuid(&format!("{}#{:016x}", run.id, run.session_nonce), op)
}

/// Decision 18's launch jitter: `100 + fnv1a(run, task, session) % 400` milliseconds.
pub fn jitter_ms(run_id: &str, task_id: &str, session: u32) -> u64 {
    100 + fnv1a(&[
        run_id.as_bytes(),
        task_id.as_bytes(),
        &session.to_le_bytes(),
    ]) % 400
}

#[cfg(test)]
mod tests {
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
                effort: Effort::Medium,
            };
            let spec = reviewer_spec(&run, task, &route);
            assert_eq!(spec.cwd, review);
            assert_eq!(spec.runtime, runtime);
            assert_eq!(spec.effort, Effort::Medium);
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
            assert_eq!(spec.claude_sandbox, None);
            assert_eq!(spec.codex_sandbox, "read-only");
            assert!(spec.codex_writable_roots.is_empty());
            if runtime == Runtime::Claude {
                assert_eq!(spec.claude_permission_mode.as_deref(), Some("dontAsk"));
                assert_eq!(
                    spec.claude_disallowed_tools,
                    ["Edit", "Write", "NotebookEdit"]
                );
            }
        }
        let worker = worker_spec(&run, task);
        let sandbox = worker.claude_sandbox.expect("workers run sandboxed");
        // Final fix batch F1: never the whole common dir, whose config and hooks the
        // daemon and the user's checkout would run.
        let branch = format!("refs/heads/anthrex/{}/t1", run.id);
        assert_eq!(
            sandbox.writable_roots,
            vec![
                PathBuf::from("/tmp/p/.git/objects"),
                PathBuf::from("/tmp/p/.git").join(&branch),
                PathBuf::from("/tmp/p/.git").join(format!("{branch}.lock")),
                PathBuf::from("/tmp/p/.git/logs").join(&branch),
            ]
        );
        assert_eq!(
            worker.claude_permission_mode.as_deref(),
            Some("acceptEdits")
        );
    }
}
