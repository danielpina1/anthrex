//! Milestone 9.1 decisions 8 and 12 for a stored profile: the tier keys' problems, what
//! a scout's findings may propose, which verified tier commands a proposal keeps, and
//! the summary's `tiers:` line.
//! Pure, like `proposal.rs`.

use proto::{CommandCheck, DroppedCommand, ModuleNames, ProfileVerification, RepoProfile};

use super::proposal::{NOT_VERIFIED, failure};
use crate::run::tiers::{
    FULL_SHARDS_MAX, GraphSource, SKIP_MARKER_CHARS_MAX, SKIP_MARKERS_MAX, TierProfile, validate,
};

/// Why a module command is dropped when the names it needs come from a graph that was.
pub const NEEDS_GRAPH: &str = "needs a module graph";
/// Why `full_shards` above 1 is dropped (decision 12).
pub const SHARDS_NEED_CHECK: &str = "needs a verified check that contains {shard} and {shards}";

/// Decision 8's problems, `<key>: <problem>`.
pub fn problems(profile: &RepoProfile) -> Vec<String> {
    validate(
        &TierProfile::from_repo(profile),
        profile.check.as_deref(),
        &profile.modules,
    )
    .into_iter()
    .map(|(key, message)| format!("{key}: {message}"))
    .collect()
}

/// A scout's tier keys with what cannot be proposed removed: a blank command or
/// filter, a marker out of bounds (and every marker past the 32nd), and an
/// out-of-range `full_shards`. The glob lists are filtered by the caller; commands are
/// left for verification, and what still breaks decision 8 after it is dropped by
/// [`apply_verification`].
pub fn from_findings(out: &mut RepoProfile) {
    for slot in [
        &mut out.build_check,
        &mut out.module_test,
        &mut out.module_tests,
        &mut out.module_graph,
        &mut out.slow_tests,
        &mut out.timing_tests,
        &mut out.toolchain_id,
    ] {
        if slot.as_deref().is_some_and(|v| v.trim().is_empty()) {
            *slot = None;
        }
    }
    out.skip_markers
        .retain(|m| !m.is_empty() && m.chars().count() <= SKIP_MARKER_CHARS_MAX);
    out.skip_markers.truncate(SKIP_MARKERS_MAX);
    out.full_shards = out
        .full_shards
        .filter(|n| (1..=FULL_SHARDS_MAX).contains(n));
}

/// Keeps `slot`'s command when `record` shows that command passed; otherwise empties
/// it and lists why, as M8b does for `setup` and `check`.
fn keep_or_drop(
    key: &str,
    slot: &mut Option<String>,
    record: &Option<CommandCheck>,
    hint: Option<&str>,
    dropped: &mut Vec<DroppedCommand>,
) {
    let Some(command) = slot.clone() else {
        return;
    };
    let (reason, tail) = match record {
        Some(c) if c.command == command && c.ok => return,
        Some(c) if c.command == command => {
            let reason = match hint {
                Some(hint) => format!("{}\n{hint}", failure(c)),
                None => failure(c),
            };
            (reason, c.tail.clone())
        }
        _ => (NOT_VERIFIED.to_string(), String::new()),
    };
    *slot = None;
    dropped.push(DroppedCommand {
        key: key.to_string(),
        command,
        reason,
        tail,
    });
}

fn drop_with(
    key: &str,
    slot: &mut Option<String>,
    reason: &str,
    dropped: &mut Vec<DroppedCommand>,
) {
    if let Some(command) = slot.take() {
        dropped.push(DroppedCommand {
            key: key.to_string(),
            command,
            reason: reason.to_string(),
            tail: String::new(),
        });
    }
}

/// Decision 12, after M8b's three: `build_check`, `module_graph` (unless `"none"`),
/// `module_test`, `module_tests` and `toolchain_id` are kept only when their own
/// command passed. With `module_names = "cargo"` the module commands go with a dropped
/// graph (`needs a module graph`). `full_shards` above 1 stays only when the verified
/// `check` still holds `{shard}` and `{shards}`.
pub fn apply_verification(
    proposed: &RepoProfile,
    profile: &mut RepoProfile,
    v: &ProfileVerification,
    hint: Option<&str>,
    dropped: &mut Vec<DroppedCommand>,
) {
    let tiers = TierProfile::from_repo(proposed);
    keep_or_drop(
        "build_check",
        &mut profile.build_check,
        &v.build_check,
        hint,
        dropped,
    );
    if tiers.module_graph != GraphSource::None {
        keep_or_drop(
            "module_graph",
            &mut profile.module_graph,
            &v.module_graph,
            hint,
            dropped,
        );
    }
    let graph_kept =
        matches!(tiers.module_graph, GraphSource::Cargo) && profile.module_graph.is_some();
    for (key, slot, record) in [
        ("module_test", &mut profile.module_test, &v.module_test),
        ("module_tests", &mut profile.module_tests, &v.module_tests),
    ] {
        if tiers.module_names == ModuleNames::Cargo && !graph_kept {
            drop_with(key, slot, NEEDS_GRAPH, dropped);
        } else {
            keep_or_drop(key, slot, record, hint, dropped);
        }
    }
    keep_or_drop(
        "toolchain_id",
        &mut profile.toolchain_id,
        &v.toolchain_id,
        hint,
        dropped,
    );
    if let Some(n) = profile.full_shards.filter(|n| *n > 1) {
        let check = profile.check.as_deref().unwrap_or_default();
        if !(check.contains("{shard}") && check.contains("{shards}")) {
            profile.full_shards = None;
            dropped.push(DroppedCommand {
                key: "full_shards".to_string(),
                command: n.to_string(),
                reason: SHARDS_NEED_CHECK.to_string(),
                tail: String::new(),
            });
        }
    }
    drop_invalid(profile, dropped);
}

/// Review I1: the key `key` taken out of `profile`, as its value's text.
fn take_key(profile: &mut RepoProfile, key: &str) -> Option<String> {
    let list = |list: &mut Vec<String>| Some(std::mem::take(list).join(", "));
    match key {
        "check" => profile.check.take(),
        "build_check" => profile.build_check.take(),
        "module_test" => profile.module_test.take(),
        "module_tests" => profile.module_tests.take(),
        "module_graph" => profile.module_graph.take(),
        "slow_tests" => profile.slow_tests.take(),
        "timing_tests" => profile.timing_tests.take(),
        "toolchain_id" => profile.toolchain_id.take(),
        "module_names" => profile.module_names.take().map(|n| match n {
            ModuleNames::Cargo => "cargo".to_string(),
            ModuleNames::Dir => "dir".to_string(),
        }),
        "full_shards" => profile.full_shards.take().map(|n| n.to_string()),
        "full_triggers" => list(&mut profile.full_triggers),
        "test_paths" => list(&mut profile.test_paths),
        "skip_markers" => list(&mut profile.skip_markers),
        _ => None,
    }
}

/// Review I1: what verification kept must pass decision 8, or every run with the
/// confirmed profile would be refused. Each key with a problem is dropped and listed
/// with it, in cascade: the keys a problem is about first, and the slow and timing
/// filters only when nothing else is left to drop (dropping a module command can make
/// them valid again). `module_names` goes when the cargo graph went.
fn drop_invalid(profile: &mut RepoProfile, dropped: &mut Vec<DroppedCommand>) {
    const FILTERS: [&str; 2] = ["slow_tests", "timing_tests"];
    // Each round drops at least one key; there are fewer keys than rounds.
    for _ in 0..16 {
        let problems = validate(
            &TierProfile::from_repo(profile),
            profile.check.as_deref(),
            &profile.modules,
        );
        let primary: Vec<(String, String)> = problems
            .iter()
            .filter(|(key, _)| !FILTERS.contains(&key.as_str()))
            .cloned()
            .collect();
        let round = if primary.is_empty() {
            problems
        } else {
            primary
        };
        if round.is_empty() {
            return;
        }
        let mut reasons: Vec<(String, String)> = Vec::new();
        for (key, message) in round {
            match reasons.iter_mut().find(|(k, _)| *k == key) {
                Some((_, reason)) => reason.push_str(&format!("\n{message}")),
                None => reasons.push((key, message)),
            }
        }
        for (key, reason) in reasons {
            let command = take_key(profile, &key).unwrap_or_default();
            dropped.push(DroppedCommand {
                key,
                command,
                reason,
                tail: String::new(),
            });
        }
    }
}

/// Milestone 9.1 decision 12: `tiers: <parts>` for a tiered profile, naming the parts
/// that are set of `build_check`, `module tests by <cargo|command> graph`, `slow
/// filter` and `<n> full triggers`; no line when none is.
pub fn summary_line(profile: &RepoProfile) -> Option<String> {
    let tiers = TierProfile::from_repo(profile);
    if !tiers.is_tiered() {
        return None;
    }
    let module_tests = tiers.module_test.is_some() || tiers.module_tests.is_some();
    let graph = match tiers.module_graph {
        GraphSource::Cargo => Some("cargo"),
        GraphSource::Command(_) => Some("command"),
        GraphSource::None => None,
    };
    let parts: Vec<String> = [
        tiers
            .build_check
            .is_some()
            .then(|| "build_check".to_string()),
        graph
            .filter(|_| module_tests)
            .map(|g| format!("module tests by {g} graph")),
        tiers
            .slow_tests
            .is_some()
            .then(|| "slow filter".to_string()),
        (!tiers.full_triggers.is_empty())
            .then(|| format!("{} full triggers", tiers.full_triggers.len())),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| format!("tiers: {}", parts.join(", ")))
}
