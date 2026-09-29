//! Decision 9: a module graph from `cargo metadata --format-version 1 --no-deps` or
//! from a graph command's JSON. Pure: the caller runs the command and hands in its
//! stdout.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use serde::de::{Deserialize, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;

use super::{ModuleGraph, ModuleInfo};

/// The workspace members of a `cargo metadata` output, each named by its package name,
/// with its manifest directory relative to the output's own `workspace_root` (`.` for
/// a root package) and its dependencies of every kind that name another member.
pub fn from_cargo_metadata(json: &str) -> Result<ModuleGraph, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("cargo metadata is not JSON: {e}"))?;
    let root = value["workspace_root"]
        .as_str()
        .ok_or("cargo metadata has no workspace_root")?;
    let members: BTreeSet<&str> = value["workspace_members"]
        .as_array()
        .ok_or("cargo metadata has no workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let packages: Vec<&Value> = value["packages"]
        .as_array()
        .ok_or("cargo metadata has no packages")?
        .iter()
        .filter(|p| p["id"].as_str().is_some_and(|id| members.contains(id)))
        .collect();
    let names: BTreeSet<&str> = packages.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut graph = ModuleGraph::default();
    for package in packages {
        let name = package["name"].as_str().ok_or("a package has no name")?;
        let manifest = package["manifest_path"]
            .as_str()
            .ok_or_else(|| format!("package {name} has no manifest_path"))?;
        let dir = Path::new(manifest)
            .parent()
            .and_then(|dir| dir.strip_prefix(root).ok())
            .map(|dir| match dir.to_string_lossy().as_ref() {
                "" => ".".to_string(),
                dir => dir.to_string(),
            });
        let deps = package["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["name"].as_str())
            .filter(|dep| names.contains(dep) && *dep != name)
            .map(str::to_string)
            .collect();
        if graph
            .modules
            .insert(name.to_string(), ModuleInfo { dir, deps })
            .is_some()
        {
            return Err(format!("two modules are named {name}"));
        }
    }
    Ok(graph)
}

/// A graph command's stdout, `{"<module>": ["<dep>", …], …}`. Every dependency must be
/// a module. `dirs` gives each module's directory where the caller knows it.
pub fn from_command_json(
    json: &str,
    dirs: &BTreeMap<String, String>,
) -> Result<ModuleGraph, String> {
    let value: Value = serde_json::from_str(json.trim())
        .map_err(|e| format!("the graph command's output is not JSON: {e}"))?;
    let Value::Object(map) = value else {
        return Err("the graph command's output is not a JSON object".to_string());
    };
    // `Value` keeps the last of two equal keys; the graph is unknown instead.
    let mut seen = BTreeSet::new();
    let Keys(keys) = serde_json::from_str(json.trim())
        .map_err(|e| format!("the graph command's output is not JSON: {e}"))?;
    for key in keys {
        if !seen.insert(key.clone()) {
            return Err(format!("two modules are named {key}"));
        }
    }
    let mut graph = ModuleGraph::default();
    for (name, deps) in &map {
        let Value::Array(deps) = deps else {
            return Err(format!("{name}: expected an array of module names"));
        };
        let mut set = BTreeSet::new();
        for dep in deps {
            let dep = dep
                .as_str()
                .ok_or_else(|| format!("{name}: expected an array of module names"))?;
            if !map.contains_key(dep) {
                return Err(format!("{dep} is not a module"));
            }
            set.insert(dep.to_string());
        }
        graph.modules.insert(
            name.clone(),
            ModuleInfo {
                dir: dirs.get(name).cloned(),
                deps: set,
            },
        );
    }
    Ok(graph)
}

/// A JSON object's top-level keys in order, repeats included.
struct Keys(Vec<String>);

impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> Visitor<'de> for Visit {
            type Value = Keys;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
                let mut keys = Vec::new();
                while let Some((key, _)) = map.next_entry::<String, IgnoredAny>()? {
                    keys.push(key);
                }
                Ok(Keys(keys))
            }
        }
        deserializer.deserialize_map(Visit)
    }
}
