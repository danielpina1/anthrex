//! Load-leniency tests: what `load` accepts, tolerates or rejects at the record and
//! schema level, as opposed to the file-level recovery (missing, unreadable, corrupt
//! bytes) covered in the parent module. Missing vs. malformed `windows`/`runs` fields,
//! version handling (decision 13's version-1 files, a too-new version), per-record
//! rejection and the id/name collision rules that must not poison a later legitimate
//! reuse, `next_id`'s `u32` boundary arithmetic, and the wire-format pins for
//! `WindowRecord` and `WorktreeRecord`.

use super::*;

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
