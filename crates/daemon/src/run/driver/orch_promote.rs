//! Moved out of `driver/orch.rs` (task M9.3.6a, move-only): decision 29's `run
//! promote`, decision 17's `get_context` and its reads, and decision 34's scout extract.

use proto::run_wire::request;
use proto::{OrchestratorChoice, RepoProfile, RunReply, ScoutReport};

use super::super::RunService;
use super::{CONTEXT_READ_TIMEOUT, answer, refused};
use crate::run::engine::{EventKind, OrchEvent};
use crate::run::model::Run;
use crate::run::orch::context::{Asker, ContextInputs, context};
use crate::run::orch::extract::ExtractSlot;

impl RunService {
    /// Decision 29: `run promote`, performed by the engine.
    pub(in crate::run::driver) async fn promote(
        &self,
        run_id: String,
        orchestrator: Option<OrchestratorChoice>,
    ) -> RunReply {
        match self.promote_refusal(&run_id, orchestrator.as_ref()).await {
            Err(text) => return RunReply::refused(request::PROMOTE, text),
            // M9.17 fix round 2: what the check found, recorded before the engine
            // performs the promotion (events are handled in order).
            Ok(Some(found)) => self.send(EventKind::Orch(OrchEvent::Installed {
                run_id: run_id.clone(),
                installed: found.headless,
                window: found.window,
            })),
            Ok(None) => {}
        }
        let event = |reply| EventKind::Promote {
            reply,
            run_id,
            orchestrator,
        };
        answer(request::PROMOTE, self.ask(event).await)
    }
}

/// Decision 17: the context of a clone of the run, its stored profile and scout
/// reports read and the answer built on `spawn_blocking`, within
/// [`CONTEXT_READ_TIMEOUT`].
pub(super) async fn get_context(run: Run, asker: Asker, only: Option<Vec<String>>) -> RunReply {
    let build = tokio::task::spawn_blocking(move || {
        let (profile, reports) = context_reads(&run);
        let inputs = ContextInputs {
            run: &run,
            asker,
            profile: profile.as_ref(),
            reports,
            only,
        };
        context(&inputs).to_string()
    });
    match tokio::time::timeout(CONTEXT_READ_TIMEOUT, build).await {
        Ok(Ok(text)) => RunReply::tool_result(true, text),
        Ok(Err(error)) => refused(format!("a blocking step did not finish: {error}")),
        Err(_) => refused(format!(
            "the run's context could not be read within {} s",
            CONTEXT_READ_TIMEOUT.as_secs()
        )),
    }
}

/// `get_context`'s reads (blocking): the repository's stored profile, and the reports
/// it lists, the onboarding one (the alias `onboarding`, when the profile names one)
/// and each run scout's. Each ref is resolved only to a report anthrex stored
/// (`scout::report::resolve_ref`, through `read_report`'s guards); one that cannot be
/// read is left out with a warning.
pub(in crate::run::driver) fn context_reads(run: &Run) -> (Option<RepoProfile>, Vec<ScoutReport>) {
    use crate::profile::store::{Stored, load};
    use crate::scout::report::{ONBOARDING_ALIAS, resolve_ref};
    use crate::scout::spec::valid_id;
    let repo_dir = &run.repo_dir;
    if repo_dir.as_os_str().is_empty() {
        return (None, Vec::new());
    }
    let profile = match load(repo_dir) {
        Stored::Found { profile, .. } => Some(profile),
        _ => None,
    };
    // The run's own directory, `<data_dir>/runs/<id>` (task M9.13's fix).
    let run_dir = &run.data_dir;
    let onboarding = run.onboarding_report.as_deref().filter(|id| valid_id(id));
    let mut refs: Vec<&str> = onboarding.map(|_| ONBOARDING_ALIAS).into_iter().collect();
    for id in &run.scout_reports {
        if valid_id(id) && id != ONBOARDING_ALIAS && !refs.contains(&id.as_str()) {
            refs.push(id);
        }
    }
    let reports = refs
        .into_iter()
        .filter_map(|reference| {
            let path = resolve_ref(reference, run_dir, repo_dir, onboarding);
            super::super::adapt::read_report(&path)
                .inspect_err(
                    |error| tracing::warn!(run = %run.id, "scout report {reference}: {error}"),
                )
                .ok()
        })
        .collect();
    (profile, reports)
}

impl RunService {
    /// Decision 34, the driver's half: a first turn with the scout extract of its
    /// slot's reports, read on a blocking thread (each resolved only to a report anthrex
    /// stored, through `read_report`'s guards); a report that cannot be read is left out
    /// with a warning (`readable_reports`). With no slot, the turn as the engine built
    /// it. A run scout's report is in the run's own directory (`OpCtx::data_dir`,
    /// `<data_dir>/runs/<id>`), the onboarding report in the repository's (`Run::
    /// repo_dir`, read under the engine lock): task M9.13 found both paths built from the
    /// run's directory as if it were the daemon's, so no extract was ever filled.
    pub(in crate::run::driver) async fn fill_extract(
        &self,
        ctx: &super::super::OpCtx,
        slot: Option<ExtractSlot>,
        first_turn: String,
    ) -> String {
        let Some(slot) = slot else {
            return first_turn;
        };
        let run_dir = ctx.data_dir.clone();
        let repo_dir = crate::lock(&self.state)
            .runs
            .get(&ctx.run_id)
            .map(|run| run.repo_dir.clone())
            .unwrap_or_default();
        let fallback = first_turn.clone();
        tokio::task::spawn_blocking(move || filled(&slot, &run_dir, &repo_dir, &first_turn))
            .await
            .unwrap_or(fallback)
    }
}

/// [`fill_extract`]'s blocking body.
pub(super) fn filled(
    slot: &ExtractSlot,
    run_dir: &std::path::Path,
    repo_dir: &std::path::Path,
    first_turn: &str,
) -> String {
    use crate::run::orch::extract::{readable_reports, scout_extract};
    use crate::scout::report::resolve_ref;
    use crate::scout::spec::valid_id;
    let read = slot
        .refs
        .iter()
        .map(|reference| {
            let report = if valid_id(reference) {
                let onboarding = slot.onboarding.as_deref().filter(|id| valid_id(id));
                let path = resolve_ref(reference, run_dir, repo_dir, onboarding);
                super::super::adapt::read_report(&path)
            } else {
                Err("not a stored report".to_string())
            };
            (reference.clone(), report)
        })
        .collect();
    slot.fill(first_turn, &scout_extract(&readable_reports(read)))
}
