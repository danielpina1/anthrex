//! The release clean's plan (milestone 9.14 decision 8, C21). Pure: it reads a recorded
//! `cargo metadata` output and the pool directory's entries, and says which `cargo
//! clean` commands to run, or that the directory must be wiped instead.

use serde::Deserialize;

use crate::launch::shell_quote;

/// Every package with no `source` (workspace members and vendored path crates), as
/// `<name>@<version>`, sorted. Duplicates are kept: [`plan`] refuses them.
pub fn path_packages(metadata_json: &str) -> Result<Vec<String>, String> {
    let metadata: Metadata = serde_json::from_str(metadata_json)
        .map_err(|error| format!("unreadable cargo metadata: {error}"))?;
    let mut specs: Vec<String> = metadata
        .packages
        .into_iter()
        .filter(|package| package.source.is_none())
        .map(|package| format!("{}@{}", package.name, package.version))
        .collect();
    specs.sort();
    Ok(specs)
}

/// The part of `cargo metadata --format-version 1` the clean reads.
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
    #[serde(default)]
    source: Option<String>,
}

/// What a release does with a pool directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanPlan {
    /// Shell commands, run in order in the outgoing checkout.
    Commands(Vec<String>),
    /// Wipe instead, for this reason.
    Wipe(String),
}

/// The plan for `packages` in a pool directory whose relative paths of depth ≤ 2 are
/// `entries`.
///
/// A profile directory is a top-level directory that holds `.fingerprint`: `debug` is
/// `--profile dev`, any other name `<p>` is `--profile <p>` (`release`, or a custom
/// profile; cargo writes `test` into `debug/` and `bench` into `release/`). A top-level
/// directory without `.fingerprint` that holds `debug` or `release` is a target-triple
/// directory, which this clean does not reach: a wipe. So are two equal specs.
pub fn plan(packages: &[String], entries: &[String]) -> CleanPlan {
    let has = |path: String| entries.contains(&path);
    let top: Vec<&str> = entries
        .iter()
        .map(String::as_str)
        .filter(|entry| !entry.contains('/'))
        .collect();
    let mut profiles = Vec::new();
    for dir in top {
        if has(format!("{dir}/.fingerprint")) {
            profiles.push(if dir == "debug" { "dev" } else { dir });
        } else if has(format!("{dir}/debug")) || has(format!("{dir}/release")) {
            return CleanPlan::Wipe("a target triple directory".to_string());
        }
    }
    let mut sorted = packages.to_vec();
    sorted.sort();
    if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
        return CleanPlan::Wipe("an ambiguous package spec".to_string());
    }
    if packages.is_empty() {
        // `cargo clean` with no `-p` would clean every package.
        return CleanPlan::Commands(Vec::new());
    }
    let specs: Vec<String> = packages
        .iter()
        .map(|spec| format!("-p {}", shell_quote(spec)))
        .collect();
    let specs = specs.join(" ");
    let commands = profiles
        .into_iter()
        .map(|profile| {
            format!(
                "cargo clean --offline --profile {} {specs}",
                shell_quote_bare(profile)
            )
        })
        .collect();
    CleanPlan::Commands(commands)
}

/// A profile name as it is: cargo's profile names are `[A-Za-z0-9_-]`; anything else is
/// quoted.
fn shell_quote_bare(name: &str) -> String {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        name.to_string()
    } else {
        shell_quote(name)
    }
}

#[cfg(test)]
#[path = "clean_tests.rs"]
mod tests;
