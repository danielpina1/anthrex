//! Milestone 9.16 decision 26: `session_files` copies a window's root session out. A
//! real manager and a real PTY window whose `claude` is a stand-in that sleeps; the test
//! kills only the window it made.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proto::{HookSource, Runtime, WindowSpec};
use serde_json::json;

use super::SessionFiles;
use crate::manager::role_window::tests::{manager, role};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_files_copies_out_without_io() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-session-files-")
        .tempdir_in("/tmp")
        .unwrap();
    let manager = manager(dir.path());
    let spec = WindowSpec {
        name: Some("3f9a/orchestrator".into()),
        runtime: Runtime::Claude,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let id = manager
        .create_run_window(spec, dir.path().to_path_buf(), role("first-3f9a"))
        .await
        .expect("the stand-in's window")
        .id;
    let blank = manager.session_files(id);
    // A path that does not exist: nothing is read.
    let transcript = dir.path().join("nowhere").join("session.jsonl");
    let start = json!({
        "hook_event_name": "SessionStart",
        "session_id": "sess-a",
        "transcript_path": transcript,
        "source": "startup",
    });
    manager.handle_hook(id, HookSource::Claude, &start).unwrap();
    let files = manager.session_files(id);
    let unknown = manager.session_files(id + 1000);
    let _ = manager.kill(id);

    let blank = blank.expect("a window that is there");
    assert_eq!((blank.transcript_path, blank.session_id), (None, None));
    let files = files.expect("a window that is there");
    let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap() + Duration::from_secs(1);
    assert!(
        (before..=after.as_secs()).contains(&files.started_at),
        "{}",
        files.started_at
    );
    assert_eq!(
        files,
        SessionFiles {
            runtime: Runtime::Claude,
            transcript_path: Some(PathBuf::from(&transcript)),
            session_id: Some("sess-a".into()),
            started_at: files.started_at,
        }
    );
    assert_eq!(unknown, None);
}
