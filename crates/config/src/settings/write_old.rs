//! Milestone 9.8 decision 40 (M9.8.12): the old model keys a save removes. Pure.
//!
//! Decision 18's keys go: `default_runtime` and `builtin_models` lines, every
//! `[[orchestrator.models]]` block, every `[orchestrator.routes.<name>]` table, and the
//! `agent`, `planners`, `scouts` and `deciders` lines that fed a row (`deciders.mode`
//! stays, decision 17). A table the removal leaves empty loses its header. A key
//! [`Kept`] holds stays (Task 3's review). An old key written in another form (a
//! dotted key, an inline table) is not found here; `write_models::edit` refuses it.

use crate::models::Kept;

use super::scan::{Edits, Scan, Section, is};

/// The tables whose lines the removal may empty, and so whose header it may remove.
const OLD_TABLES: [&str; 6] = [
    "orchestrator",
    "orchestrator.agent",
    "orchestrator.planners",
    "orchestrator.scouts",
    "orchestrator.deciders",
    "orchestrator.routes",
];

/// The keys of `table` that fed a row (decision 18).
fn old_lines(table: &str) -> &'static [&'static str] {
    match table {
        "orchestrator" => &["default_runtime", "builtin_models"],
        "orchestrator.agent" => &["runtime", "model", "effort"],
        "orchestrator.planners" | "orchestrator.scouts" => &["runtime", "strength", "effort"],
        "orchestrator.deciders" => &["strength", "effort"],
        _ => &[],
    }
}

fn dotted(path: &[String]) -> String {
    path.join(".")
}

/// Marks every old key's lines in `scan` deleted, except what `kept` holds. `true` when
/// it removed at least one.
pub(super) fn remove(scan: &Scan, e: &mut Edits, kept: &Kept) -> bool {
    let before = e.delete.len();
    for s in scan.sections.iter().filter(|s| s.header.is_some()) {
        let path = dotted(&s.path);
        if s.array {
            if path == "orchestrator.models" && !kept.holds(&path) {
                scan.delete_section(e, s);
            }
            continue;
        }
        if is_route(s) {
            if !kept.holds(&path) {
                scan.delete_section(e, s);
            }
            continue;
        }
        for &i in &s.keys {
            let (key, _, _) = scan.key_of(i);
            let old = key.len() == 1 && old_lines(&path).contains(&key[0].as_str());
            if old && !kept.holds(&format!("{path}.{}", key[0])) {
                scan.delete_key(e, i);
            }
        }
    }
    let removed = e.delete.len() > before;
    if removed {
        drop_emptied_headers(scan, e);
    }
    removed
}

/// `[orchestrator.routes.<name>]` for a name decision 18 lists.
fn is_route(s: &Section) -> bool {
    s.path.len() == 3
        && is(&s.path[..2], "orchestrator.routes")
        && crate::models::is_route_name(&s.path[2])
}

/// An old table left with no key loses its header; `[orchestrator.routes]` does when no
/// route table under it survives either.
fn drop_emptied_headers(scan: &Scan, e: &mut Edits) {
    for s in &scan.sections {
        let (Some(header), false) = (s.header, s.array) else {
            continue;
        };
        if !OLD_TABLES.iter().any(|t| is(&s.path, t)) {
            continue;
        }
        let all_gone = s.keys.iter().all(|i| e.delete.contains(i));
        let emptied = !s.keys.is_empty() && all_gone;
        let routes = is(&s.path, "orchestrator.routes")
            && s.keys.is_empty()
            && !scan.sections.iter().any(|r| {
                r.path.len() > 2
                    && r.path.starts_with(&s.path)
                    && r.header.is_some_and(|h| !e.delete.contains(&h))
            });
        if emptied || routes {
            e.delete.insert(header);
        }
    }
}
