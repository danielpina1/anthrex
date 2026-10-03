//! Builders for `adapt_tests.rs`: one of each milestone-8b type, every field set.
//! Split out to keep each file under the 600-line rule.

use super::*;

/// Spec §6's TOML block, with `protected` reduced to one extra entry, plus the three
/// keys milestone 8b adds (decision 5).
pub(super) const SPEC_PROFILE: &str = r#"
languages = ["rust"]
modules   = ["crates/*"]
hub       = ["crates/proto/**"]
source    = ["crates/*/src/**"]
generated = ["Cargo.lock"]
protected = [".cursor/**"]
setup     = "cargo fetch"
check     = "cargo build --workspace --all-targets && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check"
single_test = "cargo test --workspace -- --exact {test}"
test_passed = 'test {test} \.\.\. ok'
sample_test = "codec::round_trip"
output_filter = "failures-only"
filter_prefixes = ["cargo test", "cargo nextest"]
conventions = ["AGENTS.md", "CLAUDE.md"]
manifests = ["Cargo.toml", "crates/proto/Cargo.toml"]
[env]
CARGO_TARGET_DIR = "{worktree}/target"
"#;

pub(super) fn claude_route() -> Route {
    a_route(
        Runtime::Claude,
        Strength::Standard,
        Effort::High,
        "claude-sonnet-5",
    )
}

pub(super) fn a_profile() -> RepoProfile {
    toml::from_str(SPEC_PROFILE).unwrap()
}

pub(super) fn a_command_check(command: &str, ok: bool) -> CommandCheck {
    CommandCheck {
        command: command.into(),
        ok,
        code: Some(if ok { 0 } else { 101 }),
        timed_out: false,
        secs: 12,
        tail: "test result: ok".into(),
    }
}

pub(super) fn a_verification() -> ProfileVerification {
    ProfileVerification {
        at: 1_700_000_500,
        confined: true,
        setup: Some(a_command_check("cargo fetch", true)),
        check: Some(a_command_check("cargo test", false)),
        single_test: None,
        build_check: None,
        module_graph: None,
        module_test: None,
        module_tests: None,
        toolchain_id: None,
    }
}

pub(super) fn a_dropped() -> DroppedCommand {
    DroppedCommand {
        key: "check".into(),
        command: "cargo test".into(),
        reason: "exited 101".into(),
        tail: "error: could not compile".into(),
    }
}

pub(super) fn a_scout_info() -> ScoutInfo {
    ScoutInfo {
        id: "onboarding-1700000000".into(),
        kind: ScoutKind::Onboarding,
        question: "profile this repository".into(),
        state: ScoutState::Working,
        failure: None,
        window_id: Some(12),
        route: claude_route(),
        started_at: 1_700_000_000,
        ended_at: None,
        tool_calls: 9,
        report_bytes: Some(2048),
        files: vec!["Cargo.toml".into()],
        usage: a_token_usage(),
    }
}

pub(super) fn a_proposal() -> ProposalRecord {
    ProposalRecord {
        project: PathBuf::from("/tmp/p"),
        state: ProposalState::Failed {
            reason: "the scout timed out".into(),
        },
        origin: ProposalOrigin::Auto {
            stale: vec!["Cargo.toml".into()],
        },
        started_at: 1_700_000_000,
        updated_at: 1_700_000_100,
        base_sha: "0000000000000000000000000000000000000a".into(),
        scout_id: Some("onboarding-1700000000".into()),
        window_id: Some(12),
        profile: Some(a_profile()),
        verification: Some(a_verification()),
        dropped: vec![a_dropped()],
        proposed: Some(a_profile()),
        trusted_project: vec![".codex/config.toml".into()],
        unconfined_checks: true,
        auto_confirm: true,
    }
}

pub(super) fn a_triage() -> TriageInfo {
    TriageInfo {
        kinds: vec![TaskKind::Code],
        scale: Scale::Single,
        path: RunPath::Fast,
        reason: "one file, a clear change".into(),
        source: DeciderSource::Fallback,
        fallback_reason: Some("the decider timed out".into()),
        at: 1_700_000_200,
    }
}

pub(super) fn a_size_check() -> SizeCheckInfo {
    SizeCheckInfo {
        engine: Size::S,
        decided: Some(Size::M),
        agreed: false,
        reason: "two modules".into(),
        source: DeciderSource::Decider,
    }
}

pub(super) fn a_stats() -> HistoryStats {
    HistoryStats {
        path: PathBuf::from("/tmp/data/repos/p-1234/history.jsonl"),
        task_records: 4,
        run_records: 2,
        rows: vec![StatsRow {
            class: "S".into(),
            tasks: 3,
            merged: 2,
            median_lines: Some(14),
            median_tool_calls: Some(22),
            median_tokens: Some(40_000),
            median_work_secs: Some(310),
            bounces: 1,
            reverted: 0,
        }],
        decider_calls: 7,
        decider_fallbacks: 1,
        size_checked: 3,
        size_raised: 1,
        problems: vec!["line 9 does not parse".into()],
        flaky_proposals: Vec::new(),
        window_days: 0,
        quarantine_after: 0,
        rounds: 0,
        iterated_runs: 0,
        tuning: None,
    }
}

pub(super) fn a_task_record() -> TaskRecord {
    TaskRecord {
        v: HISTORY_VERSION,
        record_id: "run-a1b2/t1".into(),
        at: 1_700_000_900,
        run_id: "run-a1b2".into(),
        task_id: "t1".into(),
        path: Some(RunPath::Fast),
        kind: TaskKind::Code,
        hub: false,
        test_mode: TestMode::Tdd,
        planned_size: Size::S,
        final_size: Size::M,
        size_check: Some(a_size_check()),
        route: claude_route(),
        review_routes: vec![claude_route()],
        routing_decisions: vec![a_routing_decision()],
        outcome: TaskOutcome::MergedWithoutApproval,
        block: Some(BlockReason::Question),
        diff: Some(DiffStats {
            files: 2,
            hunks: 3,
            added: 40,
            removed: 5,
        }),
        tool_calls: 31,
        worker_usage: a_token_usage(),
        reviewer_usage: TokenUsage::default(),
        decider_usage: a_token_usage(),
        phases: PhaseSecs {
            queued: 1,
            preparing: 2,
            working: 3,
            proof: 4,
            check: 5,
            review: 6,
            merge: 7,
            blocked: 8,
        },
        wall_secs: 36,
        gates: GateTally {
            proofs: 1,
            checks: 2,
            checks_failed: 1,
            ..GateTally::default()
        },
        severities: SeverityTally {
            critical: 0,
            important: 1,
            minor: 2,
        },
        bounces: GateCounts::default(),
        failures: 1,
        stalls: 0,
        budget_exceeded: 0,
        conflicts: 0,
        max_rung: 1,
        sessions: 2,
        done_signal: Some(DoneSignal::TaskDone),
        merge_commit: Some("dddd4444".into()),
        stage: 1,
        origin: crate::tiers::TaskOrigin::Plan,
        pattern: None,
        race_winner: None,
        race_adopted: false,
        writer_failures: 0,
        round: 0,
    }
}

/// Decision 33a: an escalation whose chosen route is the second candidate.
pub(super) fn a_routing_decision() -> RoutingDecision {
    let codex = a_route(
        Runtime::Codex,
        Strength::Standard,
        Effort::High,
        "gpt-5-codex",
    );
    RoutingDecision {
        seq: 2,
        at: 1_700_000_500,
        role: AgentRole::Worker,
        session: 2,
        round: None,
        lane: None,
        trigger: "escalation".into(),
        source: "escalation_policy".into(),
        policy_version: "m8a-escalate-v1".into(),
        pick_policy: None,
        input: RoutingInput {
            title: "Reset token model".into(),
            brief: "Add the model.".into(),
            acceptance: vec!["tokens expire".into()],
            owns: vec!["crates/auth/**".into()],
            kind: TaskKind::Code,
            size: Size::S,
            hub: false,
            interface_change: true,
            test_mode: TestMode::Tdd,
            languages: vec!["rust".into()],
        },
        chosen: codex.clone(),
        selected_index: 1,
        candidates: vec![
            RoutingCandidate {
                route: claude_route(),
                skipped_reason: Some("effort is already high".into()),
            },
            RoutingCandidate {
                route: codex,
                skipped_reason: None,
            },
        ],
    }
}
