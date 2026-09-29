//! Milestone 8b's driver side (M8b decision 1): the executions and requests the
//! adaptation adds. `driver/requests.rs` and `driver/ops.rs` only dispatch here.
//!
//! M8b.4: [`RunService::choose_profile`], decision 6 at `run start`. The stored profile
//! is read, and its staleness checked, on `spawn_blocking`, never under a lock.
//!
//! M8b.9: [`Adaptation`], the services the daemon hands the driver once, and the routing
//! of a scout's tool call to [`ScoutService::tool`] (decision 15).
//!
//! M8b.11: `RunRequest::Profile` goes to [`ProfileService::request`].
//!
//! M8b.12: `OpKind::Decide`, executed by [`RunService::decide`] with the adaptation's
//! [`DeciderContext`], on the op's own task and never under a lock (decision 18).
//!
//! M8b.13: a size check's `evidence_refs` are read into `Evidence` first
//! ([`read_evidence`], on `spawn_blocking`), only from the reports anthrex stored.
//!
//! M8b.14: `RunRequest::StartGoal`, in `adapt_goal.rs`.
//!
//! M8b.15: the OTLP receiver's way into the engine is `driver/usage.rs`.
//!
//! M8b.17: `RunRequest::Stats`, and revert detection at every `run start`, in
//! `adapt_history.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{
    AgentRole, OutputFilter, Plan, ProfileReply, ProfileRequest, ProfileSource, RunReply, ToolCall,
};

use super::{OpCtx, RunService};
use crate::decider::call::decide;
use crate::decider::fallback::fallback_decision;
use crate::decider::{DeciderContext, DeciderRequest, Evidence};
use crate::profile::resolve::{apply_choice as apply_to_plan, run_profile};
use crate::profile::service::ProfileService;
use crate::profile::store::{self, PROFILE_FILE, Stored};
use crate::run::engine::{EventKind, OpKind, OpResult, OrchEvent};
use crate::run::model::{LogEntry, OpId, Run};
use crate::run::orch::roles;
use crate::run::plan::Preflight;
use crate::scout::service::ScoutService;

// M8b.14: `run start --goal` (decision 22).
#[path = "adapt_goal.rs"]
mod goal;
pub(super) use goal::{BuildError, fast_barrier};

// M8b.16: `MeasureDiff` and `AppendHistory` (decisions 32, 33); M8b.17: `run stats`
// and revert detection (decisions 34, 35).
#[path = "adapt_history.rs"]
mod history;

/// What decision 6 chose, for the run `build_run` makes.
pub(super) struct ProfileChoice {
    source: ProfileSource,
    output_filter: OutputFilter,
    filter_prefixes: Vec<String>,
    repo_dir: PathBuf,
    stale: Vec<String>,
    notes: Vec<String>,
    /// M8b decision 19: the stored profile's onboarding report (the alias `onboarding`).
    onboarding_report: Option<String>,
    /// M8b decision 33a: the stored profile's languages, for routing decisions.
    languages: Vec<String>,
}

/// Decision 6's refusal for a stored profile that does not parse.
fn unparseable(path: &std::path::Path, error: &str) -> String {
    format!(
        "the stored profile at {} does not parse: {error}; fix it with anthrex profile edit or re-detect it with anthrex profile detect",
        path.display()
    )
}

/// Milestone 8b's services, set once with [`RunService::set_adaptation`].
pub struct Adaptation {
    pub profiles: Arc<ProfileService>,
    pub scouts: Arc<ScoutService>,
    /// Every decider call inside a run (decision 16).
    pub deciders: DeciderContext,
}

/// The fallback reason when the daemon has no decider context (a test's service).
const NO_DECIDERS: &str = "the decider could not start: the daemon has no decider context";

impl RunService {
    /// Op `op` of `ctx`'s run: a `Decide` is [`RunService::decide_as`] with its record
    /// (milestone 9 decision 43), any other op `ops::run`.
    pub(super) async fn run_op(self: &Arc<Self>, ctx: &OpCtx, op: OpId, kind: OpKind) -> OpResult {
        match kind {
            OpKind::Decide {
                task_ids, request, ..
            } => {
                let task = task_ids.into_iter().next();
                self.decide_as(ctx, Some((op, task)), request).await
            }
            kind => super::ops::run(self, ctx, kind).await,
        }
    }

    /// `OpKind::Decide` (decision 18), with no role-routing record.
    pub(super) async fn decide(&self, ctx: &OpCtx, request: DeciderRequest) -> OpResult {
        self.decide_as(ctx, None, request).await
    }

    /// `OpKind::Decide` (decision 18): the call, always answered, a fallback included.
    /// A size check's evidence is read first (decision 19); with none readable, the
    /// check is its fallback. Milestone 9 decision 43: a call that starts a decider
    /// session is recorded as op `op`'s (for task `task`), its record sent to the
    /// engine before the call and its outcome after.
    async fn decide_as(
        &self,
        ctx: &OpCtx,
        record: Option<(OpId, Option<String>)>,
        mut request: DeciderRequest,
    ) -> OpResult {
        if let Err(error) = self.with_evidence(ctx, &mut request).await {
            let reason = format!("the decider could not start: {error}");
            return OpResult::Decided(Box::new(fallback_decision(&request, reason)));
        }
        let decision = match self.adaptation.get() {
            Some(adaptation) => {
                let route = &adaptation.deciders.route;
                let id = record.and_then(|(op, task)| {
                    self.decider_dispatched(ctx, (op, task.as_deref()), route, &request)
                });
                let decision = decide(&adaptation.deciders, &request).await;
                if let Some(record_id) = id {
                    let (outcome, result) = roles::decider_outcome(&decision);
                    self.send(EventKind::Orch(OrchEvent::RoleRouteEnded {
                        run_id: ctx.run_id.clone(),
                        record_id,
                        outcome,
                        result,
                    }));
                }
                decision
            }
            None => fallback_decision(&request, NO_DECIDERS.to_string()),
        };
        OpResult::Decided(Box::new(decision))
    }

    /// Decision 43: a run-bound decider's record, built under the engine's lock (pure)
    /// and sent to the engine; its record id.
    fn decider_dispatched(
        &self,
        ctx: &OpCtx,
        (op, task): (OpId, Option<&str>),
        route: &proto::Route,
        request: &DeciderRequest,
    ) -> Option<String> {
        let decision = crate::lock(&self.state).runs.get(&ctx.run_id).map(|run| {
            let session = (op.to_string(), request.kind().label());
            let input = roles::input_of(run);
            let at = super::unix_now();
            roles::decider_record(Some(run), (&session.0, session.1), task, route, input, at)
        })?;
        let record_id = decision.record_id.clone();
        self.send(EventKind::Orch(OrchEvent::RoleRoute {
            run_id: ctx.run_id.clone(),
            decision: Box::new(decision),
        }));
        Some(record_id)
    }

    /// Decision 19: fills a size check's `evidence` from its `evidence_refs`, resolved
    /// against the run's own directories (read under the lock, the files after it, on
    /// `spawn_blocking`). Any other request is left as it is.
    pub(super) async fn with_evidence(
        &self,
        ctx: &OpCtx,
        request: &mut DeciderRequest,
    ) -> Result<(), String> {
        let DeciderRequest::SizeCheck(input) = request else {
            return Ok(());
        };
        let (repo_dir, onboarding) = crate::lock(&self.state)
            .runs
            .get(&ctx.run_id)
            .map(|run| (run.repo_dir.clone(), run.onboarding_report.clone()))
            .ok_or_else(|| format!("unknown run {}", ctx.run_id))?;
        let (refs, run_dir) = (input.evidence_refs.clone(), ctx.data_dir.clone());
        let evidence = tokio::task::spawn_blocking(move || {
            read_evidence(&refs, &run_dir, &repo_dir, onboarding.as_deref())
        })
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))??;
        input.evidence = evidence;
        Ok(())
    }

    /// Sets the services once; a second call is ignored.
    pub fn set_adaptation(&self, adaptation: Adaptation) {
        let _ = self.adaptation.set(adaptation);
    }

    /// The per-repository git write queue, shared with the profile service.
    pub fn git_queue(&self) -> Arc<crate::run::git::GitQueue> {
        self.queue.clone()
    }

    /// `anthrex profile …` (decision 10), answered by the profile service.
    pub(super) async fn profile(&self, request: ProfileRequest) -> RunReply {
        let reply = match self.adaptation.get() {
            Some(adaptation) => adaptation.profiles.request(request).await,
            None => ProfileReply::Refused {
                message: "the profile service is not running".to_string(),
            },
        };
        RunReply::profile(reply)
    }

    /// A `submit_*` call from an agent (decision 15): a scout's goes to the scout
    /// service, every other to the engine, and so does a research task's (milestone 9
    /// decision 35: a scout window bound to a task, with no scout id). Every text
    /// reaches the agent verbatim (M8a.19), so each is worded for an agent.
    pub(super) async fn tool(&self, call: ToolCall) -> RunReply {
        // Milestone 9 decision 15: its branches go ahead of the scout's.
        if super::orch::is_orch_call(&call) {
            return self.orch_tool(call).await;
        }
        let research = call.task_id.is_some() && call.scout_id.is_none();
        if call.role == AgentRole::Scout && !research {
            return match self.adaptation.get() {
                Some(adaptation) => adaptation.scouts.tool(call).await,
                None => RunReply::tool_result(
                    false,
                    format!("unknown scout {}", call.scout_id.unwrap_or_default()),
                ),
            };
        }
        match self.ask(|reply| EventKind::Tool { reply, call }).await {
            Ok(text) => RunReply::tool_result(true, text),
            Err(text) => RunReply::tool_result(false, text),
        }
    }

    /// Decision 6, right after preflight: loads the repository's stored profile and,
    /// when there is one, makes it the plan's whole profile and empties the cloned
    /// config's `profile`, so `resolve_profile` fills no deliberate gap. The config's
    /// confinement tables are left alone: `build_run` takes them for the root. A stored
    /// profile that does not parse refuses the run.
    pub(super) async fn choose_profile(
        &self,
        plan: &mut Plan,
        config: &mut config::Orchestrator,
        pre: &Preflight,
    ) -> Result<ProfileChoice, String> {
        let repo_dir = crate::profile::repo_dir(&self.ctx.data_dir, &pre.project);
        let (dir, project) = (repo_dir.clone(), pre.project.clone());
        let (stored, stale) = tokio::task::spawn_blocking(move || {
            let stored = store::load(&dir);
            let stale = match &stored {
                Stored::Found { meta, .. } => store::stale(&project, meta),
                _ => Vec::new(),
            };
            (stored, stale)
        })
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?;
        let mut onboarding_report = None;
        let (stored, path) = match stored {
            Stored::Found {
                profile,
                path,
                meta,
            } => {
                onboarding_report = meta.report;
                (Some(profile), path)
            }
            Stored::Unparseable { path, error } => return Err(unparseable(&path, &error)),
            Stored::Absent => (None, repo_dir.join(PROFILE_FILE)),
        };
        // Decision 7 at `run start` (task 11 review, I4): a stale stored profile is still
        // used, and a re-detection starts beside the run where `onboarding.auto` allows
        // it. The preflight just passed, so the tree is clean.
        if !stale.is_empty()
            && let Some(adaptation) = self.adaptation.get()
        {
            let (profiles, pre, stale) = (adaptation.profiles.clone(), pre.clone(), stale.clone());
            tokio::spawn(async move { profiles.auto_on_stale(&pre, stale).await });
        }
        // Decision 34: every `run start`, of either kind, looks for reverts of accepted
        // runs, in the background.
        self.detect_reverts_later(pre.root.clone(), &repo_dir);
        let languages = stored.as_ref().map(|p| p.languages.clone());
        let chosen = run_profile(stored.as_ref(), &path, &plan.profile, &config.profile);
        apply_to_plan(&chosen, &mut plan.profile, &mut config.profile);
        Ok(ProfileChoice {
            source: chosen.source,
            output_filter: chosen.output_filter,
            filter_prefixes: chosen.filter_prefixes,
            repo_dir,
            stale,
            notes: chosen.notes,
            onboarding_report,
            languages: languages.unwrap_or_default(),
        })
    }
}

/// Copies the choice onto the built run: its source, filter settings, repository data
/// directory, stale files (the attention line) and one log line per ignored plan key.
pub(super) fn apply_choice(run: &mut Run, choice: ProfileChoice, now: u64) {
    run.profile_source = Some(choice.source);
    run.output_filter = choice.output_filter;
    run.filter_prefixes = choice.filter_prefixes;
    run.repo_dir = choice.repo_dir;
    run.profile_languages = choice.languages;
    run.stale_profile = choice.stale;
    run.onboarding_report = choice.onboarding_report;
    run.log.extend(
        choice
            .notes
            .into_iter()
            .map(|text| LogEntry { at: now, text }),
    );
}

/// The most of one report file [`read_evidence`] reads.
const EVIDENCE_FILE_MAX: u64 = 1 << 20;

/// Decision 19: the reports `refs` name, as a size check's evidence. Each ref is
/// resolved only to a report anthrex stored (`scout::report::resolve_ref`): the alias
/// `onboarding` to the stored profile's under `<repo_dir>/scouts/`, any other to a run
/// scout's under `<run_dir>/scouts/`. A ref (or report id) that is not a scout id is
/// never made into a path, and a file that is not a regular file (a link included),
/// is larger than 1 MiB or is not a report is skipped. An error when none is read.
/// Blocking.
pub(super) fn read_evidence(
    refs: &[String],
    run_dir: &Path,
    repo_dir: &Path,
    onboarding: Option<&str>,
) -> Result<Vec<Evidence>, String> {
    use crate::scout::report::{ONBOARDING_ALIAS, resolve_ref};
    use crate::scout::spec::valid_id;
    let onboarding = onboarding.filter(|id| valid_id(id) && !repo_dir.as_os_str().is_empty());
    let mut evidence = Vec::new();
    let mut problems = Vec::new();
    for reference in refs {
        if !valid_id(reference) || (reference == ONBOARDING_ALIAS && onboarding.is_none()) {
            problems.push(format!("{reference}: not a stored report"));
            continue;
        }
        let path = resolve_ref(reference, run_dir, repo_dir, onboarding);
        match read_report(&path) {
            Ok(report) => evidence.push(Evidence {
                id: report.id,
                summary: report.summary,
                files: report.files.into_iter().map(|f| f.path).collect(),
                modules: report.modules,
                interfaces: report.interfaces,
            }),
            Err(error) => problems.push(format!("{reference}: {error}")),
        }
    }
    if evidence.is_empty() {
        return Err(format!(
            "no scout report could be read ({})",
            problems.join("; ")
        ));
    }
    Ok(evidence)
}

/// One stored report: a regular file, not through a link, at most 1 MiB. Opened once
/// without following a link (and without blocking on a FIFO), checked through the
/// opened handle, and read through a cap one byte past the limit, so a file swapped
/// or grown after the check is neither followed nor read whole (review m3).
pub(super) fn read_report(path: &Path) -> Result<proto::ScoutReport, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP) => "not a regular file (a symbolic link)".to_string(),
            _ => e.to_string(),
        })?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() {
        return Err("not a regular file".to_string());
    }
    let mut bytes = Vec::new();
    file.take(EVIDENCE_FILE_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > EVIDENCE_FILE_MAX {
        return Err(format!("larger than {EVIDENCE_FILE_MAX} bytes"));
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("not a report: {e}"))
}

#[cfg(test)]
#[path = "adapt_evidence_tests.rs"]
mod evidence_tests;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use proto::{OutputFilter, ProfileSource};

    use super::{ProfileChoice, apply_choice};
    use crate::run::plan::{BuildContext, Preflight, build_run, parse_plan};

    pub(super) fn run() -> crate::run::model::Run {
        let plan = parse_plan(
            "goal = \"g\"\n[[task]]\nid = \"t1\"\ntitle = \"One\"\nsize = \"S\"\nowns = [\"a.rs\"]\nbrief = \"b\"\nacceptance = [\"a\"]\n",
        )
        .unwrap();
        let config = config::Orchestrator::default();
        let pre = Preflight {
            root: PathBuf::from("/tmp/ax-adapt-root"),
            project: PathBuf::from("/tmp/ax-adapt-root"),
            git_common_dir: PathBuf::from("/tmp/ax-adapt-root/.git"),
            base_branch: "main".into(),
            base_sha: "b".repeat(40),
            protected_files: Vec::new(),
        };
        let ctx = BuildContext {
            id: "g-0001".into(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: PathBuf::from("/tmp/data/runs/g-0001"),
            config: &config,
            now: 1,
            yes: false,
        };
        build_run(plan, pre, ctx).unwrap_or_else(|e| panic!("{e:?}"))
    }

    #[test]
    fn apply_choice_copies_every_field_and_logs_each_note() {
        let mut run = run();
        assert_eq!(run.profile_source, None);
        assert_eq!(run.repo_dir, PathBuf::new());
        let choice = ProfileChoice {
            source: ProfileSource::Stored,
            output_filter: OutputFilter::Tail,
            filter_prefixes: vec!["cargo test".into()],
            repo_dir: PathBuf::from("/data/repos/r-00000000"),
            stale: vec!["Cargo.toml".into()],
            notes: vec!["note one".into(), "note two".into()],
            onboarding_report: Some("onboarding-7".into()),
            languages: vec!["rust".into()],
        };
        apply_choice(&mut run, choice, 42);
        assert_eq!(run.profile_source, Some(ProfileSource::Stored));
        assert_eq!(run.output_filter, OutputFilter::Tail);
        assert_eq!(run.filter_prefixes, vec!["cargo test".to_string()]);
        assert_eq!(run.repo_dir, PathBuf::from("/data/repos/r-00000000"));
        assert_eq!(run.stale_profile, vec!["Cargo.toml".to_string()]);
        assert_eq!(run.onboarding_report.as_deref(), Some("onboarding-7"));
        assert_eq!(run.profile_languages, vec!["rust".to_string()]);
        let logged: Vec<(u64, &str)> = run.log.iter().map(|e| (e.at, e.text.as_str())).collect();
        assert_eq!(logged, vec![(42, "note one"), (42, "note two")]);
    }
}
