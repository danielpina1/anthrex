use super::*;

fn state_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("state.json")
}

/// Two records with deliberately mismatched field *types* of content (a path here, a
/// prose string there, a uuid-shaped id elsewhere) so that a bug which transposes two
/// same-typed fields between the records — the exact blind spot a symmetric fixture
/// cannot catch — shows up as a failing assertion.
fn sample_state() -> StateFile {
    StateFile {
        version: STATE_VERSION,
        next_id: 6,
        windows: vec![
            WindowRecord {
                id: 3,
                name: "api-worker".into(),
                runtime: Runtime::Claude,
                cwd: PathBuf::from("/Users/me/repos/shop"),
                project: Some(PathBuf::from("/Users/me/repos/shop")),
                worktree: Some(WorktreeRecord {
                    repo_root: PathBuf::from("/Users/me/repos/shop"),
                    path: PathBuf::from("/Users/me/repos/shop-worktrees/api-worker"),
                    branch: "feature/api-worker".into(),
                }),
                model: Some("opus".into()),
                initial_prompt: Some("fix the failing tests".into()),
                session_id: Some("5f0c2d1e-8a8b-4c1e-9d55-2b7e9f1a0c11".into()),
                created_at: 1_789_123_456,
                status: Status::Working,
                run: None,
            },
            WindowRecord {
                id: 5,
                name: "scratch".into(),
                runtime: Runtime::Shell,
                cwd: PathBuf::from("/tmp/scratch"),
                project: None,
                worktree: None,
                model: None,
                initial_prompt: None,
                session_id: None,
                created_at: 42,
                status: Status::Idle,
                run: None,
            },
        ],
        runs: Vec::new(),
    }
}

#[test]
fn save_then_load_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let state = sample_state();

    save(&path, &state).unwrap();
    let (loaded, warnings) = load(&path);

    assert_eq!(loaded, state);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

#[test]
fn save_is_atomic_and_private() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let tmp_path = dir.path().join("state.json.tmp");

    // A leftover garbage temp file from a previous crashed run must not make this save
    // fail; it gets replaced, not treated as a conflict.
    std::fs::write(&tmp_path, b"garbage from a previous crash").unwrap();

    save(&path, &sample_state()).unwrap();

    assert!(!tmp_path.exists(), "the temp file must not survive save()");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "state.json must be mode 0o600, got {mode:o}");
    }

    let (loaded, warnings) = load(&path);
    assert_eq!(loaded, sample_state());
    assert!(warnings.is_empty());
}

#[test]
fn missing_file_loads_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);

    let (state, warnings) = load(&path);

    assert!(state.windows.is_empty());
    assert_eq!(state.next_id, 1);
    assert!(warnings.is_empty());
}

/// Decision 12's asymmetry, pinned down: an unreadable file (here, a directory sitting
/// where the state file should be, which is a read error on every platform regardless of
/// the user running the test) is left exactly where it is — no corrupt-* sibling appears
/// — while a corrupt one gets renamed. Simulated with a directory rather than a
/// permission bit so the test does not depend on not running as root.
#[test]
fn unreadable_file_is_left_alone_and_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::create_dir(&path).unwrap();

    let (state, warnings) = load(&path);

    assert!(state.windows.is_empty());
    assert_eq!(state.next_id, 1);
    assert!(path.is_dir(), "an unreadable file must be left alone");
    let siblings: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        siblings,
        vec![std::ffi::OsString::from("state.json")],
        "nothing must be renamed aside for an unreadable file"
    );
    assert!(!warnings.is_empty(), "the caller needs something to log");
    assert_eq!(
        warnings[0].severity,
        Severity::Error,
        "an unreadable file is a real data-loss event, not routine recovery: {warnings:?}"
    );
}

#[test]
fn corrupt_file_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(&path, b"{not json").unwrap();

    let (state, warnings) = load(&path);

    assert!(state.windows.is_empty());
    assert_eq!(state.next_id, 1);
    assert_eq!(warnings.len(), 1);
    assert!(
        !path.exists(),
        "the corrupt file must not be left at state.json"
    );

    let corrupt_files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("state.json.corrupt-"))
        .collect();
    assert_eq!(
        corrupt_files.len(),
        1,
        "exactly one corrupt-* file: {corrupt_files:?}"
    );
    let first_name = &corrupt_files[0];
    assert!(
        warnings[0].message.contains(first_name.as_str()),
        "warning {:?} must name the new path {first_name:?}",
        warnings[0]
    );
    assert_eq!(
        warnings[0].severity,
        Severity::Warn,
        "the module already recovered by moving the file aside: {warnings:?}"
    );
    // No "-1", "-2", ... suffix yet: this is the first corrupt file at this timestamp.
    let suffix = first_name.strip_prefix("state.json.corrupt-").unwrap();
    assert!(
        suffix.chars().all(|c| c.is_ascii_digit()),
        "first corrupt file must be a bare timestamp, got {first_name:?}"
    );
    let original = std::fs::read(dir.path().join(first_name)).unwrap();
    assert_eq!(original, b"{not json");

    // A second corrupt load, in the same second, must not collide with the first: it
    // gets a "-1" suffix instead of silently overwriting or erroring.
    std::fs::write(&path, b"{not json").unwrap();
    let (_state2, warnings2) = load(&path);
    assert_eq!(warnings2.len(), 1);

    let corrupt_files_after: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("state.json.corrupt-"))
        .collect();
    assert_eq!(corrupt_files_after.len(), 2, "{corrupt_files_after:?}");
    let second_name = corrupt_files_after
        .iter()
        .find(|name| *name != first_name)
        .expect("a second corrupt-* file must have appeared");
    assert!(
        second_name.ends_with("-1"),
        "a second corrupt load in the same second must produce a name ending in -1, got {second_name:?}"
    );
}

#[test]
fn newer_version_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(&path, br#"{"version": 3, "next_id": 1, "windows": []}"#).unwrap();

    let (state, warnings) = load(&path);

    assert!(state.windows.is_empty());
    assert_eq!(state.next_id, 1);
    assert!(!path.exists());
    assert_eq!(warnings.len(), 1);

    let unsupported_files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("state.json.unsupported-"))
        .collect();
    assert_eq!(unsupported_files.len(), 1, "{unsupported_files:?}");
    assert!(warnings[0].message.contains(unsupported_files[0].as_str()));
    assert_eq!(
        warnings[0].severity,
        Severity::Warn,
        "the module already recovered by moving the file aside: {warnings:?}"
    );
}

#[test]
fn version_1_file_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 1,
        "next_id": 3,
        "windows": [
            {
                "id": 1,
                "name": "first",
                "runtime": "claude",
                "cwd": "/tmp/first",
                "worktree": null,
                "model": "opus",
                "initial_prompt": "do the thing",
                "created_at": 100,
                "status": "working"
            },
            {
                "id": 2,
                "name": "second",
                "runtime": "codex",
                "cwd": "/tmp/second",
                "worktree": null,
                "model": null,
                "initial_prompt": null,
                "session_id": "sess-2",
                "created_at": 200,
                "status": "sleeping"
            }
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, warnings) = load(&path);

    assert!(
        warnings.is_empty(),
        "a valid version-1 file must load cleanly: {warnings:?}"
    );
    assert_eq!(state.windows.len(), 2);

    let first = &state.windows[0];
    assert_eq!(first.project, None);
    assert_eq!(first.run, None);
    assert_eq!(
        first.session_id, None,
        "a missing session_id must load as None"
    );
    assert_eq!(first.status, Status::Working);

    let second = &state.windows[1];
    assert_eq!(second.project, None);
    assert_eq!(second.run, None);
    assert_eq!(
        second.status,
        Status::Exited,
        "an unknown status string must be read leniently as Exited"
    );
}

/// Four records, not three: a valid one, one with an unknown runtime, one that repeats
/// the first record's name, and a *second* valid record positioned after both bad ones.
/// Two survivors, two warnings.
///
/// The fourth record is the point: decision 12 rejects records one at a time rather than
/// failing the whole file specifically so a rejected record cannot poison the ones that
/// follow it. A three-record version where the only bad records are last cannot tell the
/// difference between "skips bad records" and "stops processing at the first bad record" —
/// both would leave exactly one record, id 1. Only a good record positioned *after* the
/// bad ones can distinguish them, which is why this test needs four records, not three.
#[test]
fn bad_records_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 2,
        "next_id": 1,
        "windows": [
            {
                "id": 1,
                "name": "good",
                "runtime": "claude",
                "cwd": "/tmp/good"
            },
            {
                "id": 2,
                "name": "unknown-runtime",
                "runtime": "perl",
                "cwd": "/tmp/perl"
            },
            {
                "id": 3,
                "name": "good",
                "runtime": "shell",
                "cwd": "/tmp/dup"
            },
            {
                "id": 4,
                "name": "also-good",
                "runtime": "codex",
                "cwd": "/tmp/also-good"
            }
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, warnings) = load(&path);

    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings.iter().all(|p| p.severity == Severity::Warn),
        "a skipped record is routine recovery, not an error: {warnings:?}"
    );
    assert_eq!(
        state.windows.len(),
        2,
        "both bad records are skipped, both good ones survive"
    );
    assert_eq!(state.windows[0].id, 1);
    assert_eq!(state.windows[0].name, "good");
    assert_eq!(
        state.windows[1].id, 4,
        "a good record after two bad ones must still load — a rejected record must not \
         poison the records that follow it"
    );
    assert_eq!(state.windows[1].name, "also-good");
}

#[test]
fn next_id_is_never_below_a_loaded_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 2,
        "next_id": 2,
        "windows": [
            {
                "id": 7,
                "name": "seven",
                "runtime": "claude",
                "cwd": "/tmp/seven"
            }
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, _warnings) = load(&path);

    assert_eq!(state.next_id, 8);
}

#[test]
fn record_json_shape() {
    let record = WindowRecord {
        id: 3,
        name: "api-worker".into(),
        runtime: Runtime::Claude,
        cwd: PathBuf::from("/Users/me/repos/shop"),
        project: Some(PathBuf::from("/Users/me/repos/shop")),
        worktree: None,
        model: Some("opus".into()),
        initial_prompt: Some("fix the failing tests".into()),
        session_id: Some("5f0c2d1e-8a8b-4c1e-9d55-2b7e9f1a0c11".into()),
        created_at: 1_789_123_456,
        status: Status::Working,
        run: None,
    };

    let json = serde_json::to_string(&record).unwrap();
    let expected = "{\"id\":3,\"name\":\"api-worker\",\"runtime\":\"claude\",\"cwd\":\"/Users/me/repos/shop\",\
\"project\":\"/Users/me/repos/shop\",\"worktree\":null,\"model\":\"opus\",\
\"initial_prompt\":\"fix the failing tests\",\
\"session_id\":\"5f0c2d1e-8a8b-4c1e-9d55-2b7e9f1a0c11\",\"created_at\":1789123456,\
\"status\":\"working\",\"run\":null}";
    assert_eq!(json, expected);
}
