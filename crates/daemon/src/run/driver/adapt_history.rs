//! M8b decisions 32 and 33, the driver side: `MeasureDiff` and `AppendHistory`, each on
//! `spawn_blocking`, never under a lock. A run record of an accepted run gets its
//! `accepted_commit` just before it is appended: the one field the reducer cannot know.

use super::super::{OpCtx, RunService};
use crate::run::engine::{OpKind, OpResult};
use crate::run::history_io::{append_line, fill_accepted_commit, measure_diff};

fn failed(message: String) -> OpResult {
    OpResult::Failed { message }
}

impl RunService {
    /// `OpKind::MeasureDiff` or `OpKind::AppendHistory`, as their executor contracts say.
    pub(in crate::run::driver) async fn history_op(&self, ctx: &OpCtx, kind: OpKind) -> OpResult {
        let (git, timeout) = (self.ctx.git.clone(), ctx.git_timeout);
        let result = match kind {
            OpKind::MeasureDiff {
                root,
                from,
                to,
                three_dot,
            } => {
                tokio::task::spawn_blocking(move || {
                    measure_diff(&git, &root, &from, &to, three_dot, timeout)
                        .map(OpResult::DiffMeasured)
                })
                .await
            }
            OpKind::AppendHistory { path, line, .. } => {
                let root = crate::lock(&self.state)
                    .runs
                    .get(&ctx.run_id)
                    .map(|run| run.root.clone());
                tokio::task::spawn_blocking(move || {
                    let line = match root {
                        Some(root) => fill_accepted_commit(&git, &root, *line, timeout),
                        None => *line,
                    };
                    append_line(&path, &line)
                        .map(|()| OpResult::HistoryAppended)
                        .map_err(|error| format!("could not append to {}: {error}", path.display()))
                })
                .await
            }
            other => return failed(format!("{} is not a history op", other.name())),
        };
        match result {
            Ok(Ok(result)) => result,
            Ok(Err(message)) => failed(message),
            Err(error) => failed(format!("a blocking step did not finish: {error}")),
        }
    }
}
