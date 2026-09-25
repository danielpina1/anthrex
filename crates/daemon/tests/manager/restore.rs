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
/// `project`, `worktree` (the watched root) vs `managed.path` (the checkout anthrex
/// made) vs `managed.repo_root`, `model` vs `initial_prompt`, and `name` vs
/// `session_id`. A record where any of these pairs coincide cannot catch a transposition
/// between them — see `restore_and_state_snapshot_do_not_transpose_a_same_typed_field_pair`.
///
/// `worktree` and `managed.path` are deliberately different here even though production
/// keeps them equal for a managed window: equal values are exactly what would hide a
/// transposition between the two fields this milestone split apart.
fn distinct_record() -> WindowRecord {
    WindowRecord {
        id: 5,
        name: "record-name".into(),
        runtime: Runtime::Claude,
        cwd: PathBuf::from("/tmp/distinct/cwd-value"),
        project: Some(PathBuf::from("/tmp/distinct/project-value")),
        worktree: Some(PathBuf::from("/tmp/distinct/watched-root-value")),
        managed: Some(state::WorktreeRecord {
            repo_root: PathBuf::from("/tmp/distinct/repo-root-value"),
            path: PathBuf::from("/tmp/distinct/managed-path-value"),
            branch: "feature/distinct".into(),
        }),
        model: Some("model-value".into()),
        initial_prompt: Some("initial-prompt-value".into()),
        session_id: Some("session-id-value".into()),
        created_at: 1_650_000_000,
        status: Status::Idle,
        run: None,
        kind: Default::default(),
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
    let managed = record.managed.clone().unwrap();
    assert_eq!(
        info.worktree,
        record.worktree.clone(),
        "the watched root comes from `worktree`, not from `managed.path`"
    );
    assert_eq!(info.branch, Some(managed.branch.clone()));
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

#[path = "restore/persistence.rs"]
mod persistence;
