//! Milestone 9.6 decision 12: the design documents' files. The engine records a version
//! (`run/design/state.rs::store`) and emits `Effect::WriteDoc`; this file writes it, and
//! reads a version back for `run show` (`RunRequest::ShowDoc`) and `get_doc`.
//!
//! Every write and read runs on `spawn_blocking`, bounded by [`IO_WAIT`], and the engine
//! lock is taken only to look a version up, never across a file access (AGENTS.md rule
//! 2). A version's file is written once: a temp file, fsynced, then hard-linked into
//! place, which fails rather than replace a file (DF §5.3, versions are immutable).

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::run_wire::request;
use proto::{DocFinding, DocKind, DocView, RunReply};

use super::RunService;
use crate::run::design::changes::line_diff;
use crate::run::design::state::{self, DocVersion};
use crate::run::design::template::kind_name;

/// How long a design file's write or read may take before it is given up.
pub const IO_WAIT: Duration = Duration::from_secs(10);
/// A document's text in a reply (`run show`, `get_doc`) is capped at 64 KiB.
pub const DOC_READ_CAP: usize = 64 * 1024;
/// What a diff or a findings file reads at most: past it, the line diff reports the
/// text too large.
const RAW_READ_CAP: u64 = 1024 * 1024;

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
    previous: Option<PathBuf>,
    findings: PathBuf,
}

impl RunService {
    /// `Effect::WriteDoc`: the file, then the index. A failure is logged; the version
    /// stays in the run's index, and reading it then names the error.
    pub(super) async fn write_doc(
        &self,
        path: PathBuf,
        text: String,
        index: Option<(PathBuf, String)>,
    ) {
        let shown = path.clone();
        let wrote = tokio::time::timeout(
            IO_WAIT,
            tokio::task::spawn_blocking(move || {
                write_new(&path, &text)?;
                match index {
                    Some((index, text)) => write_replace(&index, &text),
                    None => Ok(()),
                }
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
            Ok(Ok(view)) => view.map_err(|error| format!("could not read the {what}: {error}")),
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
            previous: (design.previous(version)).map(|p| dir.join(design.file_name(p))),
            findings: dir.join(state::findings_name(version.kind, version.n)),
            version: version.clone(),
        })
    }
}

/// The blocking half of [`RunService::doc_view`].
fn read_view(found: Found, query: &DocQuery) -> Result<DocView, String> {
    let text = read_capped(&found.path, DOC_READ_CAP)?;
    let diff = match (&found.previous, query.diff) {
        (Some(previous), true) => {
            let old = read_raw(previous)?;
            Some(line_diff(&old, &read_raw(&found.path)?))
        }
        _ => None,
    };
    let findings = match query.findings {
        true => read_findings(&found.findings)?,
        false => Vec::new(),
    };
    Ok(DocView {
        run: found.run,
        kind: found.version.kind,
        version: found.version.n,
        text,
        diff,
        findings,
    })
}

/// A version's stored findings, each with its answer; none until a review stored them.
fn read_findings(path: &Path) -> Result<Vec<(DocFinding, Option<String>)>, String> {
    match File::open(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
        Ok(_) => serde_json::from_str(&read_raw(path)?).map_err(|e| e.to_string()),
    }
}

/// Writes `text` to `path`, which must not exist: a temp file beside it, fsynced, then
/// a hard link to `path` (which fails if `path` exists, unlike a rename), then the temp
/// file removed and the folder fsynced. A reader sees no file or the whole file.
pub fn write_new(path: &Path, text: &str) -> Result<(), String> {
    let (dir, tmp) = temp_beside(path)?;
    write_synced(&tmp, text)?;
    let linked = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => sync_dir(&dir),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Err(format!(
            "{} exists; a design document is never rewritten",
            path.display()
        )),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Replaces `path` with `text` atomically: a temp file, fsync, rename (the index).
pub fn write_replace(path: &Path, text: &str) -> Result<(), String> {
    let (dir, tmp) = temp_beside(path)?;
    write_synced(&tmp, text)?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    sync_dir(&dir)
}

fn temp_beside(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let dir = (path.parent()).ok_or_else(|| format!("{} has no folder", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = (path.file_name()).ok_or_else(|| format!("{} has no name", path.display()))?;
    let mut tmp = name.to_os_string();
    tmp.push(".tmp");
    Ok((dir.to_path_buf(), dir.join(tmp)))
}

fn write_synced(tmp: &Path, text: &str) -> Result<(), String> {
    let shown = |e: std::io::Error| format!("{}: {e}", tmp.display());
    let mut file = File::create(tmp).map_err(shown)?;
    file.write_all(text.as_bytes()).map_err(shown)?;
    file.sync_all().map_err(shown)
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    (File::open(dir).and_then(|d| d.sync_all())).map_err(|e| format!("{}: {e}", dir.display()))
}

/// `path`'s text, at most `cap` bytes: a longer file keeps its head, cut at a
/// character, and ends `[cut: <n> bytes]`.
pub fn read_capped(path: &Path, cap: usize) -> Result<String, String> {
    let shown = |e: std::io::Error| format!("{}: {e}", path.display());
    let file = File::open(path).map_err(shown)?;
    let total = file.metadata().map_err(shown)?.len();
    let mut bytes = Vec::new();
    (file.take(cap as u64 + 1))
        .read_to_end(&mut bytes)
        .map_err(shown)?;
    if bytes.len() <= cap {
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    // Room for the marker, so the whole reply stays within the cap.
    let mut text = String::from_utf8_lossy(&bytes[..cap - 32]).into_owned();
    while text.ends_with('\u{fffd}') {
        text.pop();
    }
    let cut = total.saturating_sub(text.len() as u64);
    text.push_str(&format!("\n[cut: {cut} bytes]"));
    Ok(text)
}

/// `path`'s text, up to [`RAW_READ_CAP`], for a diff or a findings file.
fn read_raw(path: &Path) -> Result<String, String> {
    let shown = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut bytes = Vec::new();
    let file = File::open(path).map_err(shown)?;
    (file.take(RAW_READ_CAP))
        .read_to_end(&mut bytes)
        .map_err(shown)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
#[path = "design_io_tests.rs"]
mod tests;
