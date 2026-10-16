//! Milestone 9.16 ruling C19: every end-to-end daemon a test starts runs with the quota
//! brake off (`[orchestrator.brakes] quota_brake = false`), so no run log gains decision
//! 8's `<runtime> quota is unmeasured; dispatch is not braked` line (fake agents never
//! report a reading), unless the test opts in by supplying its own
//! `[orchestrator.brakes]` table, which replaces the default rather than adding a second
//! one (`lessons::append_toml`). The hold is not a config key: it works either way.

/// The harnesses' default `[orchestrator.brakes]` table.
pub const BRAKES_OFF: &str = "[orchestrator.brakes]\nquota_brake = false\n";

/// U11's defaults, spelled out: a test that runs the brake writes this table.
pub const BRAKES_ON: &str = "[orchestrator.brakes]\nquota_brake = true\n";

/// Whether `toml` has its own `[orchestrator.brakes]` table, or sets a `brakes.` key
/// inside `[orchestrator]` (lines a harness writes into that table).
pub fn has_brakes_table(toml: &str) -> bool {
    toml.lines().any(|l| {
        let l = l.trim();
        l == "[orchestrator.brakes]" || l.starts_with("brakes.")
    })
}

/// The default table for a config whose `[orchestrator]` lines are `orchestrator`:
/// none when those lines set a brake key themselves.
pub fn default_for(orchestrator: &str) -> &'static str {
    if has_brakes_table(orchestrator) {
        ""
    } else {
        BRAKES_OFF
    }
}
