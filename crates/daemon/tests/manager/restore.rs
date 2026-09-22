//! `WindowManager::restore` and the state persister: rebuilding the window table from a
//! saved `StateFile`, the collision and id-space rulings that came out of the M6.5 review,
//! and the debounce that gets a live change to disk.
//!
//! A submodule of `tests/manager.rs` (AGENTS.md rule 8); shares that file's fixtures via
//! `use super::*`, including `window_record`, which `restart.rs` also needs to build a
//! dormant window to restart.

use super::*;
use unicode_segmentation::UnicodeSegmentation;

/// Ruling, fix wave 4 item 7: a restored record's null `project` must survive a
/// restore-then-`state_snapshot` cycle still null — never rewritten to `cwd` and baked
/// permanently into the saved state by the next write. `window_record` (the parent
/// `manager.rs`'s helper, shared with `restart.rs`) always sets `project: None`;
/// `state_snapshot` is the "save" half of the cycle under test here
/// (`spawn_persister`'s real writes go through the very same function). The *display*
/// value (`WindowInfo.project`, which has no null case) is allowed to fall back to `cwd`
/// — that derivation happens at `Entry::info`, not by mutating what gets persisted.
#[tokio::test]
async fn a_null_project_survives_a_restore_and_state_snapshot_cycle_still_null() {
    let m = manager();
    let cwd = std::env::temp_dir();
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![window_record(1, "no-project", cwd.clone(), None)],
        runs: Vec::new(),
    });

    let info = find(&m, 1);
    assert_eq!(
        info.project, cwd,
        "the live display value may fall back to cwd: {info:?}"
    );

    let snapshot = m.state_snapshot();
    let record = snapshot
        .windows
        .iter()
        .find(|w| w.id == 1)
        .expect("restored record is in the snapshot");
    assert_eq!(
        record.project, None,
        "a restored null project must not be rewritten to cwd and baked into the saved \
         state: {record:?}"
    );
}

/// Whole-branch-review Major 2: `state.rs`'s own forward-compatibility rationale for
/// carrying `runs`/`run` as opaque JSON is "a newer daemon's runs survive a round trip
/// through an older build without this module having to know their shape" — but
/// `state_snapshot` used to write `runs: Vec::new()` and every record's `run: None`
/// unconditionally, so the *first save* after loading a file with real `runs`/`run` data
/// erased it, exactly contradicting that rationale. This pins the fix: both a top-level
/// `runs` entry and a per-window `run` value must survive load (`restore`) -> save
/// (`state_snapshot`) -> load again, byte-for-byte, even though this milestone gives
/// neither field any real meaning of its own yet (decision 8).
#[tokio::test]
async fn runs_and_run_survive_a_restore_and_state_snapshot_cycle_unchanged() {
    let m = manager();
    let run_value = serde_json::json!({"id": "r1", "label": "keep me", "future_field": 42});
    let mut record = window_record(1, "has-run", std::env::temp_dir(), None);
    record.run = Some(run_value.clone());
    let runs_value = vec![serde_json::json!({"id": "r1", "label": "keep me"})];

    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![record],
        runs: runs_value.clone(),
    });

    let snapshot = m.state_snapshot();
    assert_eq!(
        snapshot.runs, runs_value,
        "top-level `runs` did not survive restore -> state_snapshot unchanged"
    );
    let saved_record = snapshot
        .windows
        .iter()
        .find(|w| w.id == 1)
        .expect("restored record is in the snapshot");
    assert_eq!(
        saved_record.run,
        Some(run_value),
        "the window's `run` did not survive restore -> state_snapshot unchanged"
    );

    // And a second restore from that very snapshot must see the same data again —
    // the actual load -> save -> load shape decision 8's rationale is about.
    let m2 = manager();
    m2.restore(snapshot.clone());
    let round_tripped = m2.state_snapshot();
    assert_eq!(round_tripped.runs, snapshot.runs);
    assert_eq!(
        round_tripped
            .windows
            .iter()
            .find(|w| w.id == 1)
            .unwrap()
            .run,
        saved_record.run
    );
}

/// M6.5's headline acceptance test (decision 14): a restored window is listed as exited,
/// carries its saved identity, is viewable with a placeholder screen, refuses input,
/// tolerates `kill`, still guards its name against a duplicate `create`, and never lets a
/// later `create` reuse its id.
#[tokio::test]
async fn restored_windows_are_exited_and_viewable() {
    let m = manager();
    let state = StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![window_record(1, "kept", std::env::temp_dir(), Some("s-1"))],
        runs: Vec::new(),
    };
    m.restore(state);

    let info = find(&m, 1);
    assert_eq!(info.name, "kept");
    assert_eq!(info.status, Status::Exited);
    assert_eq!(info.session_id.as_deref(), Some("s-1"));
    let exit = info.exit.expect("a restored window carries an exit reason");
    assert_eq!(exit.reason, DAEMON_RESTARTED);
    assert_eq!(exit.code, None);

    let att = m.attach(1).unwrap();
    let mut parser = vt100::Parser::new(att.rows, att.cols, 0);
    parser.process(&att.snapshot);
    let screen = parser.screen().contents();
    assert!(
        screen.contains("this window stopped when the daemon restarted"),
        "{screen}"
    );
    assert!(screen.contains("resumes session s-1"), "{screen}");

    let err = m.write_input(1, b"echo hi\n").unwrap_err().to_string();
    assert!(err.contains("not running"), "{err}");

    m.kill(1).unwrap();

    let err = create(&m, spec("kept"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    let fresh = create(&m, spec("fresh"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    assert!(
        fresh.id > 1,
        "the next created window must not collide with the restored id: {fresh:?}"
    );
}

/// Minor 1, M6.5 review: a restored record whose id matches a live window must not
/// silently replace it. Before this fix, `inner.entries.insert(id, entry)` just dropped
/// the returned live `Window` with no kill and no `start_cleanup`, orphaning its PTY
/// child. `restore` runs once on an empty manager today, so this could not happen from
/// the one caller that exists — but that "it cannot happen from the only current caller"
/// reasoning is exactly what let the same id-collision class through as two Criticals one
/// task earlier, and `restore` is a public entry point in its own right, not trusted to
/// get `next_id` right by its own doc comment.
#[tokio::test]
async fn restore_refuses_a_record_whose_id_collides_with_a_live_window() {
    let m = manager();
    let live = create(&m, spec("live"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();

    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: live.id + 1,
        windows: vec![window_record(live.id, "ghost", std::env::temp_dir(), None)],
        runs: Vec::new(),
    });

    let info = find(&m, live.id);
    assert_eq!(
        info.name, "live",
        "the live window must survive a colliding restore, not be replaced by the ghost record: {info:?}"
    );
    assert_ne!(
        info.status,
        Status::Exited,
        "the live window must not be turned dormant by a colliding restore: {info:?}"
    );
}

/// The other half of Minor 1: a *name* collision must be refused too, even when the
/// colliding record's id is new. Two restores in a row is the shape the review's own
/// probe used — the second restore's record shares a name with an entry the first
/// restore already inserted.
#[tokio::test]
async fn restore_refuses_a_record_whose_name_collides_with_an_existing_entry() {
    let m = manager();
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![window_record(1, "first", std::env::temp_dir(), None)],
        runs: Vec::new(),
    });

    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 3,
        windows: vec![window_record(2, "first", std::env::temp_dir(), None)],
        runs: Vec::new(),
    });

    let list = m.list();
    assert_eq!(
        list.len(),
        1,
        "the second restore's colliding-name record must be refused, not silently taken: {list:?}"
    );
    assert_eq!(list[0].id, 1, "{list:?}");
    assert_eq!(list[0].name, "first", "{list:?}");
}

/// Decision 9: `state_snapshot` mirrors the live window table exactly, with no I/O
/// (verified indirectly — this only checks the shape it comes back with; the persister
/// tests below check it never blocks a change from reaching disk).
#[tokio::test]
async fn state_snapshot_reflects_the_window_table() {
    let m = manager();
    // `project` is deliberately not `spec.cwd` (both are `PathBuf`s `state_snapshot`
    // could transpose without either test-fixture value making the swap visible): a
    // review of this exact task found that blind spot has already shipped three
    // same-typed-field-swap bugs elsewhere in this milestone.
    let project_one = PathBuf::from("/some/distinct/project/one");
    let a = create_id(&m, spec("one"), project_one.clone(), 80, 24).await;
    let _b = create_id(&m, spec("two"), std::env::temp_dir(), 80, 24).await;
    m.rename(a, "renamed".into()).unwrap();

    let snapshot = m.state_snapshot();

    assert_eq!(snapshot.version, state::STATE_VERSION);
    assert_eq!(snapshot.next_id, 3);
    assert!(snapshot.runs.is_empty());
    assert_eq!(snapshot.windows.len(), 2);
    let mut names: Vec<_> = snapshot.windows.iter().map(|w| w.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["renamed", "two"]);
    let renamed = snapshot
        .windows
        .iter()
        .find(|w| w.name == "renamed")
        .unwrap();
    assert_eq!(renamed.cwd, std::env::temp_dir());
    assert_eq!(renamed.project.as_deref(), Some(project_one.as_path()));
    for record in &snapshot.windows {
        assert_eq!(record.runtime, Runtime::Shell);
        assert!(record.created_at > 0, "{record:?}");
    }
}

/// A fixture with a distinct, recognisable value for every same-typed field pair
/// `restore` or `state_snapshot`'s hand-written mapping could silently swap: `cwd` vs
/// `project`, `worktree.repo_root` vs `worktree.path`, `model` vs `initial_prompt`, and
/// `name` vs `session_id`. A record where any of these pairs coincide cannot catch a
/// transposition between them — see `restore_and_state_snapshot_do_not_transpose_a_same_typed_field_pair`.
fn distinct_record() -> WindowRecord {
    WindowRecord {
        id: 5,
        name: "record-name".into(),
        runtime: Runtime::Claude,
        cwd: PathBuf::from("/tmp/distinct/cwd-value"),
        project: Some(PathBuf::from("/tmp/distinct/project-value")),
        worktree: Some(state::WorktreeRecord {
            repo_root: PathBuf::from("/tmp/distinct/repo-root-value"),
            path: PathBuf::from("/tmp/distinct/worktree-path-value"),
            branch: "feature/distinct".into(),
        }),
        model: Some("model-value".into()),
        initial_prompt: Some("initial-prompt-value".into()),
        session_id: Some("session-id-value".into()),
        created_at: 1_650_000_000,
        status: Status::Idle,
        run: None,
    }
}

/// Guards the exact blind spot the M6.5 review flagged: before this task, `state.rs` was
/// pure serde derive on both sides of the (de)serialization, so there was nowhere for two
/// same-typed fields to be silently swapped. `restore` and `state_snapshot`'s hand-written
/// `Entry ↔ WindowRecord` mappings are the first real conflation sites in this module.
///
/// Checked through two different surfaces so a bug in one direction cannot cancel out a
/// matching bug in the other and pass anyway: `WindowInfo` (via `list()`) verifies most of
/// `restore`'s mapping independently of `state_snapshot`; `initial_prompt` has no
/// `WindowInfo` field to read back through, so `state_snapshot`'s own round trip is the
/// only way to check it — and it also re-checks every other field on the way back out.
#[tokio::test]
async fn restore_and_state_snapshot_do_not_transpose_a_same_typed_field_pair() {
    let m = manager();
    let record = distinct_record();
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: record.id + 1,
        windows: vec![record.clone()],
        runs: Vec::new(),
    });

    let info = find(&m, record.id);
    assert_eq!(info.name, record.name);
    assert_eq!(info.cwd, record.cwd);
    assert_eq!(info.project, record.project.clone().unwrap());
    let worktree = record.worktree.clone().unwrap();
    assert_eq!(info.worktree, Some(worktree.path.clone()));
    assert_eq!(info.branch, Some(worktree.branch.clone()));
    assert_eq!(info.session_id, record.session_id);
    assert_eq!(info.model, record.model);

    let snapshot = m.state_snapshot();
    let out = snapshot
        .windows
        .iter()
        .find(|w| w.id == record.id)
        .expect("restored record is in the snapshot");
    assert_eq!(out.name, record.name);
    assert_eq!(out.cwd, record.cwd);
    assert_eq!(out.project, record.project);
    assert_eq!(out.worktree, record.worktree);
    assert_eq!(out.model, record.model);
    assert_eq!(out.initial_prompt, record.initial_prompt);
    assert_eq!(out.session_id, record.session_id);
}

/// Sparse restored ids (1 and 9, not 1 and 2) are the case a naive "count of restored
/// windows" or "max saved next_id, trusted verbatim" implementation gets wrong: `restore`
/// must derive `next_id` from the *largest loaded id*, not from how many records there
/// are or from a caller-supplied `next_id` alone, or a `create` a few windows later would
/// silently reuse id 9.
#[tokio::test]
async fn restore_next_id_skips_a_sparse_gap() {
    let m = manager();
    let state = StateFile {
        version: state::STATE_VERSION,
        // Deliberately wrong/stale: a correct `next_id` would already be 10. `restore`
        // must not trust this value blindly.
        next_id: 2,
        windows: vec![
            window_record(1, "low", std::env::temp_dir(), None),
            window_record(9, "high", std::env::temp_dir(), None),
        ],
        runs: Vec::new(),
    };
    m.restore(state);

    for n in 0..9 {
        let info = create(
            &m,
            spec(&format!("filler-{n}")),
            std::env::temp_dir(),
            80,
            24,
        )
        .await
        .unwrap();
        assert_ne!(
            info.id, 9,
            "a created window must never reuse the restored id 9"
        );
    }
}

/// The other half of the saturation risk: a restored record already holding `u32::MAX`
/// leaves no larger id to hand out. `create` must refuse loudly rather than let
/// `next_id`'s `+= 1` wrap back to 0 and silently reuse an id already on the table.
#[tokio::test]
async fn create_refuses_when_the_restored_id_space_is_exhausted() {
    let m = manager();
    let state = StateFile {
        version: state::STATE_VERSION,
        next_id: 1,
        windows: vec![window_record(u32::MAX, "maxed", std::env::temp_dir(), None)],
        runs: Vec::new(),
    };
    m.restore(state);

    let err = create(&m, spec("one-too-many"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no window ids remain"), "{err}");
    // The refusal must not have created (and then abandoned) a window either.
    assert_eq!(m.list().len(), 1, "{:?}", m.list());
}

/// Decision 9's debounce: a burst of changes (twenty renames, no sleeps between them)
/// reaches disk as one write of the *final* state, and it does so within a second even
/// though the persister only fires 100 ms after the burst goes quiet.
#[tokio::test]
async fn persister_writes_changes_within_a_second() {
    let m = manager();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let shutdown = tokio_util::sync::CancellationToken::new();
    let handle = state::spawn_persister(m.clone(), path.clone(), shutdown.clone());

    let id = create_id(&m, spec("original"), std::env::temp_dir(), 80, 24).await;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let (loaded, _problems) = state::load(&path);
        if loaded.windows.iter().any(|w| w.id == id) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "state.json never listed the new window within 1s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    for n in 0..20 {
        m.rename(id, format!("name-{n}")).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let (loaded, _problems) = state::load(&path);
        if loaded
            .windows
            .iter()
            .any(|w| w.id == id && w.name == "name-19")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "state.json never settled on the final rename within 1s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    shutdown.cancel();
    handle.await.unwrap();
}

/// Fix wave 6, Major finding: `state.json` is a user-editable input, not a private
/// serialization format — `state::load` only rejects a record that repeats an earlier
/// record's own id or name *within the file*; it never applies `validate_name`'s length or
/// character rules. A hand-edited record whose name contains `\x1b` (the review's own
/// concrete example, `state.json`'s `windows[0].name` set to `"bad\x1bname"`) must not load
/// silently into `Entry.name` and be rendered verbatim by `anthrex ls`/the TUI sidebar.
///
/// Ruling: sanitize, do not reject — the window (its PTY history, session id, worktree
/// record) survives with its name repaired, and a `Problem`-style warning is logged naming
/// the window, rather than the record being dropped the way an id/name collision is. This
/// goes through the real file, exactly as the review reproduced it: `state::save` writes a
/// hand-built `StateFile` with the bad name to disk, `state::load` reads the bytes back
/// (proving the sanitizer runs on the same path a hand-edited or foreign-tool-written file
/// takes, not just on a `StateFile` built in memory), then `restore` is what the manager
/// ends up holding is asserted against.
#[tokio::test]
async fn restore_sanitizes_a_control_character_name_loaded_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    state::save(
        &path,
        &StateFile {
            version: state::STATE_VERSION,
            next_id: 2,
            windows: vec![window_record(1, "bad\x1bname", std::env::temp_dir(), None)],
            runs: Vec::new(),
        },
    )
    .unwrap();

    let (loaded, problems) = state::load(&path);
    assert!(
        problems.is_empty(),
        "state::load itself does not validate names, only structure: {problems:?}"
    );

    let m = manager();
    m.restore(loaded);

    let list = m.list();
    assert_eq!(
        list.len(),
        1,
        "a bad name must not cost the user their window: {list:?}"
    );
    let info = &list[0];
    assert!(
        !info.name.chars().any(char::is_control),
        "the restored name must be sanitized, not loaded verbatim: {:?}",
        info.name
    );
    assert_eq!(
        info.name, "bad_name",
        "each disallowed character is replaced, not stripped, so the name keeps its shape"
    );
}

/// The other half of decision 22's rule set restore must now honour: length. A 500-byte
/// name (well past the 64-*grapheme*-cluster limit) is truncated, not rejected outright.
#[tokio::test]
async fn restore_sanitizes_an_oversized_name_loaded_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let oversized = "a".repeat(500);
    state::save(
        &path,
        &StateFile {
            version: state::STATE_VERSION,
            next_id: 2,
            windows: vec![window_record(1, &oversized, std::env::temp_dir(), None)],
            runs: Vec::new(),
        },
    )
    .unwrap();

    let (loaded, _problems) = state::load(&path);
    let m = manager();
    m.restore(loaded);

    let info = find(&m, 1);
    assert_eq!(
        info.name.graphemes(true).count(),
        64,
        "an oversized restored name must be truncated to decision 22's limit: {:?}",
        info.name
    );
    assert_eq!(info.name, "a".repeat(64));
}

/// Fix wave 6, Minor: the bidi rule from `validate_name` applies here too ("apply the same
/// rule in the sanitizer... so a bad name cannot enter through the file either"). U+202E
/// alone, once its (imaginary) surrounding control characters are gone, is still a `Cf`
/// bidi override and must still be replaced.
#[tokio::test]
async fn restore_sanitizes_a_bidi_override_name_loaded_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let bidi = "bad\u{202E}name";
    state::save(
        &path,
        &StateFile {
            version: state::STATE_VERSION,
            next_id: 2,
            windows: vec![window_record(1, bidi, std::env::temp_dir(), None)],
            runs: Vec::new(),
        },
    )
    .unwrap();

    let (loaded, _problems) = state::load(&path);
    let m = manager();
    m.restore(loaded);

    let info = find(&m, 1);
    assert_eq!(info.name, "bad_name");
}

/// Two records that each fail validation for a different reason can still sanitize to the
/// *same* base string (two distinct control characters both become `_`); the ruling
/// requires the sanitized result "cannot itself collide with another window's name" — so
/// both windows must survive restore with distinct names, not have the second silently
/// dropped as a duplicate the way a genuine same-name collision between two otherwise-valid
/// records already is (`restore_refuses_a_record_whose_name_collides_with_an_existing_entry`,
/// above — that existing drop-on-collision behaviour is for *valid* names and must stay
/// unchanged).
#[tokio::test]
async fn restore_disambiguates_two_records_that_sanitize_to_the_same_name() {
    let m = manager();
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 3,
        windows: vec![
            window_record(1, "\x01", std::env::temp_dir(), None),
            window_record(2, "\x02", std::env::temp_dir(), None),
        ],
        runs: Vec::new(),
    });

    let list = m.list();
    assert_eq!(
        list.len(),
        2,
        "both windows must survive even though their bad names sanitize to the same string: {list:?}"
    );
    let mut names: Vec<_> = list.iter().map(|w| w.name.clone()).collect();
    names.sort();
    assert_ne!(
        names[0], names[1],
        "two sanitized names must never collide: {names:?}"
    );
    for name in &names {
        assert!(!name.chars().any(char::is_control), "{names:?}");
    }
}

/// The stability half of the ruling: "a name that changes on every restart would be worse
/// than the bug." Sanitizing, saving, reloading and restoring a second time (into a fresh
/// manager, so nothing carries over except what round-trips through the file) must produce
/// the identical name the first restore settled on.
#[tokio::test]
async fn a_sanitized_name_is_stable_across_a_save_and_reload_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    state::save(
        &path,
        &StateFile {
            version: state::STATE_VERSION,
            next_id: 2,
            windows: vec![window_record(1, "bad\x1bname", std::env::temp_dir(), None)],
            runs: Vec::new(),
        },
    )
    .unwrap();

    let (first_loaded, _) = state::load(&path);
    let first_manager = manager();
    first_manager.restore(first_loaded);
    let first_name = find(&first_manager, 1).name;

    let snapshot = first_manager.state_snapshot();
    state::save(&path, &snapshot).unwrap();

    let (second_loaded, second_problems) = state::load(&path);
    assert!(
        second_problems.is_empty(),
        "a sanitized name must already be valid on the next load: {second_problems:?}"
    );
    let second_manager = manager();
    second_manager.restore(second_loaded);
    let second_name = find(&second_manager, 1).name;

    assert_eq!(
        first_name, second_name,
        "a sanitized name must not change on every restart"
    );
}
