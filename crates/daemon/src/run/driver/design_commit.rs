//! Milestone 9.6 decisions 23 and 24 (DF §5.3), I/O: `OpKind::CommitDesignDocs`,
//! executed. The approved versions are read back off the engine, each against the size
//! and SHA-256 its index entry recorded (only the stored, approved bytes are ever
//! committed), on `spawn_blocking` within [`IO_WAIT`]; each is named
//! `<docs_dir>/<folder>/<date>-<slug>.md`, the slug from the spec's title. Then the
//! whole commit is one write through the project's `GitQueue` (`git::commit_docs`, on
//! its blocking thread, each git call with its timeout), with the driver's own
//! temporary index in the run's design folder. No lock is held here (AGENTS.md rules
//! 2, 10 and 11).

use std::fs::File;
use std::io::Read;
use std::sync::Arc;

use super::design_io::IO_WAIT;
use super::ops::failed;
use super::{OpCtx, RunService};
use crate::run::design::commit::{DocSource, repo_path, slug};
use crate::run::design::state::{DESIGN_DIR, sha256_hex};
use crate::run::engine::{OpKind, OpResult};
use crate::run::git::{self, DocsCommit, DocsOutcome};

/// The driver's own index for the documents commit, in the run's design folder.
pub const INDEX_FILE: &str = "commit.index";
/// The folder the spec goes to: its title names every document (decision 24).
const SPEC_FOLDER: &str = "specs";

/// `CommitDesignDocs`' executor contract (`engine/ops.rs`).
pub(super) async fn run(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    let OpKind::CommitDesignDocs(spec) = kind else {
        unreachable!("design_commit::run takes a CommitDesignDocs");
    };
    let sources = spec.files.clone();
    let read = tokio::time::timeout(
        IO_WAIT,
        tokio::task::spawn_blocking(move || {
            sources.iter().map(read_back).collect::<Result<Vec<_>, _>>()
        }),
    )
    .await;
    let texts: Vec<Vec<u8>> = match read {
        Ok(Ok(Ok(texts))) => texts,
        Ok(Ok(Err(error))) => return failed(error),
        Ok(Err(error)) => return failed(format!("reading the documents back failed: {error}")),
        Err(_) => {
            let secs = IO_WAIT.as_secs();
            return failed(format!("reading the documents back took over {secs} s"));
        }
    };
    let spec_text = (spec.files.iter().zip(&texts))
        .find(|(f, _)| f.folder == SPEC_FOLDER)
        .map(|(_, text)| String::from_utf8_lossy(text).into_owned());
    let slug = slug(&spec_text.unwrap_or_default(), &spec.run_id);
    let path = |f: &DocSource| repo_path(&spec.docs_dir, &f.folder, &spec.date, &slug);
    let spec_path = repo_path(&spec.docs_dir, SPEC_FOLDER, &spec.date, &slug);
    let files: Vec<(String, Vec<u8>)> = spec.files.iter().map(path).zip(texts).collect();
    let index = ctx.data_dir.join(DESIGN_DIR).join(INDEX_FILE);
    let committed = service
        .write(ctx, move |g, t| {
            let docs = DocsCommit {
                root: &spec.root,
                index: &index,
                branch: &spec.branch,
                expected: &spec.expected_head,
                integration: &spec.integration,
                files: &files,
                message: &spec.message,
            };
            git::commit_docs(g, &docs, t)
        })
        .await;
    match committed {
        Ok(DocsOutcome::Committed { head, reattach }) => {
            // As a merge: the branch holds the commit whatever the reattach did.
            if let Some(error) = reattach {
                tracing::warn!(
                    run = %ctx.run_id,
                    %error,
                    "committed the documents, but the integration worktree could not go back on its branch"
                );
            }
            OpResult::DocsCommitted {
                head,
                spec: spec_path,
            }
        }
        Ok(DocsOutcome::Symlink { path }) => OpResult::DocsThroughSymlink { path },
        Err(error) => failed(error),
    }
}

/// `source`'s bytes, when they are what its index entry recorded.
fn read_back(source: &DocSource) -> Result<Vec<u8>, String> {
    let fail = |reason: String| {
        format!(
            "the stored {} could not be read back: {reason}",
            source.what
        )
    };
    let shown = |e: std::io::Error| fail(format!("{}: {e}", source.path.display()));
    let mut bytes = Vec::new();
    let file = File::open(&source.path).map_err(shown)?;
    (file.take(source.bytes.saturating_add(1)))
        .read_to_end(&mut bytes)
        .map_err(shown)?;
    if bytes.len() as u64 != source.bytes || sha256_hex(&bytes) != source.sha256 {
        return Err(fail(format!(
            "{} is not what was stored",
            source.path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "design_commit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "design_commit_queue_tests.rs"]
mod queue_tests;
