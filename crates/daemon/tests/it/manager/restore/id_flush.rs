//! A window's id reaches `state.json` before anything can learn it. After a crash the
//! next daemon rebuilds `next_id` from that file alone, so an id published but not yet
//! saved (the persister's debounce, `state::SAVE_DEBOUNCE` up to `SAVE_MAX_DELAY`) would
//! be handed to another window, and a reconnecting client that kept focus by id would
//! type into it. Real shells in real PTYs, a real persister and real files. A submodule
//! of `restore.rs` (AGENTS.md rule 8), sharing its fixtures via `use super::*`.

use super::*;
use tokio_util::sync::CancellationToken;

/// The production wiring: `spawn_persister` keeps the file, and `create` waits for it to
/// record the id it spent. The file is read the instant `create` returns, well inside
/// the debounce, and a fresh manager restored from it (no final flush: a crash) must not
/// hand the same id out again.
#[tokio::test]
async fn a_window_id_is_on_disk_before_its_create_returns() {
    let m = manager();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let shutdown = CancellationToken::new();
    let persister = state::spawn_persister(m.clone(), path.clone(), shutdown.clone());

    let info = create(&m, spec("flushed"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    let (on_disk, _problems) = state::load(&path);
    assert!(
        on_disk.next_id > info.id,
        "window {} was published before state.json recorded its id (next_id {})",
        info.id,
        on_disk.next_id
    );

    // The crash: the persister stops with no final flush.
    shutdown.cancel();
    persister.await.unwrap();
    m.remove(info.id).unwrap();

    let reborn = manager();
    reborn.restore(state::load(&path).0);
    let next = create_id(&reborn, spec("after-crash"), std::env::temp_dir(), 80, 24).await;
    reborn.remove(next).unwrap();
    assert_ne!(
        next, info.id,
        "the restarted daemon reissued an id a client may still be focused on"
    );
}
