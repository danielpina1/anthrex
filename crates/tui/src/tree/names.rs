//! Picking each project root's display name: its own base name where that is unique,
//! disambiguated by parent directory (and, failing that, by the full path) where two
//! roots share one. Split out of `tree.rs` (task M6.10's file-size finding B,
//! `AGENTS.md` hard rule 8) — the same shape `tree/forest.rs` gives the sub-agent
//! forest.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn display_names<'a>(roots: impl IntoIterator<Item = &'a Path>) -> HashMap<PathBuf, String> {
    let roots: Vec<PathBuf> = roots
        .into_iter()
        .map(Path::to_path_buf)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut by_base: HashMap<String, Vec<&PathBuf>> = HashMap::new();
    for root in &roots {
        by_base.entry(base_name(root)).or_default().push(root);
    }

    let mut names = HashMap::new();
    for (base, group) in by_base {
        if group.len() == 1 {
            names.insert(group[0].clone(), base);
            continue;
        }

        let candidates: Vec<_> = group
            .iter()
            .map(|root| {
                let parent = root
                    .parent()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string());
                format!("{base} ({parent})")
            })
            .collect();
        let mut candidate_counts: HashMap<String, usize> = HashMap::new();
        for candidate in &candidates {
            *candidate_counts.entry(candidate.clone()).or_default() += 1;
        }
        for (root, candidate) in group.into_iter().zip(candidates) {
            let name = if candidate_counts[&candidate] > 1 {
                root.display().to_string()
            } else {
                candidate
            };
            names.insert(root.clone(), name);
        }
    }
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for name in names.values() {
        *name_counts.entry(name.clone()).or_default() += 1;
    }
    for (root, name) in &mut names {
        if name_counts[name] > 1 {
            *name = root.display().to_string();
        }
    }
    names
}

fn base_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}
