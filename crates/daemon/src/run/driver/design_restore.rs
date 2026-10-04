//! Milestone 9.6 (task 5's carry): after a restore, every design run's stored versions
//! are read back off the engine and checked against their index entries, as `run show`
//! checks them (`design_io.rs`); the engine gets the result as
//! `EventKind::DesignChecked` (`engine/design_gate.rs::checked`): a missing or changed
//! file is logged, an open gate whose version is one reopens as revising, and the
//! latest text of each gate document, and each brainstormer's latest draft, refills the
//! engine's cache of texts.
//!
//! The engine lock is taken only to list the versions; every run's reads run as one
//! `spawn_blocking` task, bounded by [`IO_WAIT`] (AGENTS.md rule 2), after the rest of
//! the restore (task M9.6.7 fix round 1, m5). A read-back that times out or panics
//! leaves the gates as they were stored, and reports each kept brainstorm draft
//! unreadable, so a merged report never waits for it forever (ruling T9-1a).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use proto::DocKind;

use super::RunService;
use super::design_io::{IO_WAIT, ReadError, read_stored};
use crate::run::design::state::{self, DesignState, DocVersion};
use crate::run::engine::{DocChecked, EventKind};

/// One version to read back: its index entry, its file, and whether its text is kept.
pub type ToCheck = (DocVersion, PathBuf, bool);

impl RunService {
    /// Reads back every design run's versions and sends each run's result to the
    /// engine. Called last by `restore`.
    pub(super) async fn check_design_docs(&self) {
        self.check_design_docs_with(IO_WAIT, check_all).await;
    }

    /// The read-back, bounded by `wait`, with its blocking half `read` ([`check_all`];
    /// a test's seam).
    pub(super) async fn check_design_docs_with<F>(&self, wait: Duration, read: F)
    where
        F: FnOnce(Vec<(String, Vec<ToCheck>)>) -> Vec<(String, Vec<DocChecked>)> + Send + 'static,
    {
        let runs = self.design_docs();
        if runs.is_empty() {
            return;
        }
        let drafts = kept_drafts(&runs);
        let read = move || read(runs);
        let why = match tokio::time::timeout(wait, tokio::task::spawn_blocking(read)).await {
            Ok(Ok(all)) => {
                for (run_id, checked) in all {
                    self.send(EventKind::DesignChecked { run_id, checked });
                }
                return;
            }
            Ok(Err(error)) => {
                tracing::error!(%error, "checking the design documents panicked");
                format!("its read failed: {error}")
            }
            Err(_) => {
                tracing::error!("checking the design documents timed out");
                format!("its read took over {} ms", wait.as_millis())
            }
        };
        // Ruling T9-1a: a merged report waits for its drafts' texts, so each kept draft
        // is reported unreadable rather than left unanswered; the gates are left as
        // stored.
        for (run_id, named) in drafts {
            let checked = unread(&named, why.clone());
            self.send(EventKind::DesignChecked { run_id, checked });
        }
    }

    /// `Effect::ReadBack` (task M9.6.10): `docs` read back off the engine lock, on its
    /// own task, each checked against its index entry and its text kept, then sent as
    /// `EventKind::DesignChecked`. A read that failed, panicked or passed [`IO_WAIT`] is
    /// sent as each version's error, so the engine never waits for it.
    pub(super) fn read_back(self: &Arc<Self>, run_id: String, docs: Vec<(DocVersion, PathBuf)>) {
        let service = self.clone();
        tokio::spawn(async move {
            let named: Vec<(DocKind, u32)> = docs.iter().map(|(v, _)| (v.kind, v.n)).collect();
            let docs = (docs.into_iter())
                .map(|(v, path)| (v, path, true))
                .collect();
            let read = tokio::task::spawn_blocking(move || check(docs));
            let checked = match tokio::time::timeout(IO_WAIT, read).await {
                Ok(Ok(checked)) => checked,
                Ok(Err(error)) => unread(&named, format!("its read failed: {error}")),
                Err(_) => unread(
                    &named,
                    format!("its read took over {} s", IO_WAIT.as_secs()),
                ),
            };
            if !service.stopped.load(Ordering::SeqCst) {
                service.send(EventKind::DesignChecked { run_id, checked });
            }
        });
    }

    /// Each design run's versions, looked up under the engine lock. A spec's review
    /// drafts are not gate documents (ruling T5-1) and are not checked.
    fn design_docs(&self) -> Vec<(String, Vec<ToCheck>)> {
        let state = crate::lock(&self.state); // lookup only; no file access under it
        let runs = state.runs.values().filter(|r| !r.state.is_terminal());
        runs.filter_map(|run| {
            let design = run.orch.design.as_ref()?;
            let dir = state::design_dir(run);
            let docs = (design.versions.iter())
                .filter(|v| v.draft_review.is_none())
                .map(|v| (v.clone(), dir.join(design.file_name(v)), kept(design, v)))
                .collect();
            Some((run.id.clone(), docs))
        })
        .collect()
    }
}

/// Whether `v`'s text is kept once read back: it is the latest version of a gate's
/// document (what the next version's change summary compares against), or its
/// brainstormer's latest draft (what the merged report's appendix attaches, task
/// M9.6.9), or the approved spec whose requirements are not stored yet (task M9.6.10).
pub(super) fn kept(design: &DesignState, v: &DocVersion) -> bool {
    let due = design
        .approved_spec
        .filter(|_| design.requirements.is_empty());
    if v.kind == DocKind::Spec && due == Some(v.n) {
        return true;
    }
    let latest = match (v.kind, v.label()) {
        (DocKind::BrainstormDraft, Some(label)) => design.draft_from(label),
        (DocKind::BrainstormDraft, None) => None,
        (kind, _) => design.find(kind, None),
    };
    v.n > 0 && latest.is_some_and(|l| l.n == v.n)
}

/// Each run's brainstorm drafts whose text the read-back keeps: what a merged report
/// waits for.
fn kept_drafts(runs: &[(String, Vec<ToCheck>)]) -> Vec<(String, Vec<(DocKind, u32)>)> {
    (runs.iter())
        .map(|(run_id, docs)| {
            let drafts = (docs.iter())
                .filter(|(v, _, keep)| *keep && v.kind == DocKind::BrainstormDraft)
                .map(|(v, _, _)| (v.kind, v.n))
                .collect::<Vec<_>>();
            (run_id.clone(), drafts)
        })
        .filter(|(_, drafts)| !drafts.is_empty())
        .collect()
}

/// Every one of `named`'s versions, unread for `reason`.
fn unread(named: &[(DocKind, u32)], reason: String) -> Vec<DocChecked> {
    (named.iter())
        .map(|&(kind, n)| DocChecked {
            kind,
            n,
            read: Err(reason.clone()),
        })
        .collect()
}

/// The blocking half for every run: [`check`] each run's versions.
fn check_all(runs: Vec<(String, Vec<ToCheck>)>) -> Vec<(String, Vec<DocChecked>)> {
    (runs.into_iter())
        .map(|(run_id, docs)| (run_id, check(docs)))
        .collect()
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
