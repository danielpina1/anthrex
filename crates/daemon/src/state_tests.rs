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

#[path = "state_tests/schema.rs"]
mod schema;

#[path = "state_tests/persister.rs"]
mod persister_tests;
