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

/// Minor #6, `runs`'s own version of the two `windows` tests just above: `runs` is
/// `#[serde(default)]` too, so a file that omits the key entirely is legitimate and must
/// load silently, exactly like a missing `windows`.
#[test]
fn runs_field_missing_entirely_loads_silently() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(&path, br#"{"version": 2, "next_id": 1, "windows": []}"#).unwrap();

    let (state, warnings) = load(&path);

    assert!(state.runs.is_empty());
    assert!(
        warnings.is_empty(),
        "a missing runs field is legitimate (#[serde(default)]), not corrupt: {warnings:?}"
    );
    assert!(path.exists(), "nothing must be renamed aside");
}

/// The other half: `runs` present but the wrong JSON type used to be swallowed into an
/// empty list with zero warnings — exactly the silent-malformed-field defect just fixed
/// for `windows` above, reintroduced for a sibling field in the very same struct. `runs`
/// gets the identical treatment: structurally invalid, corrupt, renamed aside, one
/// warning, and (unlike the `windows` case, where good records could otherwise survive) a
/// perfectly good `windows` entry alongside it must not survive either — the whole file is
/// what gets moved aside, the same as a malformed `windows` field does.
#[test]
fn runs_field_present_but_not_an_array_is_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(
        &path,
        br#"{"version": 2, "next_id": 2, "windows": [
            {"id": 1, "name": "good", "runtime": "claude", "cwd": "/tmp/good"}
        ], "runs": {"oops": true}}"#,
    )
    .unwrap();

    let (state, warnings) = load(&path);

    assert!(
        state.windows.is_empty(),
        "the whole file is corrupt, including the otherwise-good window: {state:?}"
    );
    assert!(state.runs.is_empty());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].severity, Severity::Warn);
    assert!(
        !path.exists(),
        "a malformed runs field must not be left at state.json"
    );

    let corrupt_files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("state.json.corrupt-"))
        .collect();
    assert_eq!(
        corrupt_files.len(),
        1,
        "a malformed runs field must be moved aside like any other corrupt file: {corrupt_files:?}"
    );
}

/// A `runs` array with valid JSON content, of any shape, round-trips: `Vec<serde_json::Value>`
/// is opaque this milestone, so this is the forward-compatibility guarantee the
/// `StateFile` doc promises — a newer daemon's structured runs survive a load by this
/// build untouched.
#[test]
fn runs_field_with_array_content_loads_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    std::fs::write(
        &path,
        br#"{"version": 2, "next_id": 1, "windows": [], "runs": [{"future": "shape"}, 42, "x"]}"#,
    )
    .unwrap();

    let (state, warnings) = load(&path);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        state.runs,
        vec![
            serde_json::json!({"future": "shape"}),
            serde_json::json!(42),
            serde_json::json!("x"),
        ]
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

    let (state, warnings) = load(&path);

    assert_eq!(
        state.next_id,
        u32::MAX,
        "next_id must saturate at u32::MAX, not wrap to 0"
    );
    // Minor #3: `next_id` now equals a live window's own id — a collision the caller
    // gets no signal about at all unless this is reported.
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].severity, Severity::Warn);
    assert!(
        warnings[0].key == "next_id" && warnings[0].message.contains("4294967295"),
        "warning must name the field and the colliding id: {warnings:?}"
    );
}

/// One below the boundary: `u32::MAX - 1` still has a valid, non-colliding successor
/// (`u32::MAX` itself), so ordinary max-plus-one logic applies with no saturation, and
/// nothing was silently adjusted — no `Problem` should be reported.
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

    let (state, warnings) = load(&path);

    assert_eq!(state.next_id, u32::MAX);
    assert!(warnings.is_empty(), "no adjustment happened: {warnings:?}");
}

/// An empty window list never triggers the max-plus-one path at all, so a saved
/// `next_id` of `u32::MAX` (or anything else) passes through untouched, with no
/// `Problem` — the loaded value matches the file exactly, nothing was adjusted.
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

    let (state, warnings) = load(&path);

    assert_eq!(state.next_id, u32::MAX);
    assert!(warnings.is_empty(), "no adjustment happened: {warnings:?}");
}

/// Minor #3, the other silent adjustment: a saved `next_id` past `u32::MAX` (a
/// hand-edited or corrupted file — nothing this build ever writes exceeds `u32::MAX`)
/// used to saturate to `u32::MAX` with zero problems reported, even though the loaded
/// value now differs from the file's own by billions.
#[test]
fn oversized_next_id_is_reported_as_a_problem() {
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let contents = serde_json::json!({
        "version": 2,
        "next_id": 99_999_999_999u64,
        "windows": []
    });
    std::fs::write(&path, serde_json::to_vec(&contents).unwrap()).unwrap();

    let (state, warnings) = load(&path);

    assert_eq!(state.next_id, u32::MAX);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].severity, Severity::Warn);
    assert!(
        warnings[0].key == "next_id" && warnings[0].message.contains("99999999999"),
        "warning must name the field and the out-of-range value: {warnings:?}"
    );
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

/// Re-review Major #2: `record_json_shape` above sets `worktree: None`, so nothing in
/// this file ever asserted `WorktreeRecord`'s own literal wire keys. Wave 2 gave
/// `worktree.repo_root` a distinct *value* in `sample_state()` and reasoned from that
/// that a rename would be caught — but a value distinct from its siblings and a key
/// name being the right one are different properties; `save_then_load_round_trips` (a
/// self-consistent bijection) cannot see a consistent rename either, and nothing else
/// looked. Pinning `repo_root`/`path`/`branch` here the same way `record_json_shape`
/// pins `WindowRecord`'s keys closes exactly that gap: a `#[serde(rename = "root")]`
/// added to `repo_root` must fail this test (verified below, then reverted — see the
/// task report).
#[test]
fn worktree_record_json_shape() {
    let worktree = WorktreeRecord {
        repo_root: PathBuf::from("/Users/me/repos/.bare/shop"),
        path: PathBuf::from("/Users/me/repos/shop-worktrees/api-worker"),
        branch: "feature/api-worker".into(),
    };

    let json = serde_json::to_string(&worktree).unwrap();
    let expected = "{\"repo_root\":\"/Users/me/repos/.bare/shop\",\
\"path\":\"/Users/me/repos/shop-worktrees/api-worker\",\
\"branch\":\"feature/api-worker\"}";
    assert_eq!(json, expected);
}

/// `spawn_persister`'s own risk cases: decision 9's debounce loop is a classic place to
/// lose the last write, and the failure mode that matters is a user's final change never
/// reaching disk. `crates/daemon/tests/manager.rs`'s `persister_writes_changes_within_a_second`
/// already covers the acceptance case (a burst reaches disk as the final state, within a
/// second); the tests below target the specific ways a *naive* debouncer breaks that a
/// single burst-and-settle test cannot distinguish from a correct one: a debounce that
/// blocks cancellation, and a debounce that writes on every change instead of skipping a
/// no-op one.
///
/// Not tested here: "a change arriving during the write". `save`'s own blocking write is
/// a handful of syscalls, far too fast to land a real change inside deterministically
/// without a testing hook this module's interface does not have. It is structurally safe
/// regardless — `spawn_persister`'s loop does not `select!` on `changes.changed()` while
/// awaiting the save's `spawn_blocking` task, so any change the `watch::Receiver` sees
/// during that await stays marked and is picked up by the very next `changes.changed()`
/// call once the write returns — and the same tight, sleep-free rename loop
/// `persister_writes_changes_within_a_second` already drives is, in practice, exactly the
/// kind of burst that lands changes while a write is in flight.
mod persister_tests {
    use super::*;
    use crate::manager::{ManagerConfig, WindowManager};

    /// A manager with no real windows; the event channel is drained so `handle_event`
    /// calls (none, in these tests) would not need anywhere to go, matching the pattern
    /// every other daemon test harness uses.
    fn test_manager() -> Arc<WindowManager> {
        let (m, mut events) = WindowManager::new(ManagerConfig::new(
            "/tmp/anthrex-persister-test.sock".into(),
            "/bin/sh".into(),
        ));
        tokio::spawn(async move { while events.recv().await.is_some() {} });
        m
    }

    /// One dormant window, restored rather than spawned: `spawn_persister`'s subject is
    /// the debounce loop, not a real PTY, and `restore` is the cheapest, fastest way to
    /// get a window whose name `rename` can change to trigger `watch()` publications.
    fn seed_one_window(m: &WindowManager, id: u32, name: &str) {
        m.restore(StateFile {
            version: STATE_VERSION,
            next_id: id + 1,
            windows: vec![WindowRecord {
                id,
                name: name.to_string(),
                runtime: Runtime::Shell,
                cwd: PathBuf::from("/tmp"),
                project: None,
                worktree: None,
                model: None,
                initial_prompt: None,
                session_id: None,
                created_at: 1,
                status: Status::Exited,
                run: None,
            }],
            runs: Vec::new(),
        });
    }

    /// A single change followed at once by cancellation must not hold shutdown up for
    /// the debounce window: decision 9's wait is meant to collect a burst, not to give a
    /// lone change priority over the daemon actually stopping. A naive debouncer that
    /// merely `sleep`s for `SAVE_DEBOUNCE` before checking for cancellation would fail
    /// this by taking (at least) that long to return.
    #[tokio::test(start_paused = true)]
    async fn a_single_change_then_immediate_cancellation_stops_promptly() {
        let m = test_manager();
        seed_one_window(&m, 1, "before");
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(&dir);
        let shutdown = CancellationToken::new();
        let handle = spawn_persister(m.clone(), path, shutdown.clone());

        // Let the persister actually subscribe before the change, then fire one change
        // and cancel with no gap at all — not even a yield.
        tokio::task::yield_now().await;
        m.rename(1, "after".into()).unwrap();
        shutdown.cancel();

        tokio::time::timeout(Duration::from_millis(20), handle)
            .await
            .expect("persister did not stop promptly after a single change")
            .unwrap();
    }

    /// A burst that never goes quiet on its own must still release the persister the
    /// instant it is cancelled — decision 9's collection window must never turn into an
    /// unbounded wait for quiet — and, separately, `state.json` must not go arbitrarily
    /// stale while that burst keeps running: [`SAVE_MAX_DELAY`] (fix wave 4, item 2)
    /// bounds how long a sustained stream of changes can hold the write back. Before that
    /// bound existed, this test built exactly this never-quiet input and asserted only
    /// the cancellation half — the right scenario, checking the wrong property. A naive
    /// debouncer whose collection window restarts on every change (only the outer,
    /// pre-debounce `changed()` wait had no restart problem) passes the cancellation
    /// assertion below while starving `state.json` for as long as the burst continues.
    #[tokio::test(start_paused = true)]
    async fn cancellation_during_a_never_quiet_burst_is_not_delayed() {
        let m = test_manager();
        seed_one_window(&m, 1, "before");
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(&dir);
        let shutdown = CancellationToken::new();
        let handle = spawn_persister(m.clone(), path.clone(), shutdown.clone());

        let burst_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let burst_manager = m.clone();
        let burst_stop_flag = burst_stop.clone();
        let burst = tokio::spawn(async move {
            let mut n: u64 = 0;
            while !burst_stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
                n += 1;
                burst_manager.rename(1, format!("burst-{n}")).unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });

        // The burst never goes quiet for longer than SAVE_DEBOUNCE, so only
        // SAVE_MAX_DELAY can make a write land. Wait past that bound, with the burst
        // still running, and confirm state.json moved on from the pre-burst "before" —
        // to some "burst-*" name — despite the stream never settling.
        tokio::time::sleep(SAVE_MAX_DELAY + SAVE_DEBOUNCE).await;
        let read_path = path.clone();
        let contents = tokio::task::spawn_blocking(move || std::fs::read_to_string(&read_path))
            .await
            .unwrap()
            .unwrap_or_default();
        assert!(
            contents.contains("burst-"),
            "state.json did not become current within SAVE_MAX_DELAY while the burst \
             kept running (a debounce window that only ever restarts starves the write \
             indefinitely): {contents:?}"
        );

        // Run well past SAVE_DEBOUNCE while the burst keeps the debounce window
        // perpetually restarting, then cancel with the burst still going.
        tokio::time::sleep(SAVE_DEBOUNCE * 5).await;
        shutdown.cancel();
        tokio::time::timeout(Duration::from_millis(20), handle)
            .await
            .expect("persister did not stop promptly during a never-quiet burst")
            .unwrap();

        burst_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        burst.await.unwrap();
    }

    /// Decision 9: "it skips the write when the serialized bytes equal the last ones it
    /// wrote." A rename to a window's *own current name* still publishes on `watch()`
    /// (`WindowManager::rename` does not special-case a no-op), so it is exactly the case
    /// that would make a debouncer that writes unconditionally on every change perform a
    /// write whose bytes are indistinguishable from the one already on disk. Checked by
    /// mtime, the only externally observable trace of a write `save`'s own atomic-rename
    /// implementation leaves: content equality alone cannot tell "skipped" from "wrote
    /// the same bytes again".
    #[tokio::test]
    async fn a_no_op_change_is_not_written_but_the_next_real_one_is() {
        let m = test_manager();
        seed_one_window(&m, 1, "same-name");
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(&dir);
        let shutdown = CancellationToken::new();
        let handle = spawn_persister(m.clone(), path.clone(), shutdown.clone());
        // `spawn_persister` only subscribes to `watch()` once its task actually runs;
        // without this, `rename` below can race ahead of that subscription and publish a
        // change the persister was never listening for yet.
        tokio::task::yield_now().await;

        // Wait for the first write (seeding this window is itself a call this test made
        // before the persister subscribed, so nothing has published yet — `rename` to the
        // window's own name is both the trigger and, deliberately, a no-op).
        m.rename(1, "same-name".into()).unwrap();
        wait_for_content(&path, |s| s.contains("same-name"), Duration::from_secs(2)).await;
        let after_first_write = std::fs::metadata(&path).unwrap().modified().unwrap();

        // A second no-op change: same name again. If this were written, the mtime would
        // advance even though the bytes are identical to what is already on disk.
        m.rename(1, "same-name".into()).unwrap();
        tokio::time::sleep(SAVE_DEBOUNCE * 3).await;
        let after_no_op = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            after_first_write, after_no_op,
            "a change whose bytes match the last write must not cause a second write"
        );

        // A real change must still reach disk.
        m.rename(1, "actually-different".into()).unwrap();
        wait_for_content(
            &path,
            |s| s.contains("actually-different"),
            Duration::from_secs(2),
        )
        .await;

        shutdown.cancel();
        handle.await.unwrap();
    }

    async fn wait_for_content(path: &Path, mut pred: impl FnMut(&str) -> bool, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Ok(bytes) = std::fs::read(path)
                && let Ok(text) = String::from_utf8(bytes)
                && pred(&text)
            {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for state.json to reflect the expected content"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
