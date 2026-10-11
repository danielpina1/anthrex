//! Milestone 9.11 decision 12 (M9.11.11, A9): a wake-up's `\r` is held when the user
//! typed into the orchestrator window between its paste and its submit. Against a real
//! manager and a real shell window (never an agent).

use std::path::Path;
use std::time::{Duration, Instant};

use proto::{Runtime, WindowSpec};

use super::*;
use crate::launch::LaunchGate;
use crate::manager::{ManagerConfig, WindowManager};

/// The workspace's allowance for a loaded machine (`docs/timing-budgets.md`).
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(5);
/// Long enough that the test's own input lands inside it by construction: the test
/// types only after it has read the paste back, which takes a writer-thread write, the
/// shell's `cat` and one 10 ms poll.
const LONG_DELAY: Duration = Duration::from_secs(10);

/// A manager, its events pumped, and a shell window in `dir` whose input, once
/// `ready` exists, goes raw and unechoed into `dir/got`.
async fn raw_window(dir: &Path) -> (Arc<WindowManager>, u32) {
    let mut config = ManagerConfig::for_tests(dir.join("d.sock"), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.worktrees_root = dir.join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let spec = WindowSpec {
        name: Some("wake-submit".into()),
        runtime: Runtime::Shell,
        cwd: dir.to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let info = manager
        .create(spec, dir.to_path_buf(), None, 80, 24)
        .await
        .expect("a shell window");
    // Raw mode is set before `ready` is made, so every byte written after it reaches
    // `cat` unchanged: no echo, no line discipline, and a `\r` stays a `\r`.
    let command = "stty raw -echo && : > ready && exec cat > got\r";
    manager.write_input(info.id, command.as_bytes()).unwrap();
    wait_for("the raw window", || dir.join("ready").exists()).await;
    (manager, info.id)
}

async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WALL_CLOCK_SLACK;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn got(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join("got")).unwrap_or_default()
}

fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_inside_the_submit_delay_leaves_the_paste_unsubmitted() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, id) = raw_window(dir.path()).await;
    let paste = encode_paste("[anthrex] run r1 changed");
    let delivering = manager.clone();
    let sent = paste.clone();
    let delivery =
        tokio::spawn(async move { deliver_wake_with(&delivering, id, &sent, LONG_DELAY).await });
    wait_for("the paste", || holds(&got(dir.path()), &paste)).await;
    // Inside the delay by construction: the delivery is still asleep.
    assert!(!delivery.is_finished());
    manager.write_client_input(id, b"x").unwrap();
    let delivered = tokio::time::timeout(LONG_DELAY + WALL_CLOCK_SLACK, delivery)
        .await
        .expect("the delivery returns")
        .unwrap();
    assert_eq!(delivered.unwrap(), Delivery::Unsubmitted);
    wait_for("the user's key", || got(dir.path()).ends_with(b"x")).await;
    let seen = got(dir.path());
    assert!(holds(&seen, &paste), "{seen:?}");
    assert!(!seen.contains(&b'\r'), "the paste was submitted: {seen:?}");
    let _ = manager.kill(id);
}

/// Pinning: with no input during the delay the paste is submitted, as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_input_submits() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, id) = raw_window(dir.path()).await;
    let paste = encode_paste("[anthrex] run r1 changed");
    let delivered = tokio::time::timeout(
        SUBMIT_DELAY + WALL_CLOCK_SLACK,
        deliver_wake_with(&manager, id, &paste, SUBMIT_DELAY),
    )
    .await
    .expect("the delivery returns");
    assert_eq!(delivered.unwrap(), Delivery::Submitted);
    let mut submitted = paste.clone();
    submitted.push(b'\r');
    wait_for("the submit", || got(dir.path()) == submitted).await;
    let _ = manager.kill(id);
}
