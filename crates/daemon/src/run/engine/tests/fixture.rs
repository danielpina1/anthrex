//! The engine test fixture (milestone brief, "Shared test helpers"): builds a run through
//! `plan::build_run` with a preflight whose three paths all differ, drives `step` with
//! events and inspects the returned effects. No process, no git, no clock.

use std::path::PathBuf;

use proto::TaskState;

use crate::run::engine::{
    AgentSignal, EngineState, Event, EventKind, OpKind, OpResult, ReplyId, step,
};
use crate::run::model::{OpId, Run, Task};
use crate::run::plan::{BuildContext, Preflight, build_run, parse_plan};

pub use crate::run::test_support::{plan_with, task_toml};

/// `b0` × 20: the base commit.
pub const BASE: &str = "b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0";
pub const RUN_ID: &str = "engine-test-3f9a";
/// A task head after its work (M8a.12).
pub const HEAD: &str = "d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1";
pub const H4: &str = "3f9a";
/// Distinct from every path the preflight has (`/tmp/x`, `/tmp/p`, `/tmp/p/.git`).
pub const WT: &str = "/tmp/wt";

/// Every key set, so a test that wants one missing overrides it explicitly. `setup` and
/// an `env` with `{worktree}` let tests see both reach the ops.
pub const PROFILE: &str = r#"
goal = "Engine test"

[profile]
modules = ["crates/*"]
hub = ["crates/proto/**"]
source = ["crates/*/src/**"]
check = "cargo test"
single_test = "cargo test -- --exact {test}"
setup = "make deps"
[profile.env]
TARGET = "{worktree}/target"
"#;

/// `PROFILE` with plan-level limits spliced in before `[profile]`.
pub fn profile_with(limits: &str) -> String {
    PROFILE.replacen("\n[profile]", &format!("\n{limits}\n[profile]"), 1)
}

/// A task owning `crates/<module>/**`, with `extra` TOML lines.
pub fn task(id: &str, size: &str, module: &str, extra: &str) -> String {
    task_toml(id, size, &format!("[\"crates/{module}/**\"]"), extra)
}

pub fn preflight() -> Preflight {
    Preflight {
        root: PathBuf::from("/tmp/x"),
        project: PathBuf::from("/tmp/p"),
        git_common_dir: PathBuf::from("/tmp/p/.git"),
        base_branch: "main".to_string(),
        base_sha: BASE.to_string(),
        protected_files: Vec::new(),
    }
}

pub fn build(plan_toml: &str, config: &config::Orchestrator, yes: bool) -> Run {
    let plan = parse_plan(plan_toml).unwrap_or_else(|e| panic!("fixture plan: {e}"));
    build_run(
        plan,
        preflight(),
        BuildContext {
            id: RUN_ID.to_string(),
            wt_dir: PathBuf::from(WT),
            data_dir: PathBuf::from(format!("/tmp/data/runs/{RUN_ID}")),
            config,
            now: 1_000,
            yes,
        },
    )
    .unwrap_or_else(|e| panic!("fixture run: {e:?}"))
}

pub struct Fixture {
    pub state: EngineState,
    pub plan: String,
    pub config: config::Orchestrator,
    pub now: u64,
    /// Every effect every step returned, in order.
    pub log: Vec<crate::run::engine::Effect>,
    next_reply: ReplyId,
    next_window: u32,
}

impl Fixture {
    pub fn new(plan_toml: &str) -> Self {
        Self::with_config(plan_toml, config::Orchestrator::default())
    }

    pub fn with_config(plan_toml: &str, config: config::Orchestrator) -> Self {
        Fixture {
            state: EngineState::default(),
            plan: plan_toml.to_string(),
            config,
            now: 2_000,
            log: Vec::new(),
            next_reply: 1,
            next_window: 1,
        }
    }

    pub fn reply(&mut self) -> ReplyId {
        self.next_reply += 1;
        self.next_reply
    }

    pub fn send(&mut self, now: u64, kind: EventKind) -> Vec<crate::run::engine::Effect> {
        self.now = now;
        let state = std::mem::take(&mut self.state);
        let (state, fx) = step(state, Event { now, kind });
        self.state = state;
        self.log.extend(fx.iter().cloned());
        fx
    }

    /// The next step one second later.
    pub fn next(&mut self, kind: EventKind) -> Vec<crate::run::engine::Effect> {
        self.send(self.now + 1, kind)
    }

    pub fn start(&mut self, yes: bool) -> Vec<crate::run::engine::Effect> {
        let run = build(&self.plan, &self.config, yes);
        let reply = self.reply();
        self.next(EventKind::Start {
            reply,
            run: Box::new(run),
        })
    }

    /// Start, then complete the run branch.
    pub fn ready(&mut self, yes: bool) -> Vec<crate::run::engine::Effect> {
        self.start(yes);
        let (op, _) = self.op("CreateRunBranch");
        self.done(op, OpResult::Worktree { head: BASE.into() })
    }

    pub fn approve(&mut self) -> Vec<crate::run::engine::Effect> {
        let reply = self.reply();
        self.next(EventKind::Approve {
            reply,
            run_id: RUN_ID.into(),
        })
    }

    pub fn tick(&mut self) -> Vec<crate::run::engine::Effect> {
        self.next(EventKind::Tick)
    }

    pub fn done(&mut self, op: OpId, result: OpResult) -> Vec<crate::run::engine::Effect> {
        self.next(EventKind::OpDone {
            run_id: RUN_ID.into(),
            op,
            result,
        })
    }

    pub fn signal(&mut self, window: u32, signal: AgentSignal) -> Vec<crate::run::engine::Effect> {
        self.next(EventKind::Signal {
            window_id: window,
            signal,
        })
    }

    pub fn run(&self) -> &Run {
        self.state.runs.get(RUN_ID).expect("the fixture run exists")
    }

    pub fn run_mut(&mut self) -> &mut Run {
        self.state
            .runs
            .get_mut(RUN_ID)
            .expect("the fixture run exists")
    }

    pub fn task(&self, id: &str) -> &Task {
        self.run()
            .task(id)
            .unwrap_or_else(|| panic!("no task {id}"))
    }

    pub fn task_mut(&mut self, id: &str) -> &mut Task {
        self.run_mut()
            .tasks
            .iter_mut()
            .find(|t| t.spec.id == id)
            .unwrap_or_else(|| panic!("no task {id}"))
    }

    /// The latest `Effect::Op` of this kind in the whole log.
    pub fn op(&self, name: &str) -> (OpId, OpKind) {
        self.ops(name)
            .pop()
            .unwrap_or_else(|| panic!("no {name} op in {:#?}", self.log))
    }

    /// Every `Effect::Op` of this kind in the whole log, oldest first.
    pub fn ops(&self, name: &str) -> Vec<(OpId, OpKind)> {
        ops_in(&self.log, name)
    }

    /// Completes every pending `PrepareWorktree` with its own `from` as the head.
    pub fn complete_prepares(&mut self) -> Vec<crate::run::engine::Effect> {
        let pending: Vec<(OpId, String)> = self
            .run()
            .pending_ops
            .values()
            .filter_map(|p| match &p.kind {
                OpKind::PrepareWorktree { from, .. } => Some((p.op, from.clone())),
                _ => None,
            })
            .collect();
        let mut out = Vec::new();
        for (op, from) in pending {
            out.extend(self.done(op, OpResult::Worktree { head: from }));
        }
        out
    }

    /// Completes every pending `CreateWindow`, giving each a fresh window id; returns
    /// `(task, window)` pairs.
    pub fn complete_windows(&mut self) -> Vec<(String, u32)> {
        let pending: Vec<(OpId, String)> = self
            .run()
            .pending_ops
            .values()
            .filter(|p| matches!(p.kind, OpKind::CreateWindow { .. }))
            .map(|p| (p.op, p.task_id.clone().unwrap_or_default()))
            .collect();
        let mut out = Vec::new();
        for (op, task) in pending {
            let window_id = self.next_window;
            self.next_window += 1;
            self.done(op, OpResult::Window { window_id });
            out.push((task, window_id));
        }
        out
    }

    /// Prepares and launches everything the scheduler has started.
    pub fn launch_all(&mut self) -> Vec<(String, u32)> {
        self.complete_prepares();
        self.complete_windows()
    }

    /// A worker's MCP tool call from `window` for `t1` (M8a.12).
    pub fn tool(
        &mut self,
        window: u32,
        tool: &str,
        args: serde_json::Value,
    ) -> Vec<crate::run::engine::Effect> {
        self.tool_as(proto::AgentRole::Worker, window, "t1", tool, args)
    }

    pub fn tool_as(
        &mut self,
        role: proto::AgentRole,
        window: u32,
        task: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Vec<crate::run::engine::Effect> {
        let reply = self.reply();
        self.next(EventKind::Tool {
            reply,
            call: proto::ToolCall {
                run_id: RUN_ID.into(),
                task_id: Some(task.into()),
                role,
                window_id: window,
                tool: tool.into(),
                args,
            },
        })
    }

    /// A `TurnEnded` with `outcome`, no usage and no denials.
    pub fn turn_ended(
        &mut self,
        window: u32,
        outcome: crate::run::engine::TurnOutcome,
    ) -> Vec<crate::run::engine::Effect> {
        self.signal(
            window,
            AgentSignal::TurnEnded {
                outcome,
                usage: None,
                denials: vec![],
            },
        )
    }

    /// A completed turn.
    pub fn turn_completed(&mut self, window: u32) -> Vec<crate::run::engine::Effect> {
        self.turn_ended(window, crate::run::engine::TurnOutcome::Completed)
    }

    /// `DoneChecked` for task `id` with nothing wrong: two commits on its branch, a clean
    /// tree, the red commit valid.
    pub fn clean_check(&self, id: &str) -> OpResult {
        OpResult::DoneChecked {
            commits: 2,
            dirty_tracked: 0,
            merge_in_progress: false,
            untracked_in_owns: Vec::new(),
            outside_owns: Vec::new(),
            generated_outside_owns: Vec::new(),
            protected_changed: Vec::new(),
            red_ok: Some(true),
            head: HEAD.into(),
            head_branch: Some(self.task(id).branch.clone()),
            resolution_only: None,
        }
    }

    /// Stands in for M8a.14's merge queue: the task is merged at `commit`, which becomes
    /// the run head, then a `Tick` lets the scheduler react.
    pub fn merge(&mut self, id: &str, commit: &str) -> Vec<crate::run::engine::Effect> {
        let task = self.task_mut(id);
        task.state = TaskState::Merged;
        task.merge_commit = Some(commit.to_string());
        for round in &mut task.rounds {
            round.ended = true;
            round.turn_open = false;
        }
        self.run_mut().run_head = commit.to_string();
        self.tick()
    }

    /// Stands in for the gates: the task reaches `state`, then a `Tick`.
    pub fn force(&mut self, id: &str, state: TaskState) -> Vec<crate::run::engine::Effect> {
        self.task_mut(id).state = state;
        self.tick()
    }

    /// Stands in for M8a.13's approving verdict: every reviewer round of the task ends
    /// and it moves to the merge queue. (A round that ends while the task is still in
    /// `review` gets a fresh round, decision 35.)
    pub fn end_review(&mut self, id: &str) {
        let task = self.task_mut(id);
        task.state = TaskState::MergeQueue;
        for round in &mut task.rounds {
            if round.role == proto::AgentRole::Reviewer {
                round.ended = true;
            }
        }
    }
}

pub fn op_name(kind: &OpKind) -> &'static str {
    match kind {
        OpKind::CreateRunBranch { .. } => "CreateRunBranch",
        OpKind::PrepareWorktree { .. } => "PrepareWorktree",
        OpKind::CreateWindow { .. } => "CreateWindow",
        OpKind::ResumeSession { .. } => "ResumeSession",
        OpKind::VerifyDone { .. } => "VerifyDone",
        OpKind::CountCommits { .. } => "CountCommits",
        OpKind::DiffSoFar { .. } => "DiffSoFar",
        OpKind::Proof { .. } => "Proof",
        OpKind::Check { .. } => "Check",
        OpKind::PrepareReview { .. } => "PrepareReview",
        OpKind::MergeCandidate { .. } => "MergeCandidate",
        OpKind::HandBack { .. } => "HandBack",
        OpKind::AbortMerge { .. } => "AbortMerge",
        OpKind::RemoveWorktree { .. } => "RemoveWorktree",
        OpKind::VerifyRefs { .. } => "VerifyRefs",
        OpKind::Accept { .. } => "Accept",
        OpKind::Discard { .. } => "Discard",
    }
}

pub fn ops_in(effects: &[crate::run::engine::Effect], name: &str) -> Vec<(OpId, OpKind)> {
    effects
        .iter()
        .filter_map(|e| match e {
            crate::run::engine::Effect::Op { op, kind, .. } if op_name(kind) == name => {
                Some((*op, kind.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The task a `PrepareWorktree` or `CreateWindow` op is for, read from its path.
pub fn op_task(kind: &OpKind) -> String {
    let path = match kind {
        OpKind::PrepareWorktree { path, .. } => path.clone(),
        OpKind::CreateWindow { worktree, .. } => worktree.clone(),
        OpKind::PrepareReview { path, .. } => path.clone(),
        other => panic!("op_task of {other:?}"),
    };
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    name.trim_end_matches(".review").to_string()
}

/// The tasks of every op of `name` in `effects`, in order.
pub fn tasks_of(effects: &[crate::run::engine::Effect], name: &str) -> Vec<String> {
    ops_in(effects, name)
        .iter()
        .map(|(_, k)| op_task(k))
        .collect()
}
