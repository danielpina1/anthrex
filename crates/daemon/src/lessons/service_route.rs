//! The distill worker's two side reads and writes beside the store: the repository's
//! project (for the call's route) and the call's `role_route` history record.

use std::path::{Path, PathBuf};

use proto::{HistoryLine, QueuedLine, RoleRoutingInput};

use crate::decider::Decision;
use crate::decider::call::Routed;

/// The repository's project, from its stored profile's meta, for the call's route
/// (`models.toml`); `None` routes on the global table. On `spawn_blocking`.
pub(super) async fn project_of(repo: &Path) -> Option<PathBuf> {
    let path = repo.join(crate::profile::store::META_FILE);
    let read = tokio::task::spawn_blocking(move || {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<proto::ProfileMeta>(&text)
            .ok()?
            .project
    });
    read.await.ok().flatten()
}

/// The call's `role_route` record with no run, `distill/<queued>/<n>`, appended once
/// to the repository's `history.jsonl` on `spawn_blocking`, as `record_run_name`'s.
pub(super) async fn record_route(
    repo: &Path,
    queued: &QueuedLine,
    n: u32,
    (routed, decision): (&Routed, &Decision),
    at: u64,
) {
    use crate::run::orch::roles;
    let session = format!("{}/{n}", queued.record_id);
    let label = crate::decider::DeciderKind::Distill.label();
    let tasks: Vec<String> = queued.task_id.iter().cloned().collect();
    let input = (RoleRoutingInput::default(), at);
    let mut record = roles::decider_routed(None, (session.as_str(), label), &tasks, routed, input);
    let (outcome, result) = roles::decider_outcome(decision);
    roles::finish(&mut record, outcome, result);
    let path = repo.join(crate::run::engine::HISTORY_FILE);
    let line = HistoryLine::RoleRoute(record);
    let written =
        tokio::task::spawn_blocking(move || crate::run::history_io::append_once(&path, &line))
            .await;
    if !matches!(written, Ok(Ok(_))) {
        tracing::warn!("lessons: the distill call's history record was not written");
    }
}
