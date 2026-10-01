//! Milestone 9.1 task M9.1.6: decision 9's module graphs, from `cargo metadata` and
//! from a graph command's JSON.

use std::collections::{BTreeMap, BTreeSet};

use super::graph::{from_cargo_metadata, from_command_json};
use super::*;

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|x| x.to_string()).collect()
}

fn module(graph: &ModuleGraph, name: &str) -> (Option<String>, BTreeSet<String>) {
    let info = &graph.modules[name];
    (info.dir.clone(), info.deps.clone())
}

#[test]
fn cargo_metadata_keeps_workspace_members_and_every_dependency_kind() {
    // Captured from anthrex itself (M9.1.1 check 6): seven members.
    let graph = from_cargo_metadata(include_str!("fixtures/cargo-metadata-anthrex.json")).unwrap();
    let all = [
        "anthrex",
        "anthrex-config",
        "anthrex-daemon",
        "anthrex-fake-agent",
        "anthrex-mcp",
        "anthrex-proto",
        "anthrex-tui",
    ];
    assert_eq!(
        graph.modules.keys().cloned().collect::<BTreeSet<_>>(),
        names(&all)
    );
    // `anthrex` depends on every crate but `anthrex-fake-agent`.
    let every_but_fake: Vec<&str> = all
        .iter()
        .copied()
        .filter(|n| !matches!(*n, "anthrex" | "anthrex-fake-agent"))
        .collect();
    assert_eq!(
        module(&graph, "anthrex"),
        (Some("crates/cli".into()), names(&every_but_fake))
    );
    // `anthrex-fake-agent`'s dev-dependencies are edges.
    assert_eq!(
        module(&graph, "anthrex-fake-agent"),
        (
            Some("crates/fake-agent".into()),
            names(&["anthrex-config", "anthrex-daemon", "anthrex-proto"])
        )
    );
    assert_eq!(
        module(&graph, "anthrex-daemon"),
        (
            Some("crates/daemon".into()),
            names(&["anthrex-config", "anthrex-mcp", "anthrex-proto"])
        )
    );
    // A crate named both as a normal and a dev-dependency is one edge; registry
    // dependencies are not modules.
    assert_eq!(
        module(&graph, "anthrex-mcp"),
        (Some("crates/mcp".into()), names(&["anthrex-proto"]))
    );
    assert_eq!(
        module(&graph, "anthrex-proto"),
        (Some("crates/proto".into()), names(&[]))
    );

    // Captured (M9.1.1 check 6): a renamed dependency keeps its package name, and a
    // build-dependency is an edge too.
    let three =
        from_cargo_metadata(include_str!("fixtures/cargo-metadata-three-crates.json")).unwrap();
    assert_eq!(
        module(&three, "ax-c"),
        (Some("crates/c".into()), names(&["ax-a", "ax-b"]))
    );
    assert_eq!(
        module(&three, "ax-b"),
        (Some("crates/b".into()), names(&["ax-a"]))
    );

    // A package that is not a workspace member is left out, and a root package's
    // directory is `.`.
    let json = r#"{"workspace_root":"/w","workspace_members":["r"],"packages":[
        {"id":"r","name":"root","manifest_path":"/w/Cargo.toml","dependencies":[{"name":"x"}]},
        {"id":"x","name":"x","manifest_path":"/w/x/Cargo.toml","dependencies":[]}]}"#;
    let graph = from_cargo_metadata(json).unwrap();
    assert_eq!(graph.modules.len(), 1);
    assert_eq!(module(&graph, "root"), (Some(".".into()), names(&[])));
    assert!(from_cargo_metadata("not json").is_err());
}

#[test]
fn command_graph_with_an_unknown_dep_is_unknown() {
    let dirs: BTreeMap<String, String> = [("a".to_string(), "mods/a".to_string())].into();
    let graph = from_command_json(r#" {"a": [], "b": ["a"], "c": ["b", "a"]} "#, &dirs).unwrap();
    assert_eq!(module(&graph, "a"), (Some("mods/a".into()), names(&[])));
    assert_eq!(module(&graph, "c"), (None, names(&["a", "b"])));
    assert_eq!(
        from_command_json(r#"{"a": [], "b": ["z"]}"#, &dirs),
        Err("z is not a module".to_string())
    );
    assert_eq!(
        from_command_json(r#"["a"]"#, &dirs),
        Err("the graph command's output is not a JSON object".to_string())
    );
    assert_eq!(
        from_command_json(r#"{"a": "b"}"#, &dirs),
        Err("a: expected an array of module names".to_string())
    );
    assert!(from_command_json("{", &dirs).is_err());
}

#[test]
fn duplicate_module_names_are_unknown() {
    // A graph command naming one module twice (`serde_json` alone keeps the last).
    assert_eq!(
        from_command_json(r#"{"a": [], "b": ["a"], "a": ["b"]}"#, &BTreeMap::new()),
        Err("two modules are named a".to_string())
    );
    // Two workspace members with one package name.
    let json = r#"{"workspace_root":"/w","workspace_members":["p1","p2"],"packages":[
        {"id":"p1","name":"p","manifest_path":"/w/a/Cargo.toml","dependencies":[]},
        {"id":"p2","name":"p","manifest_path":"/w/b/Cargo.toml","dependencies":[]}]}"#;
    assert_eq!(
        from_cargo_metadata(json),
        Err("two modules are named p".to_string())
    );
}
