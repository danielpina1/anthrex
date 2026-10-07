//! The decider call (decision 16): one one-turn headless session, with no window and no
//! session kept, whose answer is validated against the kind's schema. Every failure is
//! the kind's deterministic fallback with one of decision 16's reasons, so a decider
//! never blocks a run and a fallback is never an error.
//!
//! The only I/O in `decider/`. No lock is held anywhere in the call. The schema file
//! write and the spawn (a `fork`/`exec` and thread starts) run on `spawn_blocking`; the
//! wait is an async receive on the session's event channel, bounded by the context's
//! timeout. Every return after the spawn kills the process group of the exact child
//! (`HeadlessHandle::kill`), which is a no-op once it has been reaped.

use super::argv::{DECIDER_CAPS, claude_decider_args, codex_decider_args, schema_file_name};
use super::fallback::{OFF_REASON, fallback_decision};
use super::parse::{STRUCTURED_OUTPUT_TOOL, answer_from_events, json_from_text, parse_for};
use super::{DeciderAnswer, DeciderContext, DeciderRequest, Decision, prompt, schema};
use crate::headless::codex_sandbox::DialectChoice;
use crate::headless::session::HeadlessHandle;
use crate::headless::{SessionEvent, TurnOutcome, claude_stream, credential_scrub_for};
use crate::manager::ManagerConfig;
use crate::run::driver::build::installed::installed_now;
use crate::run::model::FrozenList;
use crate::run::roster::{lowest_at_or_above, peer};
use crate::run::route_pick::{Installed, RolePick, role};
use anyhow::Context;
use proto::{DeciderMode, DeciderSource, Effort, ModelEntry, Route, Runtime, Strength, TokenUsage};
use serde_json::Value;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// The most assistant text a call keeps (only the last top-level text block is read,
/// cut to this many bytes on a character boundary).
pub const ANSWER_MAX_BYTES: usize = 256 * 1024;
/// `HeadlessHandle::kill`'s grace: `SIGTERM` to the group, then `SIGKILL` after it.
pub const KILL_GRACE: Duration = Duration::from_secs(2);
/// How long a decider that has answered gets to exit by itself before it is killed
/// (M8b.1: the real CLI exits 0.4 to 0.8 s after its `result` line). Never past the
/// call's own deadline.
pub const EXIT_WAIT: Duration = Duration::from_secs(2);

/// Milestone 9.5 (rulings RL-2, I6; decision 9a): what a decider call routes over,
/// fixed at daemon start: the `decider` list against the roster, the roster and
/// `[orchestrator.deciders] strength` for a route on the other runtime, the two binaries
/// the probe stats and each runtime's program, and the list's per-daemon rotation. The
/// list and roster are fixed together, as the deciders' own route always was (review
/// 10b, minor 6: never a live roster against a fixed list).
#[derive(Debug, Clone, Default)]
pub struct Routing {
    pub list: FrozenList,
    pub models: Vec<ModelEntry>,
    pub strength: Option<Strength>,
    pub bins: (String, String),
    pub decider_bin: Option<String>,
    pub rotation: Arc<AtomicU32>,
}

impl Routing {
    pub fn new(cfg: &config::Orchestrator, manager: &ManagerConfig) -> Routing {
        Routing {
            list: FrozenList::freeze(&cfg.tuning.routes.decider, &cfg.models),
            models: cfg.models.clone(),
            strength: Some(cfg.deciders.strength),
            bins: (manager.claude_bin.clone(), manager.codex_bin.clone()),
            decider_bin: manager.decider_bin.clone(),
            rotation: Arc::default(),
        }
    }

    /// `ANTHREX_DECIDER_BIN`, else `runtime`'s command.
    pub fn program(&self, runtime: Runtime) -> OsString {
        let command = match runtime {
            Runtime::Codex => &self.bins.1,
            _ => &self.bins.0,
        };
        OsString::from(self.decider_bin.as_ref().unwrap_or(command))
    }
}

/// Decision 16's route on `runtime`: the first roster entry at the lowest strength at or
/// above `strength`, else the runtime's first entry, else no model (the CLI's default).
pub fn ladder_route(
    models: &[ModelEntry],
    runtime: Runtime,
    strength: Strength,
    effort: Effort,
) -> Route {
    let entry = lowest_at_or_above(models, runtime, strength, None)
        .or_else(|| models.iter().find(|e| e.runtime == runtime));
    Route {
        runtime,
        model: entry.map(|e| e.model.clone()).unwrap_or_default(),
        strength: entry.map_or(strength, |e| e.strength),
        effort,
    }
}

/// One call's context, routed; the `decider` list's pick it came from (`None`: no
/// list, or the deciders are off); and, when the probe moved the call to the peer
/// runtime, the mode's route it moved from (ruling T10b-1).
#[derive(Debug, Clone)]
pub struct Routed {
    pub ctx: DeciderContext,
    pub pick: Option<RolePick>,
    pub moved: Option<Route>,
}

impl Routed {
    /// The strength the call's route was picked at (`Routing.strength`, frozen at
    /// daemon start), which its record's ladder lists from (review 10b's carry).
    pub fn strength(&self) -> proto::Strength {
        (self.ctx.routing.strength).unwrap_or(self.ctx.route.strength)
    }

    /// Ruling T10b-1: the run log's line for a call the probe moved to the peer
    /// runtime, `decider: <runtime> is not installed; using <peer>`.
    pub fn moved_line(&self) -> Option<String> {
        let (from, to) = (self.moved.as_ref()?.runtime, self.ctx.route.runtime);
        Some(format!(
            "decider: {} is not installed; using {}",
            from.label(),
            to.label()
        ))
    }
}

/// Rulings RL-2 and I6: the context of one decider call, routed over what is installed
/// now. The probe (`driver::build::installed`) runs at each call, on `spawn_blocking`
/// and bounded, never under a lock; deciders that are off are left as they are.
pub async fn routed(ctx: &DeciderContext) -> Routed {
    if ctx.mode == DeciderMode::Off {
        return Routed {
            ctx: ctx.clone(),
            pick: None,
            moved: None,
        };
    }
    let (claude, codex) = ctx.routing.bins.clone();
    route_over(ctx, &installed_now(claude, codex).await)
}

/// [`routed`]'s pure half. A `decider` list takes the place of the mode's runtime
/// choice: its pick over `installed`, rotating per daemon. With no list, or every
/// candidate skipped, the mode's route, on the other runtime when the probe found the
/// mode's not installed and the other one installed, an explicit `mode` included
/// (ruling T10b-1: a decider that cannot launch helps no one). `installed` empty: as
/// before.
pub fn route_over(ctx: &DeciderContext, installed: &Installed) -> Routed {
    let r = &ctx.routing;
    let rotation = match r.list.is_empty() {
        true => 0,
        false => r.rotation.fetch_add(1, Ordering::Relaxed),
    };
    let pick = role(&r.list, rotation, ctx.route.effort, installed);
    let missing = |rt: Runtime| installed.get(rt.label()) == Some(&false);
    let (runtime, other) = (ctx.route.runtime, peer(ctx.route.runtime));
    let peer_route = (missing(runtime) && !missing(other)).then(|| {
        let strength = r.strength.unwrap_or(ctx.route.strength);
        ladder_route(&r.models, other, strength, ctx.route.effort)
    });
    let listed = pick.as_ref().and_then(|p| p.route.clone());
    let moved = (listed.is_none() && peer_route.is_some()).then(|| ctx.route.clone());
    let route = (listed.or(peer_route)).unwrap_or_else(|| ctx.route.clone());
    let mut routed = ctx.clone();
    if route.runtime != ctx.route.runtime {
        routed.mode = match route.runtime {
            Runtime::Codex => DeciderMode::Codex,
            _ => DeciderMode::Claude,
        };
        routed.program = r.program(route.runtime);
    }
    routed.route = route;
    Routed {
        ctx: routed,
        pick,
        moved,
    }
}

/// Asks `request`'s decider (decision 16). Always returns a decision: the decider's
/// validated answer, or the fallback with its reason. `usage` is the turn's, whenever a
/// turn ended, even when its answer was then refused. `secs` is the call's duration.
pub async fn decide(ctx: &DeciderContext, request: &DeciderRequest) -> Decision {
    let started = Instant::now();
    let runtime = match ctx.mode {
        DeciderMode::Off => return fallback_decision(request, OFF_REASON.into()),
        DeciderMode::Claude => Runtime::Claude,
        DeciderMode::Codex => Runtime::Codex,
    };
    let (outcome, usage) = call(ctx, runtime, request).await;
    let mut decision = match outcome {
        Ok(answer) => Decision {
            kind: request.kind(),
            answer,
            source: DeciderSource::Decider,
            fallback_reason: None,
            usage: None,
            secs: 0,
        },
        Err(reason) => fallback_decision(request, reason),
    };
    decision.usage = usage;
    decision.secs = started.elapsed().as_secs();
    decision
}

/// [`routed`] then [`decide`], the whole of it within `bound` (the run title change's
/// `run_name` call, 15 s): the call's own timeout is at most `bound`, and a call still
/// running `bound` after it began (the probe included) is dropped, which kills its
/// process, for the fallback `the decider timed out after <bound> s`. With the deciders
/// off, the fallback at once. `None`: the probe itself outlasted `bound`, so there is no
/// route to record.
pub async fn decide_within(
    ctx: &DeciderContext,
    request: &DeciderRequest,
    bound: Duration,
) -> (Option<Routed>, Decision) {
    if ctx.mode == DeciderMode::Off {
        return (None, fallback_decision(request, OFF_REASON.into()));
    }
    let started = Instant::now();
    let timed_out = || {
        let mut decision = fallback_decision(
            request,
            format!("the decider timed out after {} s", bound.as_secs()),
        );
        decision.secs = started.elapsed().as_secs();
        decision
    };
    let deadline = tokio::time::Instant::now() + bound;
    let Ok(routed) = tokio::time::timeout_at(deadline, routed(ctx)).await else {
        return (None, timed_out());
    };
    let mut bounded = routed.ctx.clone();
    bounded.timeout = bounded.timeout.min(bound);
    let decision = match tokio::time::timeout_at(deadline, decide(&bounded, request)).await {
        Ok(decision) => decision,
        Err(_) => timed_out(),
    };
    (Some(routed), decision)
}

/// The answer or the fallback reason, and the turn's usage if a turn ended.
async fn call(
    ctx: &DeciderContext,
    runtime: Runtime,
    request: &DeciderRequest,
) -> (Result<DeciderAnswer, String>, Option<TokenUsage>) {
    let (tx, mut events) = mpsc::unbounded_channel::<SessionEvent>();
    let mut spawn_ctx = ctx.clone();
    if runtime == Runtime::Codex {
        // Final review I1 (ruling R6): the sandbox dialect is the probed Codex
        // version's, so wait for the probe (the launch gate; no lock is held), then fix
        // the dialect for the blocking argv build.
        ctx.launch_gate.wait().await;
        spawn_ctx.caps.codex_sandbox = DialectChoice::Fixed(ctx.caps.codex_dialect());
    }
    let spawn_request = request.clone();
    let spawned = tokio::task::spawn_blocking(move || {
        start(&spawn_ctx, runtime, &spawn_request, move |_pid, event| {
            // The receiver is gone only once the call has returned (and killed it).
            let _ = tx.send(event);
        })
    })
    .await;
    // From here every way out of the call, including its future being dropped, kills
    // the decider: `guard` does it on drop, and the explicit paths call `kill` first.
    let guard = match spawned {
        Ok(Ok(guard)) => guard,
        Ok(Err(error)) => return (Err(format!("the decider could not start: {error:#}")), None),
        Err(error) => return (Err(format!("the decider could not start: {error}")), None),
    };

    let deadline = tokio::time::Instant::now() + ctx.timeout;
    let mut seen = Seen::default();
    let ended = loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Err(_) => {
                guard.kill();
                let secs = ctx.timeout.as_secs();
                return (Err(format!("the decider timed out after {secs} s")), None);
            }
            Ok(None) => {
                // The dispatcher delivers `ProcessExited` before it ends; defensive.
                guard.kill();
                return (Err(exited(None, None)), None);
            }
            Ok(Some(SessionEvent::ProcessExited { code, signal })) => {
                guard.kill();
                return (Err(exited(code, signal)), None);
            }
            Ok(Some(SessionEvent::TurnEnded { outcome, usage, .. })) => break (outcome, usage),
            Ok(Some(event)) => seen.keep(event),
        }
    };

    // The answer is in; give the process a moment to exit by itself, then make sure.
    let exit_by = deadline.min(tokio::time::Instant::now() + EXIT_WAIT);
    while let Ok(Some(event)) = tokio::time::timeout_at(exit_by, events.recv()).await {
        if matches!(event, SessionEvent::ProcessExited { .. }) {
            break;
        }
    }
    guard.kill();

    let (outcome, usage) = ended;
    (judge(request, &seen.events(), outcome), usage)
}

/// Writes the schema file (Codex) and starts the session; the Claude prompt is sent as
/// one stream-json user message and stdin is closed. Blocking.
fn start(
    ctx: &DeciderContext,
    runtime: Runtime,
    request: &DeciderRequest,
    on_event: impl Fn(u32, SessionEvent) + Send + Sync + 'static,
) -> anyhow::Result<KillOnDrop> {
    let kind = request.kind();
    let schema = schema::schema(kind);
    let prompt = prompt::render(request);
    std::fs::create_dir_all(&ctx.cwd)?;
    let args = match runtime {
        Runtime::Codex => {
            let file = ctx.schema_dir.join(schema_file_name(kind, &schema));
            if DECIDER_CAPS.codex_output_schema {
                write_schema(&file, &schema)?;
            }
            codex_decider_args(ctx, &DECIDER_CAPS, &file, &prompt)
        }
        _ => claude_decider_args(ctx, &DECIDER_CAPS, &schema),
    };
    let remove = credential_scrub_for(runtime, config::ClaudeAuth::Login);
    let guard = KillOnDrop::new(HeadlessHandle::spawn(
        runtime,
        &ctx.program,
        &args,
        &ctx.cwd,
        &[],
        &remove,
        on_event,
    )?);
    if runtime == Runtime::Claude {
        let handle = guard.handle();
        let sent = handle.send_line(claude_stream::user_message(&prompt, None));
        handle.close_stdin();
        // On an error `guard` is dropped here, which kills the process.
        sent.context("could not send the prompt")?;
    }
    Ok(guard)
}

/// The decider's process, killed when this is dropped: when the call's future is
/// dropped (a caller's `timeout` or `select!`, an aborted task, a runtime shutting
/// down) or unwinds from a panic, as on every explicit return (decision 16).
///
/// `HeadlessHandle::kill` never blocks (it sends `SIGTERM` and waits out its grace on a
/// thread of its own), so dropping this on a tokio worker, or while the runtime shuts
/// down, is safe and needs no runtime.
struct KillOnDrop(Option<HeadlessHandle>);

impl KillOnDrop {
    fn new(handle: HeadlessHandle) -> Self {
        Self(Some(handle))
    }

    fn handle(&self) -> &HeadlessHandle {
        self.0.as_ref().expect("armed until killed")
    }

    /// Kills now and disarms the guard.
    fn kill(mut self) {
        if let Some(handle) = self.0.take() {
            handle.kill(KILL_GRACE);
        }
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.kill(KILL_GRACE);
        }
    }
}

/// Writes `schema` to `file` once: a file already there (same name, same schema hash)
/// is kept.
fn write_schema(file: &Path, schema: &Value) -> std::io::Result<()> {
    if file.is_file() {
        return Ok(());
    }
    crate::profile::store::write_atomic(file, schema.to_string().as_bytes())
}

/// A turn's answer, in decision 16's order. A failed turn is a fallback unless its
/// events, or its result text, still hold JSON (ruling R-T1-4: a model that answered in
/// text and then ran into `--max-turns`).
fn judge(
    request: &DeciderRequest,
    events: &[SessionEvent],
    outcome: TurnOutcome,
) -> Result<DeciderAnswer, String> {
    let schema_error = |e| format!("the decider's answer does not match the schema: {e}");
    match outcome {
        TurnOutcome::Completed => {
            let value = answer_from_events(events)
                .map_err(|e| format!("the decider's answer is not JSON: {e}"))?;
            parse_for(request, &value).map_err(schema_error)
        }
        TurnOutcome::Failed { error, .. } => {
            match answer_from_events(events).or_else(|_| json_from_text(&error)) {
                Ok(value) => parse_for(request, &value).map_err(schema_error),
                Err(_) => Err(format!("the decider's turn failed: {error}")),
            }
        }
        TurnOutcome::Interrupted => Err("the decider's turn failed: interrupted".into()),
    }
}

fn exited(code: Option<i32>, signal: Option<i32>) -> String {
    let code = match (code, signal) {
        (Some(code), _) => code.to_string(),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "unknown".into(),
    };
    format!("the decider exited before answering (code {code})")
}

/// The events an answer can come from, the last of each kind only, so a call's memory
/// is bounded whatever the decider prints.
#[derive(Default)]
struct Seen {
    structured: Option<SessionEvent>,
    tool: Option<SessionEvent>,
    text: Option<SessionEvent>,
}

impl Seen {
    fn keep(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::StructuredOutput { .. } => self.structured = Some(event),
            SessionEvent::ToolUse {
                ref name,
                parent: None,
                ..
            } if name == STRUCTURED_OUTPUT_TOOL => self.tool = Some(event),
            SessionEvent::AssistantText { text, parent: None } => {
                self.text = Some(SessionEvent::AssistantText {
                    text: proto::conversation::truncate_to_char_boundary(&text, ANSWER_MAX_BYTES),
                    parent: None,
                });
            }
            _ => {}
        }
    }

    fn events(self) -> Vec<SessionEvent> {
        [self.text, self.tool, self.structured]
            .into_iter()
            .flatten()
            .collect()
    }
}
