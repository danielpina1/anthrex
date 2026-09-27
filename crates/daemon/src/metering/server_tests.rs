//! M8b.15 re-review: the receiver's metering under the ledger lock, and its address
//! file.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::TokenUsage;

use super::*;
use crate::metering::otlp::UsageKind;

type Hook = Box<dyn FnOnce() + Send>;

/// A sink whose `is_live("r")` can run a hook once, after answering.
#[derive(Default)]
struct Racing {
    live: Mutex<HashSet<String>>,
    generation: AtomicU64,
    hook: Mutex<Option<Hook>>,
}

impl UsageSink for Racing {
    fn is_live(&self, run_id: &str) -> bool {
        let live = crate::lock(&self.live).contains(run_id);
        if run_id == "r" {
            let hook = crate::lock(&self.hook).take();
            if let Some(hook) = hook {
                hook();
            }
        }
        live
    }

    fn live_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn post(&self, _: String, _: TokenUsage) {}
}

fn point(run_id: &str, value: u64) -> UsagePoint {
    UsagePoint {
        run_id: run_id.into(),
        role: ORCHESTRATOR.into(),
        session_id: "s".into(),
        model: "m".into(),
        kind: UsageKind::Input,
        value,
        cumulative: false,
    }
}

fn input(n: u64) -> TokenUsage {
    TokenUsage {
        input: n,
        ..TokenUsage::default()
    }
}

/// Review Important 1: request A reads run `r` live, `r` ends, request B evicts it,
/// then A applies. `r`'s total must not be rebuilt from A's points alone, which would
/// lower its stored usage.
#[test]
fn a_run_that_ends_while_a_request_is_metered_is_never_rebuilt_lower() {
    let sink = Arc::new(Racing::default());
    crate::lock(&sink.live).extend(["r".to_string(), "other".to_string()]);
    let shared = Arc::new(Shared {
        ledger: Mutex::new(Ledger::default()),
        sink: sink.clone(),
        logged_type: AtomicBool::new(false),
    });
    assert_eq!(
        record(&shared, vec![point("r", 100)]),
        vec![("r".to_string(), input(100))]
    );
    // While A checks `r`: `r` ends, and B, another connection's request, is metered.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let (hook_sink, hook_shared) = (sink.clone(), shared.clone());
    let b = Arc::new(Mutex::new(None));
    let b_slot = b.clone();
    *crate::lock(&sink.hook) = Some(Box::new(move || {
        crate::lock(&hook_sink.live).remove("r");
        hook_sink.generation.fetch_add(1, Ordering::SeqCst);
        let handle = std::thread::spawn(move || {
            let totals = record(&hook_shared, vec![point("other", 1)]);
            let _ = done_tx.send(());
            totals
        });
        // Under the fix B waits for A's ledger lock, so this only bounds the wait; before
        // it, B finishes here, between A's filter and A's apply.
        let _ = done_rx.recv_timeout(Duration::from_millis(300));
        *crate::lock(&b_slot) = Some(handle);
    }));
    let a = record(&shared, vec![point("r", 5)]);
    let b = crate::lock(&b)
        .take()
        .expect("the hook ran")
        .join()
        .unwrap();
    assert_eq!(b, vec![("other".to_string(), input(1))]);
    for (run, total) in &a {
        assert!(
            run != "r" || total.input >= 100,
            "r's usage went down to {total:?}"
        );
    }
    // B evicted the ended run once A was done.
    assert_eq!(
        crate::lock(&shared.ledger).ledger.total("r", ORCHESTRATOR),
        TokenUsage::default()
    );
}

/// Review Minor 2 (mutant G): a temporary file an earlier daemon left behind does not
/// stop the address file being written.
#[test]
fn a_stale_temporary_address_file_is_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(ADDR_FILE);
    std::fs::write(path.with_extension("addr.tmp"), "stale").unwrap();
    write_addr(&path, "http://127.0.0.1:1").expect("the address file is written");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "http://127.0.0.1:1"
    );
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert!(!path.with_extension("addr.tmp").exists());
}

/// A sink that records every post, and whose first post of run `r` can run a hook
/// before it is recorded.
#[derive(Default)]
struct Recording {
    posts: Mutex<Vec<(String, TokenUsage)>>,
    hook: Mutex<Option<Hook>>,
}

impl UsageSink for Recording {
    fn is_live(&self, _: &str) -> bool {
        true
    }

    fn live_generation(&self) -> u64 {
        0
    }

    fn post(&self, run_id: String, usage: TokenUsage) {
        if run_id == "r" {
            let hook = crate::lock(&self.hook).take();
            if let Some(hook) = hook {
                hook();
            }
        }
        crate::lock(&self.posts).push((run_id, usage));
    }
}

/// Fix round 2 ruling: two requests for one run post their totals in the order the
/// ledger computed them, so the latest post, the one the engine keeps, is the higher.
#[test]
fn two_requests_for_one_run_post_their_totals_in_order() {
    let sink = Arc::new(Recording::default());
    let shared = Arc::new(Shared {
        ledger: Mutex::new(Ledger::default()),
        sink: sink.clone(),
        logged_type: AtomicBool::new(false),
    });
    // As A posts its total, B, another connection's request for the same run, is
    // recorded.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let hook_shared = shared.clone();
    let b = Arc::new(Mutex::new(None));
    let b_slot = b.clone();
    *crate::lock(&sink.hook) = Some(Box::new(move || {
        let handle = std::thread::spawn(move || {
            record(&hook_shared, vec![point("r", 5)]);
            let _ = done_tx.send(());
        });
        // Under the fix B waits for A's ledger lock, so this only bounds the wait; before
        // it, B computes and posts here, between A's total and A's post.
        let _ = done_rx.recv_timeout(Duration::from_millis(300));
        *crate::lock(&b_slot) = Some(handle);
    }));
    assert_eq!(
        record(&shared, vec![point("r", 100)]),
        vec![("r".to_string(), input(100))]
    );
    crate::lock(&b)
        .take()
        .expect("the hook ran")
        .join()
        .unwrap();
    let posts = crate::lock(&sink.posts).clone();
    assert_eq!(
        posts,
        vec![("r".to_string(), input(100)), ("r".to_string(), input(105))],
        "the posts arrived out of order"
    );
}
