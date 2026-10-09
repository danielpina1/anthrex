//! Fixture builders for `run_tests.rs`'s round trips: one of each run snapshot and
//! wire shape, every field set. Split out of `run_tests.rs` (review E-M2, F4).

use super::*;

pub(super) fn a_finding() -> Finding {
    Finding {
        severity: Severity::Important,
        file: Some("crates/auth/src/token.rs".into()),
        line: Some(42),
        input: Some("token::expires_after_one_hour".into()),
        text: "off by one on the expiry check".into(),
    }
}

pub(super) fn a_route(runtime: Runtime, effort: Effort, model: &str) -> Route {
    Route {
        runtime,
        model: model.into(),
        effort,
    }
}

pub(super) fn a_token_usage() -> TokenUsage {
    TokenUsage {
        input: 111,
        output: 222,
        cache_read: 4000,
        cache_write: 333,
    }
}

pub(super) fn an_agent_round() -> AgentRoundInfo {
    AgentRoundInfo {
        role: AgentRole::Worker,
        // `session`, `round`, `window_id`, `tool_calls`, `turns`, `open_subagents` and
        // `denials` are all `u32`; each gets its own value so a swap between any two of
        // them (for instance `session`/`open_subagents`) changes the round trip's
        // result rather than surviving it.
        session: 1,
        round: 6,
        window_id: Some(9),
        route: a_route(Runtime::Claude, Effort::HIGH, "claude-sonnet-5"),
        session_id: Some("claude-session-abc".into()),
        started_at: 1_700_000_100,
        ended_at: Some(1_700_000_400),
        tool_calls: 7,
        last_event: 1_700_000_390,
        turn_open: false,
        turns: 3,
        rate_limited: false,
        open_subagents: 4,
        denials: 5,
        usage: a_token_usage(),
        // Milestone 8c's fields, unset; `run_tests_view.rs` sets each.
        rate_limited_since: None,
        rate_limited_until: None,
        sent_back_at: Vec::new(),
        lane: None,
        failed_error: None,
        failed_until: None,
    }
}

pub(super) fn a_review() -> ReviewInfo {
    ReviewInfo {
        round: 1,
        route: a_route(Runtime::Codex, Effort::MEDIUM, ""),
        verdict: Some(Verdict::Changes),
        summary: "one blocking finding".into(),
        findings: vec![a_finding()],
        blocking: true,
        lane: None,
    }
}

pub(super) fn a_task_info() -> TaskInfo {
    TaskInfo {
        actions: Vec::new(),
        id: "t1".into(),
        title: "Reset token model".into(),
        epic: Some("auth".into()),
        kind: TaskKind::Code,
        size: Size::M,
        hub: false,
        test_mode: TestMode::Tdd,
        test_mode_reason: None,
        notes: vec!["size raised from S to M: owns spans 2 modules (rule 7.2.1)".into()],
        owns: vec!["crates/auth/src/token.rs".into()],
        deps: vec!["t0".into()],
        implicit_deps: vec!["t0a".into()],
        priority: 3,
        route: a_route(Runtime::Claude, Effort::HIGH, "claude-sonnet-5"),
        review_route: Some(a_route(Runtime::Codex, Effort::MEDIUM, "")),
        budget: Budget {
            tool_calls: 120,
            minutes: 45,
            tokens: Some(3_000_000),
        },
        spent_session: Spend {
            tool_calls: 4,
            secs: 90,
            tokens: 1200,
        },
        spent_total: Spend {
            tool_calls: 11,
            secs: 400,
            tokens: 5600,
        },
        state: TaskState::Review,
        block: Some(BlockInfo {
            reason: BlockReason::Question,
            text: "waiting on an answer about the token TTL".into(),
        }),
        // `rung`, `failures`, `stalls`, `budget_exceeded` and `conflicts` are all `u8`;
        // each gets its own value so a swap between any two of them survives neither
        // this fixture nor the by-name assertions below.
        rung: 3,
        failures: 5,
        bounces: GateCounts {
            // `done`, `proof`, `check`, `review` and `merge` are all `u8`, same
            // reasoning as above.
            done: 1,
            proof: 0,
            check: 4,
            review: 2,
            merge: 3,
        },
        stalls: 2,
        budget_exceeded: 0,
        conflicts: 4,
        branch: "anthrex/run-a1b2/t1".into(),
        worktree: PathBuf::from("/tmp/wt/runs/run-a1b2/t1"),
        start_commit: Some("aaaa1111".into()),
        head: Some("bbbb2222".into()),
        test: Some("token::expires_after_one_hour".into()),
        red: Some("cccc3333".into()),
        done_signal: Some(DoneSignal::TaskDone),
        rounds: vec![an_agent_round()],
        reviews: vec![a_review()],
        last_check: None,
        last_proof: None,
        merge_commit: None,
        merged_without_approval: None,
        salvage_refs: vec!["refs/anthrex/salvage/run-a1b2/t1/1".into()],
        on_critical_path: true,
        wave: 2,
        history: vec![TaskEventInfo {
            at: 1_700_000_040,
            text: "review round 1 requested changes".into(),
        }],
        // Milestone 8b's fields, unset as in a milestone-8a snapshot; `adapt_tests.rs`
        // sets each.
        decider_usage: None,
        size_check: None,
        diff: None,
        phases: None,
        block_source: None,
        brief: String::new(),
        acceptance: Vec::new(),
        route_spec: RouteSpec::default(),
        row: None,
        hold: None,
        review_target: None,
        research_bytes: None,
        message_count: 0,
        last_message_kind: None,
        last_message_line: None,
        task_notes: Vec::new(),
        stage: 1,
        origin: crate::tiers::TaskOrigin::Plan,
        fixes: None,
        tier: None,
        weakening: Vec::new(),
        activity: None,
        atomic: false,
        atomic_reason: None,
        interface_change: false,
        round: 1,
        race: None,
        pair: None,
        covers: Vec::new(),
    }
}

pub(super) fn a_run_info() -> RunInfo {
    let mut rate_limits = BTreeMap::new();
    rate_limits.insert("claude".to_string(), 2u32);
    rate_limits.insert("codex".to_string(), 0u32);
    RunInfo {
        actions: Vec::new(),
        run_id: "run-a1b2".into(),
        goal: "Add password reset".into(),
        project: PathBuf::from("/tmp/p"),
        root: PathBuf::from("/tmp/x"),
        state: RunState::Running,
        paused_from: None,
        halted_reason: None,
        approved_by: Some("user".into()),
        base_branch: "main".into(),
        base_sha: "0000000000000000000000000000000000000a".into(),
        run_branch: "anthrex/run-a1b2/integration".into(),
        run_head: "1111111111111111111111111111111111111b".into(),
        base_moved: Some(BaseMovedInfo {
            from: "0000000000000000000000000000000000000a".into(),
            to: "2222222222222222222222222222222222222c".into(),
            commits: vec!["abc1234 alice: fix the thing".into()],
            total: 3,
        }),
        revision: 42,
        max_writers: 4,
        max_readers: 2,
        max_bounces: 3,
        writers_busy: 1,
        readers_busy: 0,
        unverified: false,
        worker_sandbox: true,
        unconfined_checks: true,
        trusted_project: vec![".codex/config.toml".into()],
        rate_limits,
        tasks: vec![a_task_info()],
        critical_path: vec!["t1".into()],
        attention: vec![
            "base main moved from 0000000 to 2222222 (3 new commits); accept will list them".into(),
        ],
        report_path: PathBuf::from("/tmp/data/runs/run-a1b2/REPORT.md"),
        outcome: None,
        created_at: 1_700_000_000,
        path: None,
        triage: None,
        promote_requested_at: None,
        profile_source: None,
        usage: None,
        scouts: Vec::new(),
        approved_at: None,
        plan_edits: Vec::new(),
        plan_edits_since_approval: 0,
        planners: Vec::new(),
        estimate_left_secs: None,
        bound_ratio_permille: None,
        orchestrator: None,
        holds: Vec::new(),
        integration: Vec::new(),
        digest_revision: 0,
        research_report: None,
        stages: Vec::new(),
        test_slots: 0,
        delivery: None,
        chain: None,
        round: 1,
        rounds: Vec::new(),
        writer_caps: Default::default(),
        design: Default::default(),
        doc_gate: None,
        docs: Vec::new(),
        round_design: None,
        halted_phase: None,
        design_agents: Vec::new(),
        title: String::new(),
    }
}

pub(super) fn a_tool_call() -> ToolCall {
    ToolCall {
        run_id: "run-a1b2".into(),
        task_id: Some("t1".into()),
        role: AgentRole::Worker,
        window_id: 9,
        tool: "task_done".into(),
        args: serde_json::json!({"summary": "ok"}),
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
    }
}
