//! Milestone 9.1 decisions 8 and 12 for a stored profile: the tier keys' problems, what
//! a scout's findings may propose, which verified tier commands a proposal keeps, and
//! the summary's `tiers:` line.
//! Pure, like `proposal.rs`.

use proto::{CommandCheck, DroppedCommand, ModuleNames, ProfileVerification, RepoProfile};

use super::proposal::{NOT_VERIFIED, failure};
use crate::run::tiers::command::{Piece, Placeholders, filter_expr, pieces, substitute};
use crate::run::tiers::{
    FULL_SHARDS_MAX, GraphSource, SKIP_MARKER_CHARS_MAX, SKIP_MARKERS_MAX, Scope, TierProfile,
    validate,
};

/// Why a module command is dropped when the names it needs come from a graph that was.
pub const NEEDS_GRAPH: &str = "needs a module graph";
/// Ruling C-3 (b): why a `check` with tier placeholders goes with the last tier key.
pub const UNTIERED_CHECK: &str = "check uses tier placeholders but no tier command was kept";
/// Ruling C-4 (3): why a filter no kept command can take is dropped.
pub const FILTER_UNUSED: &str = "no kept command can take this filter";
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
    // Review round 2, minor 1: any kept graph, cargo's or a command's.
    let graph_kept = tiers.module_graph != GraphSource::None && profile.module_graph.is_some();
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
    drop_invalid(proposed, profile, dropped);
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
/// with it, round after round. Ruling C-3 (c): a filter's problem is about the module
/// command without a `{filter:…}` slot, so that command goes, not the filter.
/// Ruling C-3 (a) and (b): then every kept command must still expand as verification
/// ran it under `proposed` ([`verified_form`]); one that would not goes too, naming
/// the dropped keys that changed it, and the rounds go on.
fn drop_invalid(
    proposed: &RepoProfile,
    profile: &mut RepoProfile,
    dropped: &mut Vec<DroppedCommand>,
) {
    const FILTERS: [&str; 2] = ["slow_tests", "timing_tests"];
    // Each round drops at least one key; there are fewer keys than rounds.
    for _ in 0..32 {
        let problems = validate(
            &TierProfile::from_repo(profile),
            profile.check.as_deref(),
            &profile.modules,
        );
        // (key to drop, the problem's own key, reason)
        let mut reasons: Vec<(String, String, String)> = Vec::new();
        for (own, message) in problems {
            // Ruling C-4 (2): only `<module command> must contain {filter:…` is about
            // the module command; any other filter problem is the filter's own.
            let command = ["module_test", "module_tests"]
                .into_iter()
                .find(|c| message.starts_with(&format!("{c} must contain {{filter:")));
            let (key, message) = match command {
                Some(c) if FILTERS.contains(&own.as_str()) => {
                    (c.to_string(), format!("{own}: {message}"))
                }
                _ => (own.clone(), message),
            };
            match reasons.iter_mut().find(|(k, _, _)| *k == key) {
                Some((_, _, reason)) => reason.push_str(&format!("\n{message}")),
                None => reasons.push((key, own, message)),
            }
        }
        if reasons.is_empty() {
            reasons = changed_expansions(proposed, profile);
        }
        if reasons.is_empty() {
            reasons = unused_filters(profile);
        }
        if reasons.is_empty() {
            return;
        }
        for (key, own, reason) in reasons {
            let before = profile.clone();
            let mut taken = take_key(profile, &key);
            let mut key = key;
            // No round is a no-op: an attribution that removed nothing falls back to
            // the problem's own key.
            if *profile == before && own != key {
                taken = take_key(profile, &own);
                key = own;
            }
            if *profile == before {
                continue;
            }
            dropped.push(DroppedCommand {
                key,
                command: taken.unwrap_or_default(),
                reason,
                tail: String::new(),
            });
        }
    }
}

/// Whether `command` holds a tier placeholder (any but `{test}`).
fn has_tier_placeholder(command: &str) -> bool {
    pieces(command)
        .iter()
        .any(|piece| !matches!(piece, Piece::Text(_) | Piece::Test))
}

/// Ruling C-4 (3): the slow and timing filters no kept command can take — neither
/// module command, nor `check` as a tiered check — each with why.
fn unused_filters(profile: &RepoProfile) -> Vec<(String, String, String)> {
    let slot = |command: &Option<String>| {
        command
            .as_deref()
            .is_some_and(|c| pieces(c).iter().any(|p| matches!(p, Piece::Filter(_))))
    };
    let tiered = TierProfile::from_repo(profile).is_tiered();
    if slot(&profile.module_test) || slot(&profile.module_tests) || (tiered && slot(&profile.check))
    {
        return Vec::new();
    }
    [
        ("slow_tests", &profile.slow_tests),
        ("timing_tests", &profile.timing_tests),
    ]
    .into_iter()
    .filter(|(_, filter)| filter.is_some())
    .map(|(key, _)| (key.to_string(), key.to_string(), FILTER_UNUSED.to_string()))
    .collect()
}

/// Ruling C-3 (a) and (b), with C-4 (1): the kept commands that would no longer run
/// as verified, each with why.
fn changed_expansions(
    proposed: &RepoProfile,
    profile: &RepoProfile,
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    // C-4 (1): a tiered proposal turned untiered loses a `check` with any tier
    // placeholder, even one (`{module}`) whose text did not change; a check that was
    // never tiered is M8b's and stays (decision 6).
    let untiered = !TierProfile::from_repo(profile).is_tiered();
    if untiered
        && TierProfile::from_repo(proposed).is_tiered()
        && profile.check.as_deref().is_some_and(has_tier_placeholder)
    {
        let key = "check".to_string();
        out.push((key.clone(), key, UNTIERED_CHECK.to_string()));
    }
    for key in ["check", "module_test", "module_tests"] {
        if out.iter().any(|(k, _, _)| k == key) {
            continue;
        }
        let Some(form) = verified_form(profile, key) else {
            continue;
        };
        if Some(&form) == verified_form(proposed, key).as_ref() {
            continue;
        }
        let changed: Vec<&str> = ["slow_tests", "timing_tests", "module_names", "module_graph"]
            .into_iter()
            .filter(|k| {
                let mut a = proposed.clone();
                let mut b = profile.clone();
                take_key(&mut a, k) != take_key(&mut b, k)
            })
            .collect();
        let why = match changed.as_slice() {
            [] => "the keys it depends on changed".to_string(),
            keys => format!("{} was dropped", keys.join(" and ")),
        };
        out.push((
            key.to_string(),
            key.to_string(),
            format!("it would no longer run as verified: {why}"),
        ));
    }
    out
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

/// The command M8b's `check` verification runs, and a run runs as the whole suite: for
/// a tiered profile, `check` with `{filter:…}` empty and `{shard}`/`{shards}` as 1 of
/// 1; otherwise `check` as written, as M8a runs it.
pub fn check_command(profile: &RepoProfile, check: &str) -> String {
    if !TierProfile::from_repo(profile).is_tiered() {
        return check.to_string();
    }
    let values = Placeholders {
        shard: Some((1, 1)),
        ..Placeholders::default()
    };
    substitute(check, &values)
}

/// `module_test` (or, `each`, `module_tests`) as verification runs it for `module`:
/// the module in, and the gate filter of `profile` (decision 25).
pub fn module_command(profile: &RepoProfile, template: &str, each: bool, module: &str) -> String {
    let values = Placeholders {
        module: (!each).then(|| module.to_string()),
        modules: each.then(|| vec![module.to_string()]),
        filter: filter_expr(&TierProfile::from_repo(profile), Scope::Gate, false),
        ..Placeholders::default()
    };
    substitute(template, &values)
}

/// What stands for the module in [`verified_form`]: the module verification picks
/// depends on the checkout, not on the profile.
const ANY_MODULE: &str = "<module>";

/// Ruling C-3 (a): how the kept command `key` (`check`, `module_test` or
/// `module_tests`) expands under `profile`, as verification ran it: the command and,
/// for a module command, how its module is named. `None` when the key is unset.
pub fn verified_form(profile: &RepoProfile, key: &str) -> Option<String> {
    let names = TierProfile::from_repo(profile).module_names;
    let module = |template: &str, each| {
        let command = module_command(profile, template, each, ANY_MODULE);
        format!("{command} ({names:?} names)")
    };
    match key {
        "check" => profile.check.as_deref().map(|c| check_command(profile, c)),
        "module_test" => profile.module_test.as_deref().map(|t| module(t, false)),
        "module_tests" => profile.module_tests.as_deref().map(|t| module(t, true)),
        _ => None,
    }
}
