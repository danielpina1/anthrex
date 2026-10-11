//! The repository profile's keys whose change re-runs verification, for `anthrex profile
//! edit` and the client's profile pages (decision 10).

/// The keys whose change re-runs verification (decision 10).
pub const REVERIFY_KEYS: &[&str] = &[
    "setup",
    "check",
    "check_timeout_secs",
    "single_test",
    "test_passed",
    "sample_test",
    "env",
    // Milestone 9.1 decision 12: the tier commands and what they depend on.
    "build_check",
    "module_test",
    "module_tests",
    "module_graph",
    "module_names",
    "slow_tests",
    "timing_tests",
    "full_shards",
    "toolchain_id",
];
