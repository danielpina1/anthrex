//! Milestone 9.13 decision 12, ruling C5 (task M9.13.6): `run_toolchain` and
//! `read_graph` run their commands outside `run_isolated_with`, so each checks the run's
//! pinned gate scripts inside its own blocking closure, before its command; a refusal
//! stops the tier job with `OpResult::GateRefused`. On the rig of `tier_tests.rs`.

use std::sync::Arc;

use super::*;
use crate::run::gate_words::PinnedFile;

/// `path` as the rig's base commit holds it, after `text` was committed there; then a
/// later commit rewrites it to touch `$TIER_STATE/ran-<path>`. The pinned file, and the
/// rewriting commit.
fn rewritten(rig: &Rig, path: &str, text: &str) -> (PinnedFile, String) {
    write(&rig.root.join(path), text);
    git(&rig.root, &["add", path]);
    git(&rig.root, &["commit", "-q", "-m", "add the script"]);
    let blob = git(&rig.root, &["rev-parse", &format!("HEAD:{path}")]);
    let touch = format!("touch \"$TIER_STATE/ran-{path}\"\n{text}");
    write(&rig.root.join(path), &touch);
    git(&rig.root, &["commit", "-q", "-am", "rewrite the script"]);
    let head = git(&rig.root, &["rev-parse", "HEAD"]);
    let pinned = PinnedFile {
        path: path.to_string(),
        blob,
    };
    (pinned, head)
}

async fn refused(rig: &Rig, spec: &TierSpec, pinned: PinnedFile) -> OpResult {
    let mut ctx = rig.ctx();
    ctx.pinned = Arc::from(vec![pinned]);
    let (sched, queue) = (TestScheduler::new(2), GitQueue::new());
    let cache = TestCache::new(0);
    run_tier(&ctx, &sched, &cache, &queue, OsStr::new("git"), 41, spec).await
}

#[tokio::test]
async fn a_rewritten_toolchain_script_stops_the_job() {
    let rig = Rig::new();
    let (pinned, head) = rewritten(&rig, "tc.sh", "echo toolchain-1\n");
    let profile = TierProfile {
        toolchain_id: Some("sh tc.sh".to_string()),
        module_graph: GraphSource::None,
        ..tiered()
    };
    let mut spec = rig.spec(profile, None);
    spec.toolchain = None;
    spec.head = head.clone();
    spec.scratch = Some(ScratchAt {
        root: rig.root.clone(),
        commit: head,
        setup: None,
    });
    let result = refused(&rig, &spec, pinned).await;
    assert_eq!(
        result,
        OpResult::GateRefused {
            path: "tc.sh".to_string()
        }
    );
    assert!(!rig.state.join("ran-tc.sh").exists(), "the toolchain ran");
    assert!(rig.log().is_empty(), "a step ran: {:?}", rig.log());
}

#[tokio::test]
async fn a_rewritten_graph_script_stops_the_job() {
    let rig = Rig::new();
    let (pinned, head) = rewritten(&rig, "graph2.sh", GRAPH);
    let profile = TierProfile {
        module_graph: GraphSource::Command("sh graph2.sh".to_string()),
        ..tiered()
    };
    let mut spec = rig.spec(profile, None);
    spec.head = head.clone();
    spec.scratch = Some(ScratchAt {
        root: rig.root.clone(),
        commit: head,
        setup: None,
    });
    let result = refused(&rig, &spec, pinned).await;
    assert_eq!(
        result,
        OpResult::GateRefused {
            path: "graph2.sh".to_string()
        }
    );
    assert!(!rig.state.join("ran-graph2.sh").exists(), "the graph ran");
    assert!(rig.log().is_empty(), "a step ran: {:?}", rig.log());
}
