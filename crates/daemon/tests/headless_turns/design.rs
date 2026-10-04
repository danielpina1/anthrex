//! Milestone 9.6 rulings T8-4 and T8-7: a design agent's session is never saved, so it
//! is never resumed. A Claude brainstormer's nudge goes on its live process's stdin and
//! no argv carries `--resume`; a resume of it, and a Codex brainstormer's next turn
//! (`codex exec resume`), are refused before any process starts.

use super::support::headless::*;
use super::{
    CLAUDE_UUID, CODEX_THREAD, Cleanup, argvs, claude_recorder, codex_recorder, is_exit,
    is_turn_end, stdin_lines,
};
use daemon::headless::{HeadlessSpec, McpTarget};
use proto::{AgentRole, Runtime};
use std::path::Path;
use std::time::Duration;

/// A brainstormer's spec on `runtime`.
fn brainstormer(runtime: Runtime, dir: &Path) -> HeadlessSpec {
    let mut spec = spec(runtime, dir);
    spec.mcp = Some(McpTarget {
        role: AgentRole::Brainstormer,
        run_id: "r-3f9a".into(),
        task_id: None,
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
        agent_label: Some("claude".into()),
    });
    spec
}

#[tokio::test]
async fn a_claude_design_agents_nudge_goes_on_stdin_and_it_is_never_resumed() {
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), false);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let spec = brainstormer(Runtime::Claude, dir.path());
    let info = create(&m, "b", spec, "first").await;
    next_signal(&mut feed, "the first turn's end", is_turn_end).await;
    m.headless_send(info.id, "the nudge").await.unwrap();
    next_signal(&mut feed, "the nudge's turn end", is_turn_end).await;
    let lines = stdin_lines(dir.path());
    assert!(lines.iter().any(|l| l.contains("the nudge")), "{lines:?}");
    let error = m
        .headless_resume(info.id, CLAUDE_UUID, "go on", Duration::ZERO)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("never resumed"), "{error}");
    let all = argvs(dir.path());
    assert_eq!(all.len(), 1, "one process, its nudge on stdin: {all:?}");
    assert!(all[0].contains(&"--no-session-persistence".to_string()));
    assert!(!all[0].contains(&"--resume".to_string()), "{:?}", all[0]);
}

#[tokio::test]
async fn a_codex_design_agents_next_turn_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let spec = brainstormer(Runtime::Codex, dir.path());
    let info = create(&m, "b", spec, "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;
    let error = m
        .headless_send(info.id, "second")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("never resumed"), "{error}");
    let error = (m
        .headless_resume(info.id, CODEX_THREAD, "again", Duration::ZERO)
        .await)
        .unwrap_err()
        .to_string();
    assert!(error.contains("never resumed"), "{error}");
    let all = argvs(dir.path());
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(all[0].contains(&"--ephemeral".to_string()), "{:?}", all[0]);
    assert!(!all[0].contains(&"resume".to_string()), "{:?}", all[0]);
}

/// Milestone 9.6 task M9.6.10 (task 8's test gap): a Codex document reviewer, like a
/// Codex brainstormer, runs one `--ephemeral` process and is never resumed.
#[tokio::test]
async fn a_codex_document_reviewer_is_ephemeral_and_never_resumed() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let mut spec = brainstormer(Runtime::Codex, dir.path());
    let mcp = spec.mcp.as_mut().unwrap();
    (mcp.role, mcp.agent_label) = (AgentRole::DocReviewer, Some("spec-r1".into()));
    let info = create(&m, "r", spec, "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;
    let error = m
        .headless_send(info.id, "second")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("never resumed"), "{error}");
    let error = (m
        .headless_resume(info.id, CODEX_THREAD, "again", Duration::ZERO)
        .await)
        .unwrap_err()
        .to_string();
    assert!(error.contains("never resumed"), "{error}");
    let all = argvs(dir.path());
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(all[0].contains(&"--ephemeral".to_string()), "{:?}", all[0]);
    let mcp = all[0]
        .iter()
        .any(|a| a.contains(r#""doc_reviewer","--run","r-3f9a","--agent-label","spec-r1""#));
    assert!(mcp, "{:?}", all[0]);
    assert!(!all[0].contains(&"resume".to_string()), "{:?}", all[0]);
}
