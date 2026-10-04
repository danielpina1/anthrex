//! Milestone 9.6 decision 12: the design documents' files. The engine records a version
//! (`run/design/state.rs::store`) and emits `Effect::WriteDoc`; this file writes it, and
//! reads a version back for `run show` (`RunRequest::ShowDoc`) and `get_doc`.
//!
//! Every write and read runs on `spawn_blocking`, bounded by [`IO_WAIT`], and the engine
//! lock is taken only to look a version up, never across a file access (AGENTS.md rule
//! 2). A version's file is written once (`driver/design_files.rs`), and it is shown only
//! while its bytes are the ones the index recorded (fix round 1, I-1).

use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::run_wire::request;
use proto::{DocFinding, DocKind, DocSeverity, DocView, RunReply};

use super::RunService;
pub use super::design_files::{DocWrites, write_new};
#[cfg(test)]
pub use super::design_files::{make_dirs, temp_name, write_new_at};
use crate::run::design::changes::line_diff;
use crate::run::design::state::{self, DocVersion, sha256_hex};
use crate::run::design::template::kind_name;

/// How long a design file's write or read may take before it is given up.
pub const IO_WAIT: Duration = Duration::from_secs(10);
/// A document's text in a reply (`run show`, `get_doc`) is capped at 64 KiB.
pub const DOC_READ_CAP: usize = 64 * 1024;
/// A reply's findings are capped at 32 KiB of JSON (fix round 1, m5).
pub const FINDINGS_CAP: usize = 32 * 1024;
/// What a findings file reads at most.
const RAW_READ_CAP: u64 = 1024 * 1024;
/// Room kept under a cap for its marker.
const MARKER_ROOM: usize = 40;
/// Room kept under [`FINDINGS_CAP`] for the cut's own entry.
const FINDING_ROOM: usize = 256;

/// What to read of one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocQuery {
    pub kind: DocKind,
    /// `None`: the latest.
    pub version: Option<u32>,
    /// A brainstorm draft's label (`get_doc`'s `from`): that brainstormer's latest.
    pub from: Option<String>,
    pub diff: bool,
    pub findings: bool,
}

/// The files one read needs, found under the engine lock.
struct Found {
    run: String,
    version: DocVersion,
    path: PathBuf,
    /// The previous version and its file, for a diff.
    previous: Option<(DocVersion, PathBuf)>,
    findings: PathBuf,
}

/// Why a read gave nothing: the file differs from the index (refused as it is), or it
/// could not be read.
pub(super) enum ReadError {
    Mismatch(String),
    Io(String),
}

impl RunService {
    /// `Effect::WriteDoc`: the file, then the index, which is numbered now so an older
    /// index never replaces a newer (m2). A failure is logged; the version stays in the
    /// run's index, and reading it then names the error.
    pub(super) async fn write_doc(
        &self,
        path: PathBuf,
        text: String,
        index: Option<(PathBuf, String)>,
    ) {
        let shown = path.clone();
        let index = index.map(|(index, text)| self.doc_writes.index_writer(index, text));
        let wrote = tokio::time::timeout(
            IO_WAIT,
            tokio::task::spawn_blocking(move || {
                write_new(&path, &text)?;
                index.map_or(Ok(()), |write| write())
            }),
        )
        .await;
        match wrote {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                tracing::error!(path = %shown.display(), %error, "design document write failed")
            }
            Ok(Err(error)) => {
                tracing::error!(path = %shown.display(), %error, "design document write panicked")
            }
            Err(_) => tracing::error!(path = %shown.display(), "design document write timed out"),
        }
    }

    /// `RunRequest::ShowDoc`, answered like `TaskDetail` but read off the engine.
    pub(super) async fn show_doc(&self, run_id: String, query: DocQuery) -> RunReply {
        match self.doc_view(&run_id, query).await {
            Ok(doc) => RunReply::Doc {
                doc: Box::new(doc),
                request_id: None,
            },
            Err(message) => RunReply::refused(request::SHOW_DOC, message),
        }
    }

    /// `get_doc` (task M9.6.6): one version's text only, capped at [`DOC_READ_CAP`], with
    /// no diff and no findings; a refusal is the read's own text.
    pub(super) async fn get_doc(
        &self,
        run_id: &str,
        kind: DocKind,
        version: Option<u32>,
        from: Option<String>,
    ) -> Result<String, String> {
        let query = DocQuery {
            kind,
            version,
            from,
            diff: false,
            findings: false,
        };
        Ok(self.doc_view(run_id, query).await?.text)
    }

    /// One version's text, with its diff against the previous version and its findings
    /// when asked: looked up under the engine lock, read after it is released.
    pub(super) async fn doc_view(&self, run_id: &str, query: DocQuery) -> Result<DocView, String> {
        let found = self.find_doc(run_id, &query)?;
        let what = format!("{} v{}", kind_name(query.kind), found.version.n);
        let read = tokio::time::timeout(
            IO_WAIT,
            tokio::task::spawn_blocking(move || read_view(found, &query)),
        )
        .await;
        match read {
            Ok(Ok(Ok(view))) => Ok(view),
            Ok(Ok(Err(ReadError::Mismatch(text)))) => Err(text),
            Ok(Ok(Err(ReadError::Io(error)))) => Err(format!("could not read the {what}: {error}")),
            Ok(Err(error)) => Err(format!("could not read the {what}: {error}")),
            Err(_) => Err(format!(
                "reading the {what} took over {} s",
                IO_WAIT.as_secs()
            )),
        }
    }

    fn find_doc(&self, run_id: &str, query: &DocQuery) -> Result<Found, String> {
        let state = crate::lock(&self.state); // lookup only; no file access under it
        let run = (state.runs.get(run_id)).ok_or_else(|| format!("unknown run {run_id}"))?;
        let design = (run.orch.design.as_ref()).ok_or_else(|| state::not_design(run_id))?;
        let name = kind_name(query.kind);
        let version = match (&query.from, query.version) {
            (Some(label), _) => design
                .draft_from(label)
                .ok_or_else(|| format!("run {run_id} has no {name} from {label}"))?,
            (None, Some(n)) => design
                .find(query.kind, Some(n))
                .ok_or_else(|| format!("run {run_id} has no {name} v{n}"))?,
            (None, None) => design
                .find(query.kind, None)
                .ok_or_else(|| format!("run {run_id} has no {name} yet"))?,
        };
        let dir = state::design_dir(run);
        Ok(Found {
            run: run_id.to_string(),
            path: dir.join(design.file_name(version)),
            previous: (design.previous(version))
                .map(|p| (p.clone(), dir.join(design.file_name(p)))),
            findings: dir.join(state::findings_name(version.kind, version.n)),
            version: version.clone(),
        })
    }
}

/// The blocking half of [`RunService::doc_view`]: each file read whole and checked
/// against its index entry, then capped.
fn read_view(found: Found, query: &DocQuery) -> Result<DocView, ReadError> {
    let bytes = read_stored(&found.path, &found.version)?;
    let diff = match (&found.previous, query.diff) {
        (Some((previous, path)), true) => {
            let old = read_stored(path, previous)?;
            let (old, new) = (
                String::from_utf8_lossy(&old),
                String::from_utf8_lossy(&bytes),
            );
            let diff = line_diff(&old, &new);
            Some(cut_text(diff.as_bytes(), DOC_READ_CAP, "diff cut"))
        }
        _ => None,
    };
    let findings = match query.findings {
        true => cap_findings(read_findings(&found.findings).map_err(ReadError::Io)?),
        false => Vec::new(),
    };
    Ok(DocView {
        run: found.run,
        kind: found.version.kind,
        version: found.version.n,
        text: cut_text(&bytes, DOC_READ_CAP, "cut"),
        diff,
        findings,
    })
}

/// `path`'s bytes, when they are the version's: its length and its SHA-256 (I-1). A
/// file that changed since it was written is never shown.
pub(super) fn read_stored(path: &Path, version: &DocVersion) -> Result<Vec<u8>, ReadError> {
    let shown = |e: std::io::Error| ReadError::Io(format!("{}: {e}", path.display()));
    let mut bytes = Vec::new();
    let file = File::open(path).map_err(shown)?;
    (file.take(version.bytes.saturating_add(1)))
        .read_to_end(&mut bytes)
        .map_err(shown)?;
    if bytes.len() as u64 != version.bytes || sha256_hex(&bytes) != version.sha256 {
        return Err(ReadError::Mismatch(format!(
            "document {} v{} does not match what was stored; it was not shown",
            kind_name(version.kind),
            version.n
        )));
    }
    Ok(bytes)
}

/// `bytes` as text, at most `cap` bytes: a longer text keeps its head, cut at a
/// character, and ends `[<marker>: <n> bytes]`, `n` the bytes left out.
pub fn cut_text(bytes: &[u8], cap: usize, marker: &str) -> String {
    if bytes.len() <= cap {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut text = String::from_utf8_lossy(&bytes[..cap - MARKER_ROOM]).into_owned();
    // A character the cut split is one replacement character at the end.
    while text.ends_with('\u{fffd}') {
        text.pop();
    }
    let cut = bytes.len().saturating_sub(text.len());
    text.push_str(&format!("\n[{marker}: {cut} bytes]"));
    text
}

/// The findings that fit in [`FINDINGS_CAP`] of JSON, in order; when some do not, a
/// last entry says how many were left out (m5).
fn cap_findings(all: Vec<(DocFinding, Option<String>)>) -> Vec<(DocFinding, Option<String>)> {
    let mut used: usize = 2; // `[]`
    let total = all.len();
    let mut kept = Vec::new();
    for item in all {
        let size = serde_json::to_string(&item).map_or(usize::MAX, |t| t.len() + 1);
        if used.saturating_add(size) > FINDINGS_CAP - FINDING_ROOM {
            let cut = DocFinding {
                id: "cut".into(),
                severity: DocSeverity::Minor,
                place: String::new(),
                text: format!("[findings cut: {} more]", total - kept.len()),
            };
            kept.push((cut, None));
            return kept;
        }
        used += size;
        kept.push(item);
    }
    kept
}

/// A version's stored findings, each with its answer; none until a review stored them.
fn read_findings(path: &Path) -> Result<Vec<(DocFinding, Option<String>)>, String> {
    let shown = |e: std::io::Error| format!("{}: {e}", path.display());
    let file = match File::open(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        file => file.map_err(shown)?,
    };
    let mut bytes = Vec::new();
    (file.take(RAW_READ_CAP))
        .read_to_end(&mut bytes)
        .map_err(shown)?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "design_io_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "design_io_tests_files.rs"]
mod tests_files;
