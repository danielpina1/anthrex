use super::*;

fn state_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("state.json")
}

/// Every same-typed field of the first record holds its own distinct, recognisable
/// value — `cwd`, `project` and `worktree.repo_root` are all `PathBuf`s, `worktree.path`
/// is a fourth; `model` and `initial_prompt` are both `Option<String>`; `name` and
/// `session_id` are both string-shaped — so a bug that transposes any pair of them (a
/// review-confirmed blind spot when a fixture gives two same-typed fields the same
/// literal value: this project has shipped exactly that shape twice before, in the
/// config crate's `bell` fields and a umask test) shows up as a failing assertion
/// instead of silently round-tripping clean. See `sample_state_fields_are_pairwise_distinct`.
fn sample_state() -> StateFile {
    StateFile {
        version: STATE_VERSION,
        next_id: 6,
        windows: vec![
            WindowRecord {
                id: 3,
                name: "api-worker".into(),
                runtime: Runtime::Claude,
                cwd: PathBuf::from("/Users/me/repos/shop/packages/api"),
                project: Some(PathBuf::from("/Users/me/repos/shop")),
                worktree: Some(WorktreeRecord {
                    repo_root: PathBuf::from("/Users/me/repos/.bare/shop"),
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

/// Guards the fixture itself, not `load`/`save`: if a future edit ever gives two
/// same-typed fields of `sample_state()`'s first record the same literal value again
/// (the exact shape this project has shipped twice before — the config crate's `bell`
/// fields and a umask test — see Minor #1 of the M6.4 review), this fails loudly instead
/// of silently reintroducing the blind spot other tests in this file rely on
/// `sample_state()` not having.
#[test]
fn sample_state_fields_are_pairwise_distinct() {
    let state = sample_state();
    let record = &state.windows[0];
    let worktree = record.worktree.as_ref().expect("fixture sets worktree");

    let paths: Vec<(&str, &PathBuf)> = vec![
        ("cwd", &record.cwd),
        (
            "project",
            record.project.as_ref().expect("fixture sets project"),
        ),
        ("worktree.repo_root", &worktree.repo_root),
        ("worktree.path", &worktree.path),
    ];
    for i in 0..paths.len() {
        for j in (i + 1)..paths.len() {
            assert_ne!(
                paths[i].1, paths[j].1,
                "{} and {} must hold distinct values in the fixture",
                paths[i].0, paths[j].0
            );
        }
    }

    let strings: Vec<(&str, &str)> = vec![
        ("name", record.name.as_str()),
        (
            "model",
            record.model.as_deref().expect("fixture sets model"),
        ),
        (
            "initial_prompt",
            record
                .initial_prompt
                .as_deref()
                .expect("fixture sets initial_prompt"),
        ),
        (
            "session_id",
            record
                .session_id
                .as_deref()
                .expect("fixture sets session_id"),
        ),
        ("worktree.branch", worktree.branch.as_str()),
    ];
    for i in 0..strings.len() {
        for j in (i + 1)..strings.len() {
            assert_ne!(
                strings[i].1, strings[j].1,
                "{} and {} must hold distinct values in the fixture",
                strings[i].0, strings[j].0
            );
        }
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

/// A fixed instant, used wherever a test needs `load_with`'s injected clock to hold
/// still across more than one call — real wall-clock seconds keep advancing mid-test,
/// which is exactly the source of `corrupt_file_is_moved_aside`'s old flake (see its
/// history: a `cargo test` run was observed to fail when the clock ticked over between
/// two `load()` calls that were supposed to land in the same second).
fn fixed_now() -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)
}

#[test]
fn corrupt_file_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let now = fixed_now();
    std::fs::write(&path, b"{not json").unwrap();

    let (state, warnings) = load_with(&path, now);

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

    // A second corrupt load, driven through the *same* injected `now` as the first —
    // deterministically the same second, not merely likely to be — must not collide
    // with the first: it gets a "-1" suffix instead of silently overwriting or erroring.
    std::fs::write(&path, b"{not json").unwrap();
    let (_state2, warnings2) = load_with(&path, now);
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

/// A third corrupt load through the same injected `now` must land on `-2`, not collide
/// with either of the first two. The review could not test this deterministically
/// before `load_with` existed, since it would have needed three real `load()` calls to
/// land in the same wall-clock second.
#[test]
fn third_corrupt_load_in_the_same_second_gets_a_dash_2_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let now = fixed_now();

    for _ in 0..3 {
        std::fs::write(&path, b"{not json").unwrap();
        let (_state, warnings) = load_with(&path, now);
        assert_eq!(warnings.len(), 1);
    }

    let mut corrupt_files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("state.json.corrupt-"))
        .collect();
    corrupt_files.sort();
    assert_eq!(corrupt_files.len(), 3, "{corrupt_files:?}");
    assert!(
        corrupt_files.iter().any(|n| n
            .strip_prefix("state.json.corrupt-")
            .unwrap()
            .chars()
            .all(|c| c.is_ascii_digit())),
        "expected a bare-timestamp name: {corrupt_files:?}"
    );
    assert!(
        corrupt_files.iter().any(|n| n.ends_with("-1")),
        "expected a -1 name: {corrupt_files:?}"
    );
    assert!(
        corrupt_files.iter().any(|n| n.ends_with("-2")),
        "a third same-second corrupt load must produce a name ending in -2: {corrupt_files:?}"
    );
}

/// If `<path>.corrupt-<ts>-1` already exists (from some earlier, unrelated run) but the
/// bare `<path>.corrupt-<ts>` does not, `aside_path_with` must pick the free bare name
/// rather than skip straight past it hunting for `-2` — the suffix search fills gaps,
/// it doesn't just count up from whatever exists.
#[test]
fn aside_path_reuses_a_free_bare_name_even_when_dash_1_is_taken() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let now = fixed_now();
    let ts = now.duration_since(UNIX_EPOCH).unwrap().as_secs();

    // Pre-create the "-1" variant only; the bare name is still free.
    std::fs::write(dir.path().join(format!("state.json.corrupt-{ts}-1")), b"x").unwrap();

    let picked = aside_path_with(&path, "corrupt", now);

    assert_eq!(
        picked,
        PathBuf::from(format!("{}.corrupt-{ts}", path.display())),
        "the bare timestamp is free and must be reused, not skipped for -2"
    );
}

/// `windows` is `#[serde(default)]`, so a file that simply omits the key entirely (a
/// hand-written file, or a hypothetical future version that dropped it) is legitimate,
/// not corrupt: it must load silently, with no warning and no rename, exactly like any
/// other defaulted field.
#[test]
fn windows_field_missing_entirely_loads_silently() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(&path, br#"{"version": 2, "next_id": 1}"#).unwrap();

    let (state, warnings) = load(&path);

    assert!(state.windows.is_empty());
    assert_eq!(state.next_id, 1);
    assert!(
        warnings.is_empty(),
        "a missing windows field is legitimate (#[serde(default)]), not corrupt: {warnings:?}"
    );
    assert!(path.exists(), "nothing must be renamed aside");
    let siblings: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        siblings,
        vec![std::ffi::OsString::from("state.json")],
        "a missing windows field must not produce a corrupt-* sibling"
    );
}

/// Unlike a missing `windows` key, a `windows` field that is *present* but the wrong
/// JSON type (here, an object instead of an array) is structurally invalid - ruled to be
/// treated exactly like every other corrupt case: renamed aside, one warning, empty
/// state. Silently loading it as empty (the pre-fix behavior) left the caller with zero
/// signal that anything was wrong and no forensic trail.
#[test]
fn windows_field_present_but_not_an_array_is_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(
        &path,
        br#"{"version": 2, "next_id": 1, "windows": {"oops": true}}"#,
    )
    .unwrap();

    let (state, warnings) = load(&path);

    assert!(state.windows.is_empty());
    assert_eq!(state.next_id, 1);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].severity, Severity::Warn);
    assert!(
        !path.exists(),
        "a malformed windows field must not be left at state.json"
    );

    let corrupt_files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("state.json.corrupt-"))
        .collect();
    assert_eq!(
        corrupt_files.len(),
        1,
        "a malformed windows field must be moved aside like any other corrupt file: {corrupt_files:?}"
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

/// A record rejected for a duplicate *name* must not claim its id: a later record that
/// legitimately reuses that id must still load. Record 2 ("a" again) is rejected because
/// its name collides with record 1's; nothing surviving ever holds id 3, so record 3's
/// reuse of id 3 is legitimate, not a collision.
#[test]
fn duplicate_name_rejection_does_not_poison_the_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 2,
        "next_id": 1,
        "windows": [
            {"id": 1, "name": "a", "runtime": "claude", "cwd": "/tmp/a"},
            {"id": 3, "name": "a", "runtime": "shell", "cwd": "/tmp/dup"},
            {"id": 3, "name": "unique-name", "runtime": "claude", "cwd": "/tmp/reuse"}
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, warnings) = load(&path);

    assert_eq!(
        warnings.len(),
        1,
        "only the duplicate-name record should be rejected: {warnings:?}"
    );
    assert_eq!(state.windows.len(), 2, "{state:?}");
    assert_eq!(state.windows[0].id, 1);
    assert_eq!(
        state.windows[1].id, 3,
        "id 3 was never claimed by a surviving record, so the third record's reuse of it is legitimate"
    );
    assert_eq!(state.windows[1].name, "unique-name");
}

/// Symmetric case: a record rejected for a duplicate *id* must not claim its name
/// either. Record 2 is rejected because its id (1) collides with record 1's; nothing
/// surviving ever holds the name "b", so record 3's reuse of that name is legitimate.
#[test]
fn duplicate_id_rejection_does_not_poison_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 2,
        "next_id": 1,
        "windows": [
            {"id": 1, "name": "a", "runtime": "claude", "cwd": "/tmp/a"},
            {"id": 1, "name": "b", "runtime": "shell", "cwd": "/tmp/dup"},
            {"id": 2, "name": "b", "runtime": "claude", "cwd": "/tmp/reuse"}
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, warnings) = load(&path);

    assert_eq!(
        warnings.len(),
        1,
        "only the duplicate-id record should be rejected: {warnings:?}"
    );
    assert_eq!(state.windows.len(), 2, "{state:?}");
    assert_eq!(state.windows[0].id, 1);
    assert_eq!(state.windows[0].name, "a");
    assert_eq!(state.windows[1].id, 2);
    assert_eq!(
        state.windows[1].name, "b",
        "name \"b\" was never claimed by a surviving record, so record 3's reuse of it is legitimate"
    );
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

/// A loaded record already holding `u32::MAX` cannot be followed by any valid,
/// non-colliding `u32` id (`u32::MAX + 1` does not exist as a `u32`). The invariant that
/// matters here is that `next_id` is never an id a loaded record already holds *when a
/// valid choice exists*; when it does not, `next_id` saturates at `u32::MAX` instead of
/// silently wrapping to 0 (which would immediately collide with the lowest live id) -
/// the deliberate consequence being that the next window-creation attempt must see
/// `next_id` already collides with a live window and fail loudly, rather than the daemon
/// silently handing out a reused id.
#[test]
fn next_id_saturates_instead_of_wrapping_when_a_loaded_id_is_u32_max() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 2,
        "next_id": 1,
        "windows": [
            {"id": u32::MAX, "name": "maxed", "runtime": "claude", "cwd": "/tmp/maxed"}
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, _warnings) = load(&path);

    assert_eq!(
        state.next_id,
        u32::MAX,
        "next_id must saturate at u32::MAX, not wrap to 0"
    );
}

/// One below the boundary: `u32::MAX - 1` still has a valid, non-colliding successor
/// (`u32::MAX` itself), so ordinary max-plus-one logic applies with no saturation.
#[test]
fn next_id_advances_normally_at_u32_max_minus_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 2,
        "next_id": 1,
        "windows": [
            {"id": u32::MAX - 1, "name": "near-max", "runtime": "claude", "cwd": "/tmp/near-max"}
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, _warnings) = load(&path);

    assert_eq!(state.next_id, u32::MAX);
}

/// An empty window list never triggers the max-plus-one path at all, so a saved
/// `next_id` of `u32::MAX` (or anything else) passes through untouched.
#[test]
fn next_id_with_empty_windows_preserves_the_saved_value_at_the_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 2,
        "next_id": u32::MAX,
        "windows": []
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, _warnings) = load(&path);

    assert_eq!(state.next_id, u32::MAX);
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
