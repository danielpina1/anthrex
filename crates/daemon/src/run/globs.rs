//! `owns`-glob rules, decision 11, plus decision 56's literal-name check. Pure — no
//! `std::fs`, `std::process`, `std::thread`, `tokio` or `std::time::SystemTime` (design
//! decision 2).
//!
//! Two different notions of "glob" meet here, and decision 11 keeps them apart:
//! - **Intersection** ([`intersects`], [`any_intersect`], [`modules_spanned`],
//!   [`inside_area`]) is a conservative, purely syntactic test on a glob's *literal
//!   prefix* ([`literal_prefix`]) — the path components before the first one containing
//!   a glob metacharacter. It never looks at an actual path.
//! - **Matching** ([`OwnsMatcher`]) decides whether one real path falls under an `owns`
//!   list, with `globset` and `literal_separator(true)` so `*` never crosses a `/`.

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

/// A glob metacharacter, per decision 11's literal-prefix rule.
const WILDCARD_CHARS: [char; 3] = ['*', '?', '['];

/// True when `component` contains no glob metacharacter.
fn is_literal_component(component: &str) -> bool {
    !component.contains(WILDCARD_CHARS)
}

/// True when the whole glob has no metacharacter anywhere, so it names one path (and,
/// via [`OwnsMatcher`], everything below it) rather than a pattern.
fn is_literal_glob(glob: &str) -> bool {
    !glob.contains(WILDCARD_CHARS)
}

/// A glob's literal prefix: its `/`-separated components up to, but not including, the
/// first one containing `*`, `?` or `[`. `crates/proto/**` -> `["crates", "proto"]`;
/// `**/*.rs` -> `[]`; `src/a[0-9].rs` -> `["src"]`. *(Decision 11.)*
pub fn literal_prefix(glob: &str) -> Vec<&str> {
    let mut prefix = Vec::new();
    for component in glob.split('/') {
        if !is_literal_component(component) {
            break;
        }
        prefix.push(component);
    }
    prefix
}

/// Two globs intersect when either one's literal prefix is a component-prefix of the
/// other's — the refreshed M8 brief's conservative test (decision 11). `**/*.rs`
/// intersects everything, because its literal prefix is empty and the empty sequence is
/// a prefix of any sequence.
pub fn intersects(a: &str, b: &str) -> bool {
    let pa = literal_prefix(a);
    let pb = literal_prefix(b);
    pa.starts_with(pb.as_slice()) || pb.starts_with(pa.as_slice())
}

/// True when some glob in `a` intersects some glob in `b`. Two empty lists never
/// intersect, since there is nothing to compare.
pub fn any_intersect(a: &[String], b: &[String]) -> bool {
    a.iter().any(|ga| b.iter().any(|gb| intersects(ga, gb)))
}

/// Which module (or modules) an `owns` list spans, decision 9's "Modules" paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleSpan {
    One(String),
    Many,
}

/// One glob's own module, or a marker that it spans more than one module by itself
/// (decision 9: a glob shorter than a `profile.modules` pattern but a component-prefix
/// of that pattern's own literal part).
enum GlobModule {
    Named(String),
    Many,
}

fn glob_module(glob: &str, modules: &[String]) -> GlobModule {
    let prefix = literal_prefix(glob);
    for pattern in modules {
        let pattern_components: Vec<&str> = pattern.split('/').collect();
        let k = pattern_components.len();
        if prefix.len() >= k {
            let matches = pattern_components
                .iter()
                .zip(prefix.iter())
                .all(|(pc, gc)| *pc == "*" || pc == gc);
            if matches {
                return GlobModule::Named(prefix[..k].join("/"));
            }
        } else {
            let pattern_literal = literal_prefix(pattern);
            let is_component_prefix = !prefix.is_empty()
                && prefix.len() <= pattern_literal.len()
                && prefix
                    .iter()
                    .zip(pattern_literal.iter())
                    .all(|(a, b)| a == b);
            if is_component_prefix {
                return GlobModule::Many;
            }
        }
    }
    GlobModule::Named(".".to_string())
}

/// Which module (or modules) `owns` spans, given `profile.modules`. With no modules
/// configured, every glob's module is `.`. *(Decision 9.)*
pub fn modules_spanned(owns: &[String], modules: &[String]) -> ModuleSpan {
    let mut distinct: Vec<String> = Vec::new();
    for glob in owns {
        match glob_module(glob, modules) {
            GlobModule::Named(name) => {
                if !distinct.contains(&name) {
                    distinct.push(name);
                }
                if distinct.len() > 1 {
                    return ModuleSpan::Many;
                }
            }
            GlobModule::Many => return ModuleSpan::Many,
        }
    }
    match distinct.into_iter().next() {
        Some(name) => ModuleSpan::One(name),
        None => ModuleSpan::One(".".to_string()),
    }
}

/// True when `glob` lies inside `area` (decision 12, `EditScope::Area`): an area glob is
/// `<literal>/**` or a literal path, and `glob` is inside when its literal prefix starts
/// with the area glob's literal prefix.
pub fn inside_area(glob: &str, area: &[String]) -> bool {
    let g = literal_prefix(glob);
    area.iter().any(|a| {
        let ap = literal_prefix(a);
        g.starts_with(ap.as_slice())
    })
}

/// Not blank, not absolute, no `..` component. *(Decision 11.)*
pub fn validate_glob(glob: &str) -> Result<(), String> {
    if glob.trim().is_empty() {
        return Err("must not be blank".to_string());
    }
    if glob.starts_with('/') {
        return Err("must not be absolute".to_string());
    }
    if glob.split('/').any(|component| component == "..") {
        return Err("must not contain ..".to_string());
    }
    Ok(())
}

/// Decision 56's literal-name check: does `owns` contain `path`, named exactly, with no
/// glob metacharacter? A literal entry is compared after dropping a leading `./` and a
/// trailing `/`. A wildcard never counts, and neither does a plain directory entry that
/// merely covers `path` (`.claude` in `owns` does not name `.claude/settings.json`).
pub fn names_literally(owns: &[String], path: &str) -> bool {
    let target = normalize_literal(path);
    owns.iter()
        .any(|entry| is_literal_glob(entry) && normalize_literal(entry) == target)
}

fn normalize_literal(s: &str) -> &str {
    let s = s.strip_prefix("./").unwrap_or(s);
    s.trim_end_matches('/')
}

/// Whether a path falls under an `owns` list, decision 11's "Matching": `globset` with
/// `literal_separator(true)`, so `*` never crosses a `/`. An entry with no glob
/// metacharacter matches itself and everything below it; a trailing `/` is ignored.
pub struct OwnsMatcher {
    set: GlobSet,
}

impl OwnsMatcher {
    pub fn new(owns: &[String]) -> Result<Self, String> {
        let mut builder = GlobSetBuilder::new();
        for entry in owns {
            for pattern in match_patterns(entry) {
                let glob = build_glob(&pattern)?;
                builder.add(glob);
            }
        }
        let set = builder
            .build()
            .map_err(|err| format!("invalid owns patterns: {err}"))?;
        Ok(OwnsMatcher { set })
    }

    pub fn matches(&self, path: &str) -> bool {
        self.set.is_match(path)
    }
}

/// The one or two glob patterns one `owns` entry expands to: a literal entry matches
/// itself and everything below it, so it becomes both `<literal>` and `<literal>/**`; a
/// wildcard entry is used as written. A trailing `/` is dropped either way.
fn match_patterns(entry: &str) -> Vec<String> {
    if is_literal_glob(entry) {
        let trimmed = entry.trim_end_matches('/');
        vec![trimmed.to_string(), format!("{trimmed}/**")]
    } else {
        vec![entry.trim_end_matches('/').to_string()]
    }
}

fn build_glob(pattern: &str) -> Result<Glob, String> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|err| format!("invalid glob {pattern:?}: {err}"))
}

#[cfg(test)]
#[path = "globs_tests.rs"]
mod tests;
