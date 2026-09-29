//! Milestone 9.1 task M9.1.6: decision 30's cache key.

use super::cache_key::key;
use super::steps::plan;
use super::*;

fn ctx() -> CacheCtx {
    CacheCtx {
        profile_hash: "0123456789abcdef".into(),
        toolchain: "none".into(),
    }
}

fn profile() -> TierProfile {
    TierProfile {
        build_check: Some("cargo build".into()),
        module_tests: Some("cargo test {modules:-p %}".into()),
        ..TierProfile::default()
    }
}

fn modules(names: &[&str]) -> Affected {
    Affected::Modules(names.iter().map(|x| x.to_string()).collect())
}

/// Each step's key for `tree`.
fn keys(tier: u8, affected: &Affected, tree: &str) -> Vec<cache_key::CacheKey> {
    let plan = plan(tier, affected, &profile(), Some("cargo test"));
    plan.steps
        .iter()
        .map(|step| key(tree, plan.scope, &step.command, &step.affected_key, &ctx()))
        .collect()
}

#[test]
fn cache_key_uses_scope_not_tier() {
    let set = modules(&["a", "b"]);
    let tier1 = keys(1, &set, "t1");
    assert_eq!(tier1.len(), 2);
    assert_eq!(tier1, keys(2, &set, "t1"));
    let tests = &tier1[1];
    assert_eq!(tests.tree, "t1");
    assert_eq!(tests.scope, Scope::Gate);
    assert_eq!(tests.command, "cargo test -p 'a' -p 'b'");
    assert_eq!(tests.affected, "a,b");
    assert_eq!(tests.profile_hash, "0123456789abcdef");
    assert_eq!(tests.toolchain, "none");
    // Tier 3's scope is `full`: the same command's key differs.
    let full = keys(3, &set, "t1");
    assert_eq!(full[0].command, "cargo test");
    assert_eq!(full[0].scope, Scope::Full);
    assert_ne!(full[0], key("t1", Scope::Gate, "cargo test", "a,b", &ctx()));
    // Every part counts.
    let base = key("t", Scope::Gate, "c", "a", &ctx());
    let other = CacheCtx {
        toolchain: "unknown".into(),
        ..ctx()
    };
    for changed in [
        key("u", Scope::Gate, "c", "a", &ctx()),
        key("t", Scope::Full, "c", "a", &ctx()),
        key("t", Scope::Gate, "d", "a", &ctx()),
        key("t", Scope::Gate, "c", "b", &ctx()),
        key("t", Scope::Gate, "c", "a", &other),
        key(
            "t",
            Scope::Gate,
            "c",
            "a",
            &CacheCtx {
                profile_hash: "x".into(),
                ..ctx()
            },
        ),
    ] {
        assert_ne!(changed, base);
    }
}

#[test]
fn build_step_key_ignores_the_affected_set() {
    let one = keys(1, &modules(&["a"]), "t1");
    let two = keys(2, &modules(&["a", "b"]), "t1");
    let docs = keys(1, &modules(&[]), "t1");
    assert_eq!(one[0].affected, "-");
    assert_eq!(one[0], two[0]);
    assert_eq!(one[0], docs[0]);
    // The tests steps differ.
    assert_ne!(one[1], two[1]);
}
