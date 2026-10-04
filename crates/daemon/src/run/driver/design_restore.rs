//! Milestone 9.6 (task 5's carry): after a restore, every design run's stored versions
//! are read back off the engine and checked against their index entries, as `run show`
//! checks them (`design_io.rs`); the engine gets the result as
//! `EventKind::DesignChecked` (`engine/design_gate.rs::checked`): a missing or changed
//! file is logged, an open gate whose version is one reopens as revising, and the
//! latest text of each gate document refills its change summaries' cache.
//!
//! The engine lock is taken only to list the versions; every read runs on
//! `spawn_blocking`, bounded by [`IO_WAIT`] per run (AGENTS.md rule 2).

use std::path::PathBuf;

use proto::DocKind;

use super::RunService;
use super::design_io::{IO_WAIT, ReadError, read_stored};
use crate::run::design::state::{self, DocVersion};
use crate::run::engine::{DocChecked, EventKind};

/// One version to read back: its index entry, its file, and whether its text is kept.
pub type ToCheck = (DocVersion, PathBuf, bool);

impl RunService {
    /// Reads back every design run's versions and sends each run's result to the
    /// engine. Called by `restore`, once its effects ran.
    pub(super) async fn check_design_docs(&self) {
        for (run_id, docs) in self.design_docs() {
            let checked =
                tokio::time::timeout(IO_WAIT, tokio::task::spawn_blocking(move || check(docs)))
                    .await;
            match checked {
                Ok(Ok(checked)) => self.send(EventKind::DesignChecked { run_id, checked }),
                Ok(Err(error)) => {
                    tracing::error!(run = %run_id, %error, "checking the design documents panicked")
                }
                Err(_) => tracing::error!(run = %run_id, "checking the design documents timed out"),
            }
        }
    }

    /// Each design run's versions, looked up under the engine lock.
    fn design_docs(&self) -> Vec<(String, Vec<ToCheck>)> {
        let state = crate::lock(&self.state); // lookup only; no file access under it
        let runs = state.runs.values().filter(|r| !r.state.is_terminal());
        runs.filter_map(|run| {
            let design = run.orch.design.as_ref()?;
            let dir = state::design_dir(run);
            let latest = |v: &DocVersion| {
                let gate_doc =
                    matches!(v.kind, DocKind::Brainstorm | DocKind::Spec | DocKind::Plan);
                gate_doc && v.n > 0 && design.find(v.kind, None).is_some_and(|l| l.n == v.n)
            };
            let docs = (design.versions.iter())
                .map(|v| (v.clone(), dir.join(design.file_name(v)), latest(v)))
                .collect();
            Some((run.id.clone(), docs))
        })
        .collect()
    }
}

/// The blocking half: each file read whole and checked against its index entry.
pub fn check(docs: Vec<ToCheck>) -> Vec<DocChecked> {
    (docs.into_iter())
        .map(|(version, path, keep)| {
            let read = match read_stored(&path, &version) {
                Ok(bytes) => Ok(keep.then(|| String::from_utf8_lossy(&bytes).into_owned())),
                Err(ReadError::Mismatch(_)) => Err("its file differs from what was stored".into()),
                Err(ReadError::Io(error)) => Err(error),
            };
            DocChecked {
                kind: version.kind,
                n: version.n,
                read,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "design_restore_tests.rs"]
mod tests;
