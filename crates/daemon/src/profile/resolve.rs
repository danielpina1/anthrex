//! Decision 6's precedence, exact. Pure (decision 1): no file, process, thread,
//! async-runtime or clock access.
//!
//! One source supplies the whole profile. A confirmed stored profile wins outright:
//! every key comes from it, a key it leaves unset stays unset, and the plan's
//! `[profile]` and `[orchestrator.profile]` are ignored (each ignored plan key gets a
//! run-log note). Otherwise M8a decision 7's per-key rule applies unchanged. `protected`
//! is the one exception: adding protection only tightens, so it always merges.
//!
//! Confinement is not a profile key: `build_run` takes `cache_dirs` and the `confined_*`
//! entries from the user's config for the repository root, whatever this chooses.

use std::path::Path;

use proto::{OutputFilter, ProfileSource, ProfileSpec, RepoProfile};

/// The profile a run starts with, and what the driver copies onto the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChosenProfile {
    /// What becomes the plan's `[profile]`.
    pub spec: ProfileSpec,
    pub source: ProfileSource,
    /// Decision 28's filter settings: a stored profile's only, else the defaults.
    pub output_filter: OutputFilter,
    pub filter_prefixes: Vec<String>,
    /// Run-log lines: each plan key a stored profile made the run ignore.
    pub notes: Vec<String>,
}

/// Each plan key a stored profile overrides, with its name in the notes. `protected`
/// is absent: it merges and is never ignored.
fn set_keys(spec: &ProfileSpec) -> Vec<&'static str> {
    [
        ("modules", spec.modules.is_some()),
        ("hub", spec.hub.is_some()),
        ("source", spec.source.is_some()),
        ("check", spec.check.is_some()),
        ("check_timeout_secs", spec.check_timeout_secs.is_some()),
        ("single_test", spec.single_test.is_some()),
        ("test_passed", spec.test_passed.is_some()),
        ("setup", spec.setup.is_some()),
        ("generated", spec.generated.is_some()),
        ("build_check", spec.build_check.is_some()),
        ("module_test", spec.module_test.is_some()),
        ("module_tests", spec.module_tests.is_some()),
        ("module_graph", spec.module_graph.is_some()),
        ("module_names", spec.module_names.is_some()),
        ("full_triggers", spec.full_triggers.is_some()),
        ("slow_tests", spec.slow_tests.is_some()),
        ("timing_tests", spec.timing_tests.is_some()),
        ("skip_markers", spec.skip_markers.is_some()),
        ("test_paths", spec.test_paths.is_some()),
        ("full_shards", spec.full_shards.is_some()),
        ("toolchain_id", spec.toolchain_id.is_some()),
        ("env", spec.env.is_some()),
    ]
    .into_iter()
    .filter_map(|(key, set)| set.then_some(key))
    .collect()
}

/// Decision 6. `stored_path` is the stored profile's file, named in the notes.
pub fn run_profile(
    stored: Option<&RepoProfile>,
    stored_path: &Path,
    plan: &ProfileSpec,
    config: &ProfileSpec,
) -> ChosenProfile {
    let Some(stored) = stored else {
        let any = *plan != ProfileSpec::default() || *config != ProfileSpec::default();
        return ChosenProfile {
            spec: plan.clone(),
            source: if any {
                ProfileSource::Plan
            } else {
                ProfileSource::None
            },
            output_filter: OutputFilter::default(),
            filter_prefixes: Vec::new(),
            notes: Vec::new(),
        };
    };
    let mut spec = stored.spec();
    // A plan whose profile is the stored one (the fast path's, M8b.14) ignores nothing.
    let same = *plan == spec;
    // The stored extras, then the plan's, then the config's: the config's `profile` is
    // cleared for this run (`apply_choice`), so its entries travel in the spec.
    let mut protected: Vec<String> = Vec::new();
    for entry in stored
        .protected
        .iter()
        .chain(plan.protected.iter().flatten())
        .chain(config.protected.iter().flatten())
    {
        if !protected.contains(entry) {
            protected.push(entry.clone());
        }
    }
    spec.protected = (!protected.is_empty()).then_some(protected);
    let notes = set_keys(plan)
        .into_iter()
        .filter(|_| !same)
        .map(|key| {
            format!(
                "profile.{key} from the plan file is ignored: this repository has a stored profile ({})",
                stored_path.display()
            )
        })
        .collect();
    let filter_prefixes = if stored.filter_prefixes.is_empty() {
        derived_prefixes(stored)
    } else {
        stored.filter_prefixes.clone()
    };
    ChosenProfile {
        spec,
        source: ProfileSource::Stored,
        output_filter: stored.output_filter,
        filter_prefixes,
        notes,
    }
}

/// What `choose_profile` does to the plan's `[profile]` and the run's cloned config
/// (decision 6, refreshed): with a stored profile, the plan's profile becomes the
/// chosen spec and the config's `profile` is emptied, so M8a's per-key
/// `resolve_profile` finds nothing to fill a deliberate gap with. The config's
/// confinement tables are not touched. Otherwise nothing changes.
pub fn apply_choice(chosen: &ChosenProfile, plan: &mut ProfileSpec, config: &mut ProfileSpec) {
    if chosen.source == ProfileSource::Stored {
        *plan = chosen.spec.clone();
        *config = ProfileSpec::default();
    }
}

/// The first two whitespace-separated words of `text`, or `None` when it has none.
fn two_words(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().take(2).collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// Decision 28: the output filter's prefixes when the profile names none — the first
/// two words of each segment of `check` (split on `&&`, `||` and `;`), then the text of
/// `single_test` before `{test}` (cut back to its last separator when `{test}` is glued
/// to a word character), cut to two words; de-duplicated, in order.
pub fn derived_prefixes(profile: &RepoProfile) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(check) = &profile.check {
        for segment in check
            .split("&&")
            .flat_map(|s| s.split("||"))
            .flat_map(|s| s.split(';'))
        {
            candidates.extend(two_words(segment));
        }
    }
    if let Some(single) = &profile.single_test {
        let head = single.split("{test}").next().unwrap_or_default();
        // `{test}` glued to a word character (`tests/test_{test}.py`): cut back to the
        // last separator, or the prefix would never match its own command, since a
        // prefix ending in a word character must be followed by a boundary.
        let head = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
        candidates.extend(two_words(head).filter(|p| p.chars().any(char::is_alphanumeric)));
    }
    let mut out: Vec<String> = Vec::new();
    for candidate in candidates {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}
