//! Decision 16: `<data_dir>/brakes.json`, written atomically and privately, and read
//! failing closed. Real files in a scratch directory; the unreadable case is a FIFO
//! nobody writes, released at the end (ruling C26).

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use proto::brakes::DispatchHold;
use serde_json::json;

use super::{BRAKES_FILE, CORRUPT_BY, UNREADABLE_BY, load, save};

const NOW: u64 = 1_790_000_000;
/// What a test's own reads and writes are given: the production bound.
const WAIT: Duration = super::BRAKES_IO_WAIT;

fn user_hold() -> DispatchHold {
    DispatchHold {
        since: 1_789_999_000,
        by: "user".into(),
    }
}

fn on_disk(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir.join(BRAKES_FILE)).unwrap()).unwrap()
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn save_is_atomic_and_private() {
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), Some(user_hold()), WAIT).await.unwrap();
    assert_eq!(
        on_disk(dir.path()),
        json!({"v": 1, "hold": {"since": 1_789_999_000u64, "by": "user"}})
    );
    let mode = std::fs::metadata(dir.path().join(BRAKES_FILE))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(names(dir.path()), [BRAKES_FILE], "a temp file was left");

    save(dir.path(), None, WAIT).await.unwrap();
    assert_eq!(on_disk(dir.path()), json!({"v": 1, "hold": null}));
    assert_eq!(names(dir.path()), [BRAKES_FILE]);
    assert_eq!(load(dir.path(), WAIT, NOW).await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_hold_loads_back() {
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), Some(user_hold()), WAIT).await.unwrap();
    assert_eq!(load(dir.path(), WAIT, NOW).await, Some(user_hold()));
}

/// The moved-aside names in `dir`.
fn corrupt_names(dir: &Path) -> Vec<String> {
    let prefix = format!("{BRAKES_FILE}.corrupt-");
    names(dir)
        .into_iter()
        .filter(|n| n.starts_with(&prefix))
        .collect()
}

async fn corrupt_case(text: &str) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(BRAKES_FILE), text).unwrap();
    let held = load(dir.path(), WAIT, NOW).await;
    assert_eq!(
        held,
        Some(DispatchHold {
            since: NOW,
            by: CORRUPT_BY.into()
        }),
        "{text}"
    );
    assert_eq!(CORRUPT_BY, "corrupt brakes.json");
    let moved = corrupt_names(dir.path());
    assert_eq!(moved.len(), 1, "{:?}", names(dir.path()));
    let suffix = moved[0].rsplit('-').next().unwrap();
    let unix: u64 = suffix.parse().expect("brakes.json.corrupt-<unix>");
    assert!(unix >= NOW, "{unix}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(&moved[0])).unwrap(),
        text
    );
    assert!(!dir.path().join(BRAKES_FILE).exists());
    // A new save works beside the moved file.
    save(dir.path(), None, WAIT).await.unwrap();
    assert_eq!(on_disk(dir.path()), json!({"v": 1, "hold": null}));
}

#[tokio::test(flavor = "multi_thread")]
async fn corrupt_brakes_file_is_moved_aside_and_holds() {
    corrupt_case("{").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_version_is_corrupt() {
    corrupt_case(r#"{"v":2,"hold":null}"#).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_file_is_off() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load(dir.path(), WAIT, NOW).await, None);
    assert!(names(dir.path()).is_empty(), "{:?}", names(dir.path()));
}

/// Decision 16's read that does not finish in time: the path is a FIFO nobody writes
/// to, so the read blocks; with the wait injected as 200 ms the load holds, and the
/// FIFO is left where it is. A writer thread opens the FIFO once the read is blocked
/// in `open` and holds it until the test ends, so the read's thread is released
/// (ruling C26; the writer gives up after the wait × 100).
#[tokio::test(flavor = "multi_thread")]
async fn unreadable_file_holds_and_stays() {
    use std::os::unix::fs::OpenOptionsExt;
    let wait = Duration::from_millis(200);
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join(BRAKES_FILE);
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let held = fifo.clone();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + wait * 100;
        let file = loop {
            let opened = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&held);
            match opened {
                Ok(file) => break file,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => {
                    let mut rdwr = std::fs::OpenOptions::new();
                    rdwr.read(true).write(true);
                    break rdwr.open(&held).expect("a FIFO opens O_RDWR");
                }
            }
        };
        let _ = released.recv_timeout(wait * 100);
        drop(file);
    });

    let loaded = load(dir.path(), wait, NOW).await;
    assert_eq!(
        loaded,
        Some(DispatchHold {
            since: NOW,
            by: UNREADABLE_BY.into()
        })
    );
    assert_eq!(UNREADABLE_BY, "unreadable brakes.json");
    let meta = std::fs::symlink_metadata(&fifo).expect("the FIFO is still there");
    assert!(std::os::unix::fs::FileTypeExt::is_fifo(&meta.file_type()));
    assert!(corrupt_names(dir.path()).is_empty(), "the FIFO was renamed");
    let _ = release.send(());
    tokio::task::spawn_blocking(move || writer.join().unwrap())
        .await
        .unwrap();
    // The read's own thread, released, still renames nothing.
    assert!(corrupt_names(dir.path()).is_empty(), "the FIFO was renamed");
}

/// A save into a directory nobody may write in fails and leaves no file.
#[tokio::test(flavor = "multi_thread")]
async fn a_save_that_cannot_write_fails() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o555)).unwrap();
    // A process that may write anyway (root) cannot test this.
    if std::fs::write(data.join("probe"), b"").is_ok() {
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let saved = save(&data, Some(user_hold()), WAIT).await;
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(saved.is_err(), "{saved:?}");
    assert!(names(&data).is_empty(), "{:?}", names(&data));
}
