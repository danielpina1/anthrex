//! The real state.json round trips: `spawn_persister`'s debounce getting a live change
//! to disk, and decision 22's name sanitization on a load from real bytes on disk (a
//! hand-edited or foreign-tool-written file, not a `StateFile` built in memory) —
//! everything in this file goes through a real `state::save`/`state::load` against a
//! tempdir path, as opposed to the parent module's in-memory `StateFile` construction. A
//! submodule of `restore.rs` (AGENTS.md rule 8), sharing its fixtures via `use super::*`.

use super::*;

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
    // Whole-branch-review m18: this used to be a bare `Duration::from_secs(1)`,
    // numerically equal to `state::SAVE_MAX_DELAY` (1s) even though the burst below
    // ends immediately, so the write actually settles on `SAVE_DEBOUNCE` (100ms) with
    // roughly a 10x real margin -- derived from the constants instead, matching
    // `state_tests.rs`'s own sibling sites.
    let deadline = Instant::now() + state::SAVE_MAX_DELAY + state::SAVE_DEBOUNCE;
    loop {
        let (loaded, _problems) = state::load(&path);
        if loaded.windows.iter().any(|w| w.id == id) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "state.json never listed the new window within SAVE_MAX_DELAY + SAVE_DEBOUNCE"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    for n in 0..20 {
        m.rename(id, format!("name-{n}")).unwrap();
    }
    let deadline = Instant::now() + state::SAVE_MAX_DELAY + state::SAVE_DEBOUNCE;
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
