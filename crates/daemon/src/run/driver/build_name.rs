//! The run title change: a goal's run is named by the `run_name` decider before its id
//! is drawn (`build.rs`, right before `pick_id`). The decider's slug heads the id
//! (`plan::slug`'s `<head>-<4 hex>`, the redraw loop unchanged) and its title is the
//! run's `title`, which clients show in place of the goal. On any failure, timeout or
//! invalid answer the run keeps today's id from its goal and an empty title: a run never
//! fails because of naming and never waits past [`RUN_NAME_BOUND`].
//!
//! The call is pre-run triage's kind of call: off the manager lock, its probe and spawn
//! on `spawn_blocking` (`decider::call`), recorded as a `role_route` line with no run
//! (`run_name/<unix nanos>/<n>`) in the repository's `history.jsonl`, written on
//! `spawn_blocking` and never awaited by the start, and its usage added to the run's
//! decider usage. Like triage it is not one of the run's engine `Decided` calls, so
//! `decider_calls` does not count it.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use proto::{HistoryLine, RoleRoutingInput, TokenUsage};

use super::adapt::Adaptation;
use super::{RunService, unix_now};
use crate::decider::call::{Routed, decide_within};
use crate::decider::{DeciderAnswer, DeciderRequest, Decision, RunNameInput};
use crate::run::engine::HISTORY_FILE;
use crate::run::history_io::append_once;
use crate::run::orch::roles;

/// How long a start waits for its run's name, in all.
pub const RUN_NAME_BOUND: Duration = Duration::from_secs(15);

/// What naming gave a run: its title and its id's head, each empty when the decider
/// fell back, and the call's usage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunName {
    pub title: String,
    pub slug: String,
    pub usage: Option<TokenUsage>,
}

impl RunName {
    /// The name in `decision`: a decider's validated answer, else nothing.
    pub(crate) fn from_decision(decision: &Decision) -> RunName {
        let (title, slug) = match (&decision.source, &decision.answer) {
            (proto::DeciderSource::Decider, DeciderAnswer::RunName { title, slug }) => {
                (title.clone(), slug.clone())
            }
            _ => (String::new(), String::new()),
        };
        RunName {
            title,
            slug,
            usage: decision.usage,
        }
    }

    /// What `pick_id` slugs: the model's slug, else the goal (today's id).
    pub(crate) fn id_head<'a>(&'a self, goal: &'a str) -> &'a str {
        if self.slug.is_empty() {
            goal
        } else {
            &self.slug
        }
    }
}

/// Run-name records made by this daemon: each record id's `<n>`.
static RUN_NAME_SEQ: AtomicU64 = AtomicU64::new(1);

impl RunService {
    /// Names `goal`'s run (see the module). `project` is the repository whose
    /// `history.jsonl` gets the record; `languages` the profile's, for its input.
    pub(super) async fn name_run(
        &self,
        goal: &str,
        project: &Path,
        languages: &[String],
    ) -> RunName {
        let Some(adaptation) = self.adaptation.get() else {
            return RunName::default();
        };
        let request = DeciderRequest::RunName(RunNameInput {
            goal: goal.to_string(),
        });
        let (routed, decision) =
            decide_within(&adaptation.deciders, &request, RUN_NAME_BOUND).await;
        if let Some(line) = routed.as_ref().and_then(Routed::moved_line) {
            tracing::info!("{line}");
        }
        if let Some(reason) = &decision.fallback_reason {
            tracing::info!(%reason, "the run_name decider fell back; the run is named by its goal");
        }
        if let Some(routed) = &routed {
            self.record_run_name(adaptation, project, (goal, languages), (routed, &decision));
        }
        RunName::from_decision(&decision)
    }

    /// The `run_name` decider's record, with no run, appended to the repository's
    /// `history.jsonl` on `spawn_blocking`, as pre-run triage's is
    /// (`adapt_goal.rs::record_triage`), but not awaited: the start does not wait on the
    /// file system. A failure is only logged.
    fn record_run_name(
        &self,
        adaptation: &Adaptation,
        project: &Path,
        (goal, languages): (&str, &[String]),
        (routed, decision): (&Routed, &Decision),
    ) {
        let n = RUN_NAME_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let input = RoleRoutingInput {
            goal: Some(roles::capped_goal(goal)),
            languages: languages.to_vec(),
            ..RoleRoutingInput::default()
        };
        let session = format!("{nanos}/{n}");
        let route = &routed.ctx.route;
        let roster = &adaptation.scouts.context().roster.current();
        let chosen = (
            route,
            roles::decider_candidates(roster, route, routed.strength()),
        );
        let label = crate::decider::DeciderKind::RunName.label();
        let pick = (routed.pick.as_ref(), routed.moved.as_ref());
        let at = (input, unix_now());
        let mut record =
            roles::decider_listed(None, (session.as_str(), label), &[], chosen, pick, at);
        let (outcome, result) = roles::decider_outcome(decision);
        roles::finish(&mut record, outcome, result);
        let repo_dir = crate::profile::repo_dir(&self.ctx.data_dir, project);
        let path = repo_dir.join(HISTORY_FILE);
        let line = HistoryLine::RoleRoute(record);
        drop(tokio::task::spawn_blocking(move || {
            if let Err(error) = append_once(&path, &line) {
                tracing::warn!(%error, "the run_name decider's history record was not written");
            }
        }));
    }
}

#[cfg(test)]
#[path = "build_name_tests.rs"]
mod tests;
