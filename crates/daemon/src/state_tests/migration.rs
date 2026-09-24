//! The state file's two worktree fields, and reading a file written before they were
//! separated.
//!
//! Milestone 6 saved one key, `worktree`, holding `Entry.managed` — the checkout anthrex
//! itself created. The git worktree *root* a window sits in (`Entry.worktree`, milestone
//! 4.5's field, and what the git registry watches) was never saved at all, so a window
//! that merely stood inside an existing checkout came back from a restart with no root.
//! They are now two keys: `worktree` is the root, `managed` is the created checkout.
//!
//! That renames an existing key's meaning and changes its type, so `STATE_VERSION` went
//! to 3 and `load` migrates a version-2 file on the way in.

use super::*;

/// A real `state.json`, captured verbatim from the milestone-6 build (`748618b`, before
/// the split) by running the daemon with an isolated `ANTHREX_SOCKET`/`ANTHREX_DATA_DIR`
/// and creating two shell windows in a temporary repository: one plain, one with
/// `--worktree feature-x`. Nothing here is hand-written, which is the point — a fixture
/// typed out to match what the loader expects tests the expectation, not the format.
///
/// Note what the shipped build recorded for window 1: `"worktree": null`, even though
/// its `cwd` is a git worktree root. That null is the defect this change closes.
const SHIPPED_V2_STATE: &str = r#"{
  "version": 2,
  "next_id": 3,
  "windows": [
    {
      "id": 1,
      "name": "plain-window",
      "runtime": "shell",
      "cwd": "/private/tmp/anx6fx/repo",
      "project": "/private/tmp/anx6fx/repo",
      "worktree": null,
      "model": null,
      "initial_prompt": null,
      "session_id": null,
      "created_at": 1790073102,
      "status": "exited",
      "run": null
    },
    {
      "id": 2,
      "name": "managed-window",
      "runtime": "shell",
      "cwd": "/private/tmp/anx6fx/data/worktrees/repo-a2d61530/feature-x",
      "project": "/private/tmp/anx6fx/repo",
      "worktree": {
        "repo_root": "/private/tmp/anx6fx/repo",
        "path": "/private/tmp/anx6fx/data/worktrees/repo-a2d61530/feature-x",
        "branch": "feature-x"
      },
      "model": null,
      "initial_prompt": null,
      "session_id": null,
      "created_at": 1790073102,
      "status": "exited",
      "run": null
    }
  ],
  "runs": []
}"#;

#[test]
fn a_state_file_from_the_shipped_build_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(&path, SHIPPED_V2_STATE).unwrap();

    let (state, problems) = load(&path);

    assert!(
        problems.is_empty(),
        "a file the previous build wrote must load without complaint: {problems:?}"
    );
    assert_eq!(state.next_id, 3);
    assert_eq!(state.windows.len(), 2, "neither record may be skipped");

    let plain = &state.windows[0];
    assert_eq!(plain.name, "plain-window");
    assert_eq!(plain.managed, None);
    assert_eq!(
        plain.worktree, None,
        "the old format recorded no root for a plain window; nothing may be invented here"
    );

    let managed = &state.windows[1];
    assert_eq!(
        managed.managed,
        Some(WorktreeRecord {
            repo_root: PathBuf::from("/private/tmp/anx6fx/repo"),
            path: PathBuf::from("/private/tmp/anx6fx/data/worktrees/repo-a2d61530/feature-x"),
            branch: "feature-x".into(),
        }),
        "the old object must land in `managed`, field for field"
    );
    assert_eq!(
        managed.worktree,
        Some(PathBuf::from(
            "/private/tmp/anx6fx/data/worktrees/repo-a2d61530/feature-x"
        )),
        "and the root is the created checkout's own path, so the window stays watched"
    );
}

/// The migrated file is written back in the new shape, and reading *that* gives the same
/// records — so one save after an upgrade is not a second, lossy migration.
#[test]
fn a_migrated_file_saves_and_reloads_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(&path, SHIPPED_V2_STATE).unwrap();
    let (migrated, _) = load(&path);

    save(&path, &migrated).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("\"version\": 3"),
        "the save must claim the new version: {written}"
    );
    assert!(
        written.contains("\"managed\": {"),
        "the created checkout belongs under its own key now: {written}"
    );

    let (reloaded, problems) = load(&path);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(reloaded.windows, migrated.windows);
}

/// The wire pin for the two fields, with a deliberately different value in each: a
/// record whose `worktree` and `managed.path` were transposed — the one mistake this
/// split makes possible — cannot pass this.
///
/// Production keeps the two equal when both are set (a managed window sits in the
/// checkout anthrex made for it), which is exactly why they must not be equal here:
/// equal values would make a transposition invisible.
#[test]
fn the_two_worktree_fields_are_independent_on_the_wire() {
    let record = WindowRecord {
        id: 7,
        name: "split".into(),
        runtime: Runtime::Codex,
        cwd: PathBuf::from("/cwd/of/the/window"),
        project: Some(PathBuf::from("/the/project/root")),
        worktree: Some(PathBuf::from("/the/watched/worktree/root")),
        managed: Some(WorktreeRecord {
            repo_root: PathBuf::from("/the/managed/repo/root"),
            path: PathBuf::from("/the/managed/checkout/path"),
            branch: "the-managed-branch".into(),
        }),
        model: Some("a-model".into()),
        initial_prompt: Some("an initial prompt".into()),
        session_id: Some("a-session-id".into()),
        created_at: 1_790_073_102,
        status: Status::Idle,
        run: None,
        kind: Default::default(),
    };

    let json = serde_json::to_string(&record).unwrap();
    let expected = "{\"id\":7,\"name\":\"split\",\"runtime\":\"codex\",\"cwd\":\"/cwd/of/the/window\",\
\"project\":\"/the/project/root\",\"worktree\":\"/the/watched/worktree/root\",\
\"managed\":{\"repo_root\":\"/the/managed/repo/root\",\"path\":\"/the/managed/checkout/path\",\
\"branch\":\"the-managed-branch\"},\"model\":\"a-model\",\
\"initial_prompt\":\"an initial prompt\",\"session_id\":\"a-session-id\",\
\"created_at\":1790073102,\"status\":\"idle\",\"run\":null,\"kind\":\"pty\"}";
    assert_eq!(json, expected);

    let back: WindowRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back, record, "each key must read back into its own field");
}

/// A version-1 file (core spec 3.6) carries a `worktree` *string*, which is the root —
/// the meaning the new key has. It loads as one, with no `managed`, exactly as decision
/// 16 of the milestone-6 brief says: "restores watchable but not worktree-removable".
#[test]
fn a_version_one_worktree_string_is_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(
        &path,
        br#"{"version": 1, "next_id": 2, "windows": [
            {"id": 1, "name": "old", "runtime": "shell", "cwd": "/repo",
             "worktree": "/repo", "status": "idle"}
        ]}"#,
    )
    .unwrap();

    let (state, problems) = load(&path);

    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(state.windows[0].worktree, Some(PathBuf::from("/repo")));
    assert_eq!(state.windows[0].managed, None);
}

/// A `worktree` object missing `path` cannot be migrated into a root. The record is not
/// thrown away over it: the rest of the window is intact and losing it would be the
/// worse outcome, so it loads with neither worktree field and one warning.
#[test]
fn an_unmigratable_worktree_object_costs_the_field_not_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(
        &path,
        br#"{"version": 2, "next_id": 2, "windows": [
            {"id": 1, "name": "half", "runtime": "shell", "cwd": "/repo",
             "worktree": {"repo_root": "/repo", "branch": "b"}, "status": "idle"}
        ]}"#,
    )
    .unwrap();

    let (state, problems) = load(&path);

    assert_eq!(state.windows.len(), 1, "the window must survive");
    assert_eq!(state.windows[0].name, "half");
    assert_eq!(state.windows[0].worktree, None);
    assert_eq!(state.windows[0].managed, None);
    assert_eq!(problems.len(), 1, "and say so once: {problems:?}");
    assert_eq!(problems[0].severity, Severity::Warn);
}
