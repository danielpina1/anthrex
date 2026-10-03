//! Milestone 9.5 review ruling I6: the installed probe is bounded, and a probe that gave
//! up, or found nothing installed, leaves the route as before. Timing in
//! `docs/timing-budgets.md` (M9.5.10b): the bound is the test's own timeout (50 ms),
//! the probe's block ten times it.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::*;

const TIMEOUT: Duration = Duration::from_millis(50);

fn map(claude: bool, codex: bool) -> BTreeMap<String, bool> {
    [("claude".to_string(), claude), ("codex".to_string(), codex)].into()
}

fn found_with(headless: BTreeMap<String, bool>) -> Found {
    Found {
        window: headless.clone(),
        headless,
    }
}

#[tokio::test]
async fn the_installed_probe_gives_up_after_its_timeout() {
    let started = Instant::now();
    let slow = move || {
        std::thread::sleep(TIMEOUT * 10);
        found_with(map(true, true))
    };
    let got = within(slow, TIMEOUT).await;
    assert!(got.is_none(), "the probe gave up");
    assert!(
        started.elapsed() < TIMEOUT * 10,
        "it did not wait for the probe: {:?}",
        started.elapsed()
    );
    // The caller resolves as today: every runtime counts as installed.
    assert!(installed_of(got).is_empty());

    // A probe that answers in time gives its headless map.
    let quick = || found_with(map(false, true));
    let got = within(quick, Duration::from_secs(5)).await;
    assert_eq!(installed_of(got), map(false, true));
    // Nothing installed: as today too.
    assert!(installed_of(Some(found_with(map(false, false)))).is_empty());
}

#[tokio::test]
async fn found_within_stats_the_configured_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let codex = dir.path().join("codex");
    std::fs::write(&codex, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&codex, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let claude = dir.path().join("claude").display().to_string();
    let codex = codex.display().to_string();
    let got = found_within(claude.clone(), codex.clone(), INSTALLED_PROBE_TIMEOUT).await;
    assert_eq!(got.map(|f| f.headless), Some(map(false, true)));
    assert_eq!(installed_now(claude, codex).await, map(false, true));
}
