//! Milestone 9.1 task M9.1.14: tier 3 in the run-level `.full` scratch checkout, its
//! shards as separate steps (decision 18), on the rig of `tier_tests.rs`.

use super::*;

#[tokio::test]
async fn shards_run_as_separate_steps() {
    let rig = Rig::new();
    let full = rig.dir.with_file_name(".full");
    let mut spec = rig.spec(
        TierProfile {
            full_shards: 2,
            ..tiered()
        },
        Some("sh check.sh {shard} {shards}"),
    );
    spec.tier = 3;
    spec.dir = full.clone();
    spec.priority = Priority::FullStage;
    spec.diff_base = String::new();
    let outcome = rig.tier(9, &spec).await;
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(outcome.scope, Scope::Full);
    assert_eq!(outcome.affected, Affected::Full("full suite".to_string()));
    let steps: Vec<(StepKind, u32, bool)> = outcome
        .steps
        .iter()
        .map(|s| (s.kind, s.granted, s.cached))
        .collect();
    assert_eq!(
        steps,
        [
            (StepKind::Shard { k: 1, of: 2 }, 2, false),
            (StepKind::Shard { k: 2, of: 2 }, 2, false),
        ]
    );
    let log = rig.log();
    let runs: Vec<(String, String, String)> = log
        .iter()
        .map(|l| {
            let args = l
                .strip_prefix("check args=")
                .and_then(|a| a.split(" pwd=").next())
                .unwrap_or_default()
                .to_string();
            (
                args,
                value(l, "pwd").to_string(),
                value(l, "slots").to_string(),
            )
        })
        .collect();
    let at = full.display().to_string();
    assert_eq!(
        runs,
        [
            ("1 2".to_string(), at.clone(), "2".to_string()),
            ("2 2".to_string(), at, "2".to_string()),
        ],
        "{log:#?}"
    );
    // Prepared at the stage head, as `<task>.proof` is.
    assert_eq!(git(&full, &["rev-parse", "HEAD"]), rig.head);
}
