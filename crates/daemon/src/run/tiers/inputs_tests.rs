//! Milestone 9.14 task M9.14.2: decision 26's inputs id. Pure.

use std::collections::BTreeSet;

use proto::ModuleNames;

use super::inputs::{INPUTS_PREFIX, closure, inputs_id, keeper, key_tree};
use super::{ModuleGraph, ModuleInfo};
use crate::run::git::TreeEntry;

const TREE: &str = "0123456789abcdef0123456789abcdef01234567";

/// `a` depends on `b` and, as a dev-dependency only (one list, as
/// `from_cargo_metadata` records every kind), on `d`; `c` stands alone.
fn graph() -> ModuleGraph {
    let mut graph = ModuleGraph::default();
    for (name, deps) in [("a", &["b", "d"][..]), ("b", &[]), ("c", &[]), ("d", &[])] {
        graph.modules.insert(
            name.to_string(),
            ModuleInfo {
                dir: Some(format!("crates/{name}")),
                deps: deps.iter().map(|d| d.to_string()).collect(),
            },
        );
    }
    graph
}

fn modules() -> Vec<String> {
    vec!["crates/*".to_string()]
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

const PATHS: [&str; 11] = [
    ".cargo/config.toml",
    "Cargo.lock",
    "Cargo.toml",
    "crates/a/src/lib.rs",
    "crates/b/src/lib.rs",
    "crates/c/src/lib.rs",
    "crates/d/src/lib.rs",
    "crates/zz/x.txt",
    "rust-toolchain.toml",
    "scripts/x.sh",
    "src/main.rs",
];

/// One entry per path of [`PATHS`], every object `1…1`, except `changed`'s.
fn entries(changed: Option<&str>) -> Vec<TreeEntry> {
    PATHS
        .iter()
        .map(|path| TreeEntry {
            mode: "100644".to_string(),
            object: if Some(*path) == changed {
                "2".repeat(40)
            } else {
                "1".repeat(40)
            },
            path: path.to_string(),
        })
        .collect()
}

/// The key tree of step modules `names` over `entries`.
fn id(entries: &[TreeEntry], names: &[&str]) -> String {
    key_tree(
        TREE,
        entries,
        &set(names),
        &modules(),
        ModuleNames::Dir,
        &graph(),
    )
}

#[test]
fn a_change_in_an_unrelated_module_keeps_the_id() {
    let (before, after) = (entries(None), entries(Some("crates/c/src/lib.rs")));
    assert_eq!(id(&before, &["a"]), id(&after, &["a"]));
    assert_ne!(id(&before, &["c"]), id(&after, &["c"]));
}

#[test]
fn a_change_in_a_dependency_moves_the_id() {
    let (before, after) = (entries(None), entries(Some("crates/b/src/lib.rs")));
    assert_ne!(
        id(&before, &["a"]),
        id(&after, &["a"]),
        "b is in a's closure"
    );
    assert_eq!(id(&before, &["c"]), id(&after, &["c"]));
    assert_eq!(closure(&graph(), &set(&["a"])), set(&["a", "b", "d"]));
    assert_eq!(closure(&graph(), &set(&["b"])), set(&["b"]), "not reverse");
}

#[test]
fn a_dev_dependency_is_in_the_closure() {
    let (before, after) = (entries(None), entries(Some("crates/d/src/lib.rs")));
    assert_ne!(id(&before, &["a"]), id(&after, &["a"]));
}

#[test]
fn a_root_file_change_moves_every_id() {
    let before = entries(None);
    for path in [
        "Cargo.lock",
        ".cargo/config.toml",
        "rust-toolchain.toml",
        "scripts/x.sh",
        "src/main.rs",
    ] {
        let after = entries(Some(path));
        assert_ne!(id(&before, &["a"]), id(&after, &["a"]), "{path}");
        assert_ne!(id(&before, &["c"]), id(&after, &["c"]), "{path}");
    }
}

#[test]
fn a_path_with_no_module_name_is_kept() {
    let (before, after) = (entries(None), entries(Some("crates/zz/x.txt")));
    for names in [&["a"][..], &["b"], &["c"]] {
        assert_ne!(id(&before, names), id(&after, names), "{names:?}");
    }
    let names = modules();
    let closure = set(&["c"]);
    let graph = graph();
    let keep = keeper(&closure, &names, ModuleNames::Dir, &graph);
    assert!(keep("crates/zz/x.txt") && keep("Cargo.lock") && keep("crates/c/y"));
    assert!(!keep("crates/a/src/lib.rs"));
}

#[test]
fn a_closure_of_every_module_keeps_the_tree_id() {
    let all = entries(None);
    assert_eq!(id(&all, &["a", "c"]), TREE);
    assert_eq!(id(&all, &["a", "b", "c", "d"]), TREE);
    assert!(id(&all, &["a"]).starts_with(INPUTS_PREFIX));
}

#[test]
fn the_id_is_prefixed_and_stable() {
    let all = entries(None);
    let keep = |_: &str| true;
    let first = inputs_id(&all, &keep);
    assert!(first.starts_with(INPUTS_PREFIX), "{first}");
    let hex = &first[INPUTS_PREFIX.len()..];
    assert_eq!(hex.len(), 64, "{first}");
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    assert_eq!(
        first,
        inputs_id(&entries(None), &keep),
        "equal input, equal id"
    );
    let mut executable = entries(None);
    executable[9].mode = "100755".to_string();
    assert_ne!(
        first,
        inputs_id(&executable, &keep),
        "a mode change moves it"
    );
    // The exact bytes: `<mode> <object>\t<path>\0` for each kept entry, in order.
    let one = [TreeEntry {
        mode: "100644".into(),
        object: "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".into(),
        path: "a".into(),
    }];
    use sha2::{Digest, Sha256};
    let want = Sha256::digest(b"100644 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\ta\0");
    let want: String = want.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(inputs_id(&one, &keep), format!("{INPUTS_PREFIX}{want}"));
}
