//! Milestone 9.1 decision 5: the tier keys of `[orchestrator.profile]`. Split out of
//! `profile.rs` to keep it within its budget.

use super::super::Orchestrator;
use super::{read_profile_string, read_profile_string_list};
use crate::Problem;

/// Milestone 9.1 decision 5's keys. Their rules across keys (placeholders, filters,
/// shards) are the daemon's, checked when a plan resolves (decision 8); here only each
/// value's type, `module_names`'s two words and `full_shards`'s range.
pub(super) fn read_tier_keys(
    profile: &toml::Table,
    o: &mut Orchestrator,
    problems: &mut Vec<Problem>,
) {
    let p = &mut o.profile;
    for (key, field) in [
        ("build_check", &mut p.build_check),
        ("module_test", &mut p.module_test),
        ("module_tests", &mut p.module_tests),
        ("module_graph", &mut p.module_graph),
        ("slow_tests", &mut p.slow_tests),
        ("timing_tests", &mut p.timing_tests),
        ("toolchain_id", &mut p.toolchain_id),
    ] {
        read_profile_string(profile, key, field, problems);
    }
    for (key, field) in [
        ("full_triggers", &mut p.full_triggers),
        ("skip_markers", &mut p.skip_markers),
        ("test_paths", &mut p.test_paths),
    ] {
        read_profile_string_list(profile, key, field, problems);
    }
    if let Some(v) = profile.get("module_names") {
        match v.as_str() {
            Some("cargo") => p.module_names = Some(proto::ModuleNames::Cargo),
            Some("dir") => p.module_names = Some(proto::ModuleNames::Dir),
            _ => problems.push(Problem {
                key: "orchestrator.profile.module_names".to_string(),
                message: "expected \"cargo\" or \"dir\"".to_string(),
                default: "unset".to_string(),
            }),
        }
    }
    if let Some(v) = profile.get("full_shards") {
        match v
            .as_integer()
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| (1..=16).contains(n))
        {
            Some(n) => p.full_shards = Some(n),
            None => problems.push(Problem {
                key: "orchestrator.profile.full_shards".to_string(),
                message: "must be between 1 and 16".to_string(),
                default: "unset".to_string(),
            }),
        }
    }
}
