//! Milestone 9.6 decision 9, the driver's half (task M9.6.8): `OpKind::StartDesignAgent`
//! executed as a sub-planner's start is (`orch_ops.rs::start_planner`). A brainstormer's
//! first turn gets decision 11's input pack, read off the engine; each folder its
//! Claude sandbox denies is denied by its canonical path too (ruling T8-3); the session starts on
//! the scout service; its end is sent as `OrchEvent::DesignAgentEnded`, with its usage
//! and tool calls, after the op's result reached the engine.
//!
//! No lock is held across an await: the engine's state is looked up and cloned under
//! its lock, and the files (the stored profile, the frozen scout reports, a continued
//! goal's previous spec and a rethink's previous report, each checked against its
//! frozen index entry) are read on
//! `spawn_blocking`, within `CONTEXT_READ_TIMEOUT` and `IO_WAIT` (AGENTS.md rule 2).

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use proto::{Runtime, TokenUsage};

use super::design_io::{
    DOC_READ_CAP, IO_WAIT, ReadError, cut_text, read_checked, read_stored, write_new,
};
use super::ops::failed;
use super::orch::{CONTEXT_READ_TIMEOUT, context_reads};
use super::{OpCtx, RunService};
use crate::run::design::pack::{
    Earlier, EarlierSpec, EarlierText, FrozenRethink, PACK_MAX, PackFile, PackInputs, RethinkInput,
    pack, pack_path,
};
use crate::run::design::state::sha256_hex;
use crate::run::engine::{EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::scout::design_spec::{DesignAgentKind, DesignAgentSpec};
use crate::scout::service::{ScoutHandle, ScoutOutcome, ScoutService};

/// Decision 30 and ruling T15-1: the frozen earlier spec read back, round 1's and each
/// approved amendment, each against its index entry. What cannot be read in time is left
/// out (without round 1's, the whole block is).
async fn earlier_read(run_id: &str, earlier: EarlierSpec) -> Option<Earlier> {
    let Some(text) = stored_text(earlier.path.clone(), earlier.version).await else {
        tracing::warn!(run = %run_id, "the earlier spec is left out");
        return None;
    };
    let mut read = Earlier {
        path: earlier.path,
        text,
        own: earlier.run == run_id,
        amendments: Vec::new(),
    };
    for a in earlier.amendments {
        let text = stored_text(a.path.clone(), a.version).await;
        if text.is_none() {
            let k = a.round;
            tracing::warn!(run = %run_id, "the round {k} amendment could not be read back");
        }
        read.amendments.push(EarlierText {
            round: a.round,
            path: a.path,
            unread: text.is_none(),
            text,
        });
    }
    Some(read)
}

/// A stored document's text, read on `spawn_blocking` within `IO_WAIT` against its
/// index entry, cut at `DOC_READ_CAP`.
async fn stored_text(
    path: PathBuf,
    version: crate::run::design::state::DocVersion,
) -> Option<String> {
    let read = tokio::task::spawn_blocking(move || read_stored(&path, &version));
    match tokio::time::timeout(IO_WAIT, read).await {
        Ok(Ok(Ok(bytes))) => Some(cut_text(&bytes, DOC_READ_CAP, "cut")),
        _ => None,
    }
}

impl RunService {
    /// Decision 9: design agent `spec` on the scout machine, a brainstormer's first turn
    /// with the input pack appended; its end is sent once this op's result is in.
    pub(super) async fn start_design_agent(
        self: &Arc<Self>,
        ctx: &OpCtx,
        mut spec: DesignAgentSpec,
    ) -> OpResult {
        // The pack first: one that cannot be read halts the run, whatever else fails.
        let mut pack_file = None;
        if matches!(spec.kind, DesignAgentKind::Brainstormer { .. }) {
            let (pack, file) = match self.brainstorm_pack(&ctx.run_id).await {
                Ok(read) => read,
                Err(reason) => return OpResult::DesignPackUnreadable { reason },
            };
            pack_file = file;
            spec.first_turn = format!("{}\n\n{pack}", spec.first_turn);
        }
        let Some(scouts) = self.adaptation.get().map(|a| a.scouts.clone()) else {
            return failed("the scout service is not running");
        };
        deny_codex_sessions(&mut spec, codex_sessions_dir(|k| std::env::var_os(k)));
        if let Some(sandbox) = spec.headless.claude_sandbox.as_mut() {
            let given = std::mem::take(&mut sandbox.deny_read);
            sandbox.deny_read = with_canonical(given).await;
        }
        let (kind, session) = (spec.kind.clone(), spec.session);
        match scouts.start_design_agent(spec).await {
            Ok(handle) => {
                let window_id = handle.window_id;
                let (service, run_id) = (self.clone(), ctx.run_id.clone());
                tokio::spawn(async move {
                    let (outcome, usage, calls) = ended(&scouts, handle).await;
                    let k = kind.clone();
                    service
                        .settled(&run_id, move |op| {
                            matches!(op, OpKind::StartDesignAgent { spec }
                                if spec.kind == k && spec.session == session)
                        })
                        .await;
                    service.send(EventKind::Orch(OrchEvent::DesignAgentEnded {
                        run_id,
                        role: kind.role(),
                        label: kind.label(),
                        session,
                        outcome,
                        usage,
                        calls,
                    }));
                });
                OpResult::DesignAgentStarted {
                    window_id,
                    pack: pack_file,
                }
            }
            Err(error) => failed(error.to_string()),
        }
    }

    /// Decision 11: run `run_id`'s input pack, from exactly the inputs the engine froze
    /// when the brainstormers were queued (ruling T8-2). What cannot be read in time is
    /// left out (the goal and the answers are always in). Ruling T8-6: the round's first
    /// start writes it to `design/brainstorm/pack-r<k>.md` and returns its length and
    /// SHA-256 for the engine to record; every later start sends that file, checked
    /// against them, and gets `Err` when it is missing or changed.
    pub(super) async fn brainstorm_pack(
        &self,
        run_id: &str,
    ) -> Result<(String, Option<PackFile>), String> {
        let (mut run, frozen, path) = {
            let state = crate::lock(&self.state);
            let Some(run) = state.runs.get(run_id) else {
                return Err(format!("unknown run {run_id}"));
            };
            let frozen = run.orch.design.as_ref().and_then(|d| d.pack.clone());
            let frozen = frozen.unwrap_or_default();
            let path = pack_path(run, frozen.round.max(1));
            (run.clone(), frozen, path)
        };
        // Ruling T8-6: a later start of the round sends the file its first start wrote.
        if let Some(file) = frozen.file {
            let text = self.pack_file(path, None, Some(file), write_new).await?;
            return Ok((text, None));
        }
        let mut inputs = PackInputs {
            goal: run.goal.clone(),
            answers: run.orch.design.as_ref().and_then(|d| d.answers.clone()),
            ..PackInputs::default()
        };
        if let Some(earlier) = frozen.earlier {
            inputs.earlier = earlier_read(run_id, earlier).await;
        }
        // Task M9.6.9: a rethink's note, and the report it replaces, read as stored.
        if let Some(FrozenRethink {
            note,
            path,
            version,
        }) = frozen.rethink
        {
            let n = version.n;
            let report = stored_text(path, version).await;
            if report.is_none() {
                tracing::warn!(run = %run_id, "the previous brainstorm report is left out");
            }
            inputs.rethink = Some(RethinkInput {
                version: n,
                note,
                report,
            });
        }
        run.scout_reports = frozen.reports;
        let read = tokio::task::spawn_blocking(move || context_reads(&run));
        match tokio::time::timeout(CONTEXT_READ_TIMEOUT, read).await {
            Ok(Ok((profile, reports))) => {
                inputs.profile = profile.as_ref().map(crate::profile::summary);
                inputs.reports = reports;
            }
            _ => tracing::warn!(run = %run_id, "the pack's profile and reports are left out"),
        }
        // The first start writes it; a start that raced it, or followed a launch that
        // failed after writing it, finds the file there and sends that instead.
        let text = (self.pack_file(path, Some(pack(&inputs)), None, write_new)).await?;
        let file = PackFile {
            bytes: text.len() as u64,
            sha256: sha256_hex(text.as_bytes()),
        };
        Ok((text, Some(file)))
    }
}

impl RunService {
    /// Ruling T8-6, off the engine on `spawn_blocking` within `IO_WAIT`: `write`, when
    /// given, written at `path` by `writer` ([`write_new`]; a test's seam) unless a file
    /// is there (`write_new` never replaces one), then the file read back whole, checked
    /// against `expected` when given.
    ///
    /// The final fix wave's FW-33: each round's pack file has one slot, held from the
    /// write to the read-back (inside the blocking task, so even past the wait), which
    /// records what the round's first start read back. A start that raced it waits for
    /// the slot and reads the file against that record without writing, so it never
    /// sends a pack half written in place (no hard links). FW-41: a write that failed,
    /// when the read then fails too, is the reason.
    pub(super) async fn pack_file<W>(
        &self,
        path: PathBuf,
        write: Option<String>,
        expected: Option<PackFile>,
        writer: W,
    ) -> Result<String, String>
    where
        W: FnOnce(&Path, &str) -> Result<(), String> + Send + 'static,
    {
        let slot = self.doc_writes.pack_slot(&path);
        let Ok(mut slot) = tokio::time::timeout(IO_WAIT, slot.lock_owned()).await else {
            let secs = IO_WAIT.as_secs();
            return Err(format!(
                "waiting for the round's first start took over {secs} s"
            ));
        };
        let io = tokio::task::spawn_blocking(move || {
            let expected = expected.or_else(|| slot.clone());
            let wrote = match (write, &expected) {
                (Some(text), None) => writer(&path, &text).err(),
                _ => None,
            };
            let read = read_pack(&path, expected.as_ref());
            match (&read, wrote) {
                (Ok(text), wrote) => {
                    if let Some(error) = wrote {
                        tracing::debug!(%error, "the pack file is already written");
                    }
                    slot.get_or_insert_with(|| PackFile {
                        bytes: text.len() as u64,
                        sha256: sha256_hex(text.as_bytes()),
                    });
                }
                (Err(_), Some(error)) => {
                    tracing::warn!(%error, "the pack file could not be written");
                    return Err(format!("writing it failed: {error}"));
                }
                (Err(_), None) => {}
            }
            read
        });
        match tokio::time::timeout(IO_WAIT, io).await {
            Ok(Ok(read)) => read,
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err(format!("reading it took over {} s", IO_WAIT.as_secs())),
        }
    }
}

/// The pack file at `path`, at most [`PACK_MAX`] bytes, and, when `expected` is given,
/// exactly its length and SHA-256. Blocking.
fn read_pack(path: &Path, expected: Option<&PackFile>) -> Result<String, String> {
    let shown = |e: std::io::Error| format!("{}: {e}", path.display());
    let mismatch = || {
        let path = path.display();
        format!("{path} does not match the pack this round's first start wrote")
    };
    // The driver's one checked read (FW-42) when the round's first start recorded it.
    let bytes = match expected {
        Some(f) => read_checked(path, (f.bytes, &f.sha256)).map_err(|error| match error {
            ReadError::Io(error) => error,
            ReadError::Mismatch(_) => mismatch(),
        })?,
        None => {
            let mut bytes = Vec::new();
            let file = std::fs::File::open(path).map_err(shown)?;
            (file.take(PACK_MAX as u64 + 1))
                .read_to_end(&mut bytes)
                .map_err(shown)?;
            bytes
        }
    };
    if bytes.len() > PACK_MAX {
        return Err(mismatch());
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8", path.display()))
}

/// Ruling T8-4: the folder Codex saves its sessions in, `$CODEX_HOME/sessions`, else
/// `~/.codex/sessions`, from the daemon's environment (`var`).
pub(super) fn codex_sessions_dir(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |key: &str| var(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    match set("CODEX_HOME") {
        Some(home) => Some(home.join("sessions")),
        None => set("HOME").map(|home| home.join(".codex").join("sessions")),
    }
}

/// Ruling T8-4: a Claude brainstormer is also denied the Codex sessions folder; a Codex
/// one gets nothing more, since it cannot be denied reads.
pub(super) fn deny_codex_sessions(spec: &mut DesignAgentSpec, sessions: Option<PathBuf>) {
    let brainstormer = matches!(spec.kind, DesignAgentKind::Brainstormer { .. });
    if !brainstormer || spec.headless.runtime != Runtime::Claude {
        return;
    }
    if let (Some(sandbox), Some(dir)) = (spec.headless.claude_sandbox.as_mut(), sessions) {
        sandbox.deny_read.push(dir);
    }
}

/// Ruling T8-3: each denied folder by its path as given and by its canonical path, so
/// a symlinked data dir (macOS's `/tmp`) is denied however a session names it. Resolved
/// on `spawn_blocking` within `IO_WAIT`; past it, the paths as given, with a warning.
pub(super) async fn with_canonical(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    resolved_by(paths, canonical, IO_WAIT).await
}

/// [`with_canonical`] with its resolver and its wait.
pub(super) async fn resolved_by(
    paths: Vec<PathBuf>,
    resolve: fn(&Path) -> Option<PathBuf>,
    wait: Duration,
) -> Vec<PathBuf> {
    let given = paths.clone();
    let resolve = tokio::task::spawn_blocking(move || {
        let mut out: Vec<PathBuf> = Vec::new();
        for path in paths {
            let real = resolve(&path);
            out.push(path);
            if let Some(real) = real.filter(|r| !out.contains(r)) {
                out.push(real);
            }
        }
        out
    });
    let why = match tokio::time::timeout(wait, resolve).await {
        Ok(Ok(out)) => return out,
        Ok(Err(error)) => error.to_string(),
        Err(_) => format!("not resolved within {} ms", wait.as_millis()),
    };
    // Fix round 2: the fallback is visible.
    let shown: Vec<String> = given.iter().map(|p| p.display().to_string()).collect();
    tracing::warn!(paths = ?shown, %why, "a design agent's read denials are denied as given");
    given
}

/// `path` canonical: a folder not made yet (a run's design folder before its first
/// document) through its nearest existing ancestor. Blocking.
fn canonical(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    for ancestor in path.ancestors() {
        if let Ok(real) = std::fs::canonicalize(ancestor) {
            return Some(rest.iter().rev().fold(real, |at, part| at.join(part)));
        }
        rest.push(ancestor.file_name()?);
    }
    None
}

/// How a design agent's session ended, what it spent and how many tools it called.
async fn ended(scouts: &ScoutService, handle: ScoutHandle) -> (ScoutEnd, TokenUsage, u32) {
    let info = |id: &str| scouts.info(id).map(|i| (i.usage, i.tool_calls));
    let outcome = design_end(handle.outcome.await.ok());
    let (usage, calls) = info(&handle.id).unwrap_or_default();
    (outcome, usage, calls)
}

/// A design agent's session's end as the engine takes it (`None`: no outcome came).
/// Ruling T8-7 and the final fix wave's FW-36: the machine's typed end of a turn that
/// ended unnudged without the submission is `ScoutEnd::Unsubmitted`, so the engine
/// relaunches by the cause's type; no text is compared here either.
pub(super) fn design_end(outcome: Option<ScoutOutcome>) -> ScoutEnd {
    match outcome {
        Some(ScoutOutcome::Accepted | ScoutOutcome::Report(_)) => ScoutEnd::Reported,
        Some(ScoutOutcome::Unsubmitted { reason }) => ScoutEnd::Unsubmitted { reason },
        Some(ScoutOutcome::Failed { reason }) => ScoutEnd::Failed { reason },
        None => ScoutEnd::Failed {
            reason: "the session ended without an outcome".to_string(),
        },
    }
}
