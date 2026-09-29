//! Milestone 9.1 task M9.1.6: decision 22's affected set.

use std::collections::BTreeSet;

use proto::ModuleNames;

use super::affected::affected;
use super::graph::from_cargo_metadata;
use super::steps::plan;
use super::*;

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| x.to_string()).collect()
}

fn set(items: &[&str]) -> Affected {
    Affected::Modules(items.iter().map(|x| x.to_string()).collect::<BTreeSet<_>>())
}

fn full(reason: &str) -> Affected {
    Affected::Full(reason.to_string())
}

/// `mods/a`, `mods/b` (depends on `a`), `mods/c` (depends on `b`) and `mods/d`, which
/// nothing depends on and which depends on nothing.
fn graph() -> GraphState {
    let mut graph = ModuleGraph::default();
    for (name, deps) in [
        ("a", vec![]),
        ("b", vec!["a"]),
        ("c", vec!["b"]),
        ("d", vec![]),
    ] {
        graph.modules.insert(
            name.to_string(),
            ModuleInfo {
                dir: Some(format!("mods/{name}")),
                deps: deps.into_iter().map(str::to_string).collect(),
            },
        );
    }
    GraphState::Known(graph)
}

fn tiers() -> TierProfile {
    TierProfile {
        build_check: Some("sh build.sh".into()),
        module_test: Some("sh test.sh {module} {filter:--filter %}".into()),
        full_triggers: s(&["*.lock", "ci/**"]),
        test_paths: s(&["tests/**"]),
        ..TierProfile::default()
    }
}

/// The affected set of `changed` in the four-module repository, with `hub` and `source`.
fn of(changed: &[&str], hub: &[&str], source: &[&str]) -> Affected {
    affected(
        &s(changed),
        &tiers(),
        &s(hub),
        &s(source),
        &s(&["mods/*"]),
        &graph(),
    )
}

const SOURCE: &[&str] = &["mods/**", "scripts/**"];

fn anthrex() -> GraphState {
    GraphState::Known(
        from_cargo_metadata(include_str!("fixtures/cargo-metadata-anthrex.json")).unwrap(),
    )
}

fn cargo_named(changed: &[&str]) -> Affected {
    let tiers = TierProfile {
        module_graph: GraphSource::Cargo,
        module_names: ModuleNames::Cargo,
        ..tiers()
    };
    affected(
        &s(changed),
        &tiers,
        &[],
        &s(&["crates/**"]),
        &s(&["crates/*"]),
        &anthrex(),
    )
}

#[test]
fn affected_leaf_crate_is_itself() {
    assert_eq!(of(&["mods/c/src/lib.rs"], &[], SOURCE), set(&["c"]));
    assert_eq!(
        of(&["mods/d/x.rs", "mods/d/y.rs"], &[], SOURCE),
        set(&["d"])
    );
    // Cargo names: the graph's package whose manifest directory is the module's.
    assert_eq!(cargo_named(&["crates/cli/src/main.rs"]), set(&["anthrex"]));
}

#[test]
fn affected_crate_with_dependents_includes_them_transitively() {
    assert_eq!(
        of(&["mods/a/src/lib.rs"], &[], SOURCE),
        set(&["a", "b", "c"])
    );
    assert_eq!(of(&["mods/b/x"], &[], SOURCE), set(&["b", "c"]));
    // A dev-dependency is an edge: the daemon dev-depends on mcp, and fake-agent
    // dev-depends on the daemon; config and proto do not depend on mcp.
    assert_eq!(
        cargo_named(&["crates/mcp/src/lib.rs"]),
        set(&[
            "anthrex",
            "anthrex-daemon",
            "anthrex-fake-agent",
            "anthrex-mcp",
            "anthrex-tui"
        ])
    );
    // Cycles are allowed.
    let mut cyclic = ModuleGraph::default();
    for (name, dep) in [("a", "b"), ("b", "a"), ("c", "c")] {
        cyclic.modules.insert(
            name.to_string(),
            ModuleInfo {
                dir: None,
                deps: [dep.to_string()].into(),
            },
        );
    }
    assert_eq!(
        affected(
            &s(&["mods/a/x"]),
            &tiers(),
            &[],
            &[],
            &s(&["mods/*"]),
            &GraphState::Known(cyclic)
        ),
        set(&["a", "b"])
    );
}

#[test]
fn affected_hub_file_is_full() {
    assert_eq!(
        of(&["mods/c/x", "mods/a/lib.rs"], &["mods/a/lib.rs"], SOURCE),
        full("hub file: mods/a/lib.rs")
    );
    // A hub entry with no wildcard covers everything below it, as `owns` does.
    assert_eq!(
        of(&["mods/b/proto/x.rs"], &["mods/b/proto"], SOURCE),
        full("hub file: mods/b/proto/x.rs")
    );
}

#[test]
fn affected_full_trigger_is_full() {
    assert_eq!(
        of(&["mods/c/x", "Cargo.lock"], &[], SOURCE),
        full("full trigger: Cargo.lock")
    );
    assert_eq!(
        of(&["ci/deep/workflow.yml"], &[], SOURCE),
        full("full trigger: ci/deep/workflow.yml")
    );
    // A trigger is checked before the hub for the same path.
    assert_eq!(
        of(&["x.lock"], &["x.lock"], SOURCE),
        full("full trigger: x.lock")
    );
}

#[test]
fn affected_unowned_path_is_full() {
    // In no module, but matching `source`: something executes it.
    assert_eq!(
        of(&["mods/a/x", "scripts/run.sh"], &[], SOURCE),
        full("unowned path scripts/run.sh")
    );
    // In no module and not in `source`, but matching `test_paths`.
    assert_eq!(
        of(&["tests/e2e.rs"], &[], SOURCE),
        full("unowned path tests/e2e.rs")
    );
    // A module directory the graph has no package for.
    assert_eq!(
        cargo_named(&["crates/newcrate/src/lib.rs"]),
        full("no package for module crates/newcrate")
    );
    // A directory-named module the graph does not list.
    assert_eq!(
        of(&["mods/e/x"], &[], SOURCE),
        full("module e is not in the module graph")
    );
}

#[test]
fn affected_docs_only_change_is_empty_and_plans_build_only() {
    let docs = of(&["docs/guide.md", "README.md"], &[], SOURCE);
    assert_eq!(docs, set(&[]));
    let plan = plan(1, &docs, &tiers(), Some("sh check.sh"));
    assert_eq!(plan.scope, Scope::Gate);
    assert_eq!(
        plan.steps,
        vec![Step {
            kind: StepKind::Build,
            command: "sh build.sh".into(),
            exclusive: false,
            affected_key: "-".into(),
        }]
    );
    // An ignored path next to a module path leaves only the module.
    assert_eq!(of(&["docs/guide.md", "mods/c/x"], &[], SOURCE), set(&["c"]));
}

#[test]
fn affected_every_module_is_full() {
    assert_eq!(
        of(&["mods/a/x", "mods/d/y"], &[], SOURCE),
        full("every module affected")
    );
    // proto: every crate depends on it, fake-agent through a dev-dependency.
    assert_eq!(
        cargo_named(&["crates/proto/src/lib.rs"]),
        full("every module affected")
    );
}

#[test]
fn unknown_graph_is_full_and_plans_check() {
    let got = affected(
        &s(&["mods/a/x"]),
        &tiers(),
        &[],
        &s(SOURCE),
        &s(&["mods/*"]),
        &GraphState::Unknown("module graph: x is not a module".into()),
    );
    assert_eq!(
        got,
        full("module graph unknown: module graph: x is not a module")
    );
    let plan = plan(1, &got, &tiers(), Some("sh check.sh"));
    assert_eq!(
        plan.steps,
        vec![Step {
            kind: StepKind::Tests,
            command: "sh check.sh".into(),
            exclusive: false,
            affected_key: "full:module graph unknown: module graph: x is not a module".into(),
        }]
    );
}

#[test]
fn empty_source_never_ignores_a_path() {
    assert_eq!(
        of(&["docs/guide.md"], &[], &[]),
        full("unowned path docs/guide.md")
    );
    assert_eq!(of(&["docs/guide.md"], &[], &["mods/**"]), set(&[]));
}

#[test]
fn first_rule_wins_and_names_the_first_path() {
    // Rule 1 beats an unowned path and an unknown graph, and names the first path in
    // sorted order, not in the order given.
    let got = affected(
        &s(&["zz/unowned", "b.lock", "Cargo.lock", "mods/a/x"]),
        &tiers(),
        &[],
        &s(SOURCE),
        &s(&["mods/*"]),
        &GraphState::Unknown("none".into()),
    );
    assert_eq!(got, full("full trigger: Cargo.lock"));
    // Rule 2 beats rule 3.
    let got = affected(
        &s(&["scripts/z.sh", "scripts/a.sh", "mods/a/x"]),
        &tiers(),
        &[],
        &s(SOURCE),
        &s(&["mods/*"]),
        &GraphState::Unknown("none".into()),
    );
    assert_eq!(got, full("unowned path scripts/a.sh"));
    // A hub path sorted before a trigger path is named, both being rule 1.
    assert_eq!(
        of(&["Cargo.lock", "Build.rs"], &["Build.rs"], SOURCE),
        full("hub file: Build.rs")
    );
}

#[test]
fn invalid_globs_are_full_never_ignored() {
    // A broken `source` glob must not make every unowned path look ignorable, nor
    // match nothing: the whole set is full.
    let got = of(&["docs/guide.md"], &[], &["src/[", "src/**"]);
    let Affected::Full(reason) = got else {
        panic!("expected full, got {got:?}");
    };
    assert!(reason.starts_with("source globs are invalid: "), "{reason}");
    // A broken hub.
    let got = of(&["mods/a/x"], &["mods/[a"], SOURCE);
    let Affected::Full(reason) = got else {
        panic!("expected full, got {got:?}");
    };
    assert!(reason.starts_with("hub globs are invalid: "), "{reason}");
    // A broken full trigger and test path, too.
    let broken = TierProfile {
        full_triggers: s(&["[x"]),
        ..tiers()
    };
    let got = affected(
        &s(&["mods/a/x"]),
        &broken,
        &[],
        &s(SOURCE),
        &s(&["mods/*"]),
        &graph(),
    );
    assert!(
        matches!(&got, Affected::Full(r) if r.starts_with("full_triggers globs are invalid: ")),
        "{got:?}"
    );
    let broken = TierProfile {
        test_paths: s(&["[x"]),
        ..tiers()
    };
    let got = affected(
        &s(&["mods/a/x"]),
        &broken,
        &[],
        &s(SOURCE),
        &s(&["mods/*"]),
        &graph(),
    );
    assert!(
        matches!(&got, Affected::Full(r) if r.starts_with("test_paths globs are invalid: ")),
        "{got:?}"
    );
}

#[test]
fn docs_only_change_on_an_empty_graph_is_empty() {
    let got = affected(
        &s(&["docs/guide.md"]),
        &tiers(),
        &[],
        &s(SOURCE),
        &s(&["mods/*"]),
        &GraphState::Known(ModuleGraph::default()),
    );
    assert_eq!(got, set(&[]));
}
