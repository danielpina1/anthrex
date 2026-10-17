//! Milestone 9.15 decision 19 (task M9.15.5): the four `LessonsRequest`s.
//!
//! `dir` is resolved to its project as `run stats` resolves it (ruling C20: git with
//! `--no-optional-locks` and a scrubbed environment, within `git_timeout_secs`, on
//! `spawn_blocking`). Every store and history read runs on `spawn_blocking` too.
//! `Decide` and `Forget` take the repository's [`LessonLocks`] entry, read the store
//! and the history again under it and re-derive the request (milestone 9.5 decision
//! 11's shape): an id that is unknown, or a lesson in a state the request does not
//! apply to, refuses the whole request and writes nothing. Nothing here takes the
//! manager lock or the engine lock. `List` and `Graph` do not detect reverts: credit is
//! as of the last `run start` or `run stats` detection (C20).
//!
//! `Graph` (task M9.15.13, decision 41) reads the store, the history and the module
//! source on `spawn_blocking`, off every lock, and builds the graph purely
//! (`lessons::graph::build`). Its modules are the newest entry of `module-graph.json`,
//! else the stored profile's `modules` matched in the checkout, else none; `cargo` is
//! never run for it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::bounds::GRAPH_MODULES_BOUND;
use proto::run_wire::request;
use proto::{
    DismissedLine, HistoryLine, LESSONS_VERSION, LessonGraph, LessonInfo, LessonLine, LessonOrigin,
    LessonStatus, LessonsRequest, ModuleSource, RetiredLine, RunReply, VerdictLine,
};

use super::adapt::checkout_of;
use super::{RunService, unix_now};
use crate::lessons::LESSONS_FILE;
use crate::lessons::consolidate::consolidate;
use crate::lessons::fold::{Lesson, fold};
use crate::lessons::graph::build;
use crate::lessons::store::{LessonLocks, read_store};
use crate::run::engine::HISTORY_FILE;
use crate::run::history_io::{append_json, read_history};

/// The reason a forgotten distilled lesson is retired with (decision 19).
pub const FORGOTTEN: &str = "forgotten by the user";
/// The shortest id prefix a request may name a lesson by (decision 19).
pub const PREFIX_MIN: usize = 6;

/// Where a request's repository is found, and the `[lessons]` table it is read with.
#[derive(Clone)]
pub struct LessonsAt {
    pub git: OsString,
    pub data_dir: PathBuf,
    pub git_timeout: Duration,
    pub cfg: config::Lessons,
}

impl RunService {
    /// `RunRequest::Lessons`.
    pub(in crate::run::driver) async fn lessons(&self, req: LessonsRequest) -> RunReply {
        let at = LessonsAt {
            git: self.ctx.git.clone(),
            data_dir: self.ctx.data_dir.clone(),
            git_timeout: Duration::from_secs(
                self.ctx.settings.current().orchestrator.git_timeout_secs,
            ),
            cfg: self.ctx.lessons.clone(),
        };
        answer(&self.lessons, at, req, unix_now()).await
    }
}

/// Answers `req` at `now`: `List` with `RunReply::Lessons`, `Graph` with
/// `RunReply::LessonsGraph`, `Decide` and `Forget` with `Done` (`approved <n>`,
/// `rejected <n>`, `forgot <n>`), every refusal with `Refused` under `request::LESSONS`.
pub async fn answer(locks: &LessonLocks, at: LessonsAt, req: LessonsRequest, now: u64) -> RunReply {
    let refused = |message: String| RunReply::refused(request::LESSONS, message);
    let dir = match &req {
        LessonsRequest::List { dir, .. }
        | LessonsRequest::Decide { dir, .. }
        | LessonsRequest::Forget { dir, .. }
        | LessonsRequest::Graph { dir } => dir.clone(),
    };
    let (git, data_dir, timeout) = (at.git.clone(), at.data_dir.clone(), at.git_timeout);
    let (checkout, project, repo_dir) = match blocking(move || {
        let (checkout, project) = checkout_of(&git, &dir, timeout)?;
        let repo_dir = crate::profile::repo_dir(&data_dir, &project);
        Ok((checkout, project, repo_dir))
    })
    .await
    {
        Ok(found) => found,
        Err(message) => return refused(message),
    };
    let cfg = at.cfg;
    let result = match req {
        LessonsRequest::Graph { .. } => {
            let work = move || Ok(graph_at(&repo_dir, &checkout, project, now, &cfg));
            return match blocking(work).await {
                Ok(graph) => RunReply::LessonsGraph {
                    graph: Box::new(graph),
                    request_id: None,
                },
                Err(message) => refused(message),
            };
        }
        LessonsRequest::List { all, .. } => {
            return match blocking(move || Ok(list_at(&repo_dir, all, now, &cfg))).await {
                Ok(lessons) => RunReply::Lessons {
                    lessons,
                    message: None,
                    request_id: None,
                },
                Err(message) => refused(message),
            };
        }
        LessonsRequest::Decide { ids, approve, .. } => {
            decide(locks, &repo_dir, ids, approve, now, cfg).await
        }
        LessonsRequest::Forget { ids, .. } => forget(locks, &repo_dir, ids, now, cfg).await,
    };
    match result {
        Ok(message) => RunReply::done(request::LESSONS, message),
        Err(message) => refused(message),
    }
}

/// `Decide` under `repo_dir`'s lock: approve or reject every lesson `ids` names.
pub async fn decide(
    locks: &LessonLocks,
    repo_dir: &Path,
    ids: Vec<String>,
    approve: bool,
    now: u64,
    cfg: config::Lessons,
) -> Result<String, String> {
    let _guard = locks.lock(repo_dir).await;
    let repo_dir = repo_dir.to_path_buf();
    blocking(move || decide_at(&repo_dir, &ids, approve, now, &cfg)).await
}

/// `Forget` under `repo_dir`'s lock: retire or dismiss every lesson `ids` names.
pub async fn forget(
    locks: &LessonLocks,
    repo_dir: &Path,
    ids: Vec<String>,
    now: u64,
    cfg: config::Lessons,
) -> Result<String, String> {
    let _guard = locks.lock(repo_dir).await;
    let repo_dir = repo_dir.to_path_buf();
    blocking(move || forget_at(&repo_dir, &ids, now, &cfg)).await
}

/// Every lesson of `repo_dir` as consolidation orders it; without `all`, the rejected
/// and retired ones are left out. Blocking.
pub fn list_at(repo_dir: &Path, all: bool, now: u64, cfg: &config::Lessons) -> Vec<LessonInfo> {
    (lessons_at(repo_dir, now, cfg).iter())
        .filter(|l| all || !matches!(l.status, LessonStatus::Rejected | LessonStatus::Retired))
        .map(Lesson::info)
        .collect()
}

/// `Graph`'s blocking core (decision 41): the store and the history of `repo_dir`,
/// consolidated at `now`, drawn over [`modules_at`]'s modules, for `project`.
pub fn graph_at(
    repo_dir: &Path,
    checkout: &Path,
    project: PathBuf,
    now: u64,
    cfg: &config::Lessons,
) -> LessonGraph {
    graph_within(repo_dir, checkout, project, now, cfg, GRAPH_MODULES_BOUND)
}

/// [`graph_at`] with its module walk's bound (the tests' seam). Task 9's review, B2:
/// the deadline is taken after the store read and the consolidation, right before
/// [`modules_at_until`], so a slow store does not spend the walk's bound.
fn graph_within(
    repo_dir: &Path,
    checkout: &Path,
    project: PathBuf,
    now: u64,
    cfg: &config::Lessons,
    bound: Duration,
) -> LessonGraph {
    let (lines, _) = read_store(&repo_dir.join(LESSONS_FILE));
    let store = fold(&lines);
    let (history, _): (Vec<HistoryLine>, _) = read_history(&repo_dir.join(HISTORY_FILE));
    let lessons = consolidate(&store, &history, now, cfg);
    let (modules, source) = modules_at_until(repo_dir, checkout, Instant::now() + bound);
    LessonGraph {
        project,
        ..build(&store, &lessons, &modules, source)
    }
}

/// Decision 41's modules, as `(name, directory)`: the newest entry of `repo_dir`'s
/// module graph cache (ruling C17), else the stored profile's `modules` patterns
/// matched in `checkout` (each named by its last component, or by its whole directory
/// when two share one), else none. Nothing is run. Blocking, for at most
/// [`GRAPH_MODULES_BOUND`] of walking ([`modules_at_until`]).
pub fn modules_at(
    repo_dir: &Path,
    checkout: &Path,
) -> (Vec<(String, Option<String>)>, ModuleSource) {
    modules_at_until(repo_dir, checkout, Instant::now() + GRAPH_MODULES_BOUND)
}

/// [`modules_at`] with the profile walk's deadline: milestone 9.18 decision 11. Past
/// it the source is [`ModuleSource::None`] with no modules, as with no profile; the
/// module graph cache is read whatever the deadline. Ruling C17: one `read_dir` that
/// hangs is still unbounded (the request is on `spawn_blocking`).
pub(crate) fn modules_at_until(
    repo_dir: &Path,
    checkout: &Path,
    deadline: Instant,
) -> (Vec<(String, Option<String>)>, ModuleSource) {
    if let Some(graph) = super::graph::cache_newest(&repo_dir.join(super::graph::GRAPH_CACHE_FILE))
    {
        let modules = (graph.modules.into_iter())
            .map(|(name, info)| (name, info.dir))
            .collect();
        return (modules, ModuleSource::CargoCache);
    }
    let crate::profile::store::Stored::Found { profile, .. } =
        crate::profile::store::load(repo_dir)
    else {
        return (Vec::new(), ModuleSource::None);
    };
    if profile.modules.is_empty() {
        return (Vec::new(), ModuleSource::None);
    }
    let Some(dirs) =
        crate::profile::verify_tiers::module_dirs_until(checkout, &profile.modules, deadline)
    else {
        return (Vec::new(), ModuleSource::None);
    };
    let last = |dir: &str| dir.rsplit('/').next().unwrap_or(dir).to_string();
    let modules = (dirs.iter())
        .map(|dir| {
            let shared = dirs.iter().filter(|d| last(d) == last(dir)).count() > 1;
            let name = if shared { dir.clone() } else { last(dir) };
            (name, Some(dir.clone()))
        })
        .collect();
    (modules, ModuleSource::Profile)
}

/// `Decide`'s blocking core; the caller holds `repo_dir`'s lock.
pub fn decide_at(
    repo_dir: &Path,
    ids: &[String],
    approve: bool,
    now: u64,
    cfg: &config::Lessons,
) -> Result<String, String> {
    let lessons = lessons_at(repo_dir, now, cfg);
    let chosen = resolve_all(&lessons, ids)?;
    for lesson in &chosen {
        if !matches!(
            lesson.status,
            LessonStatus::Proposed | LessonStatus::OnTrial
        ) {
            return Err(format!(
                "{} is {}; only a proposed or on-trial lesson can be approved or rejected",
                lesson.id,
                lesson.status.name()
            ));
        }
    }
    let lines = chosen.iter().map(|lesson| {
        LessonLine::Verdict(VerdictLine {
            v: LESSONS_VERSION,
            record_id: format!("{}/verdict/{now}", lesson.id),
            at: now,
            id: lesson.id.clone(),
            approved: approve,
        })
    });
    write_all(repo_dir, lines.collect())?;
    let verb = if approve { "approved" } else { "rejected" };
    Ok(format!("{verb} {}", chosen.len()))
}

/// `Forget`'s blocking core; the caller holds `repo_dir`'s lock.
pub fn forget_at(
    repo_dir: &Path,
    ids: &[String],
    now: u64,
    cfg: &config::Lessons,
) -> Result<String, String> {
    let lessons = lessons_at(repo_dir, now, cfg);
    let chosen = resolve_all(&lessons, ids)?;
    for lesson in &chosen {
        if matches!(
            lesson.status,
            LessonStatus::Rejected | LessonStatus::Retired
        ) {
            return Err(format!(
                "{} is {}; it is already gone",
                lesson.id,
                lesson.status.name()
            ));
        }
    }
    let lines = chosen.iter().map(|lesson| match lesson.origin {
        LessonOrigin::Signature => {
            let fp = lesson.id.strip_prefix("kf-").unwrap_or(&lesson.id);
            LessonLine::Dismissed(DismissedLine {
                v: LESSONS_VERSION,
                record_id: format!("kf-{fp}/dismissed"),
                at: now,
                fp: fp.to_string(),
            })
        }
        LessonOrigin::Distilled => LessonLine::Retired(RetiredLine {
            v: LESSONS_VERSION,
            record_id: format!("{}/retired", lesson.id),
            at: now,
            id: lesson.id.clone(),
            reason: FORGOTTEN.to_string(),
        }),
    });
    write_all(repo_dir, lines.collect())?;
    Ok(format!("forgot {}", chosen.len()))
}

/// The store and the history of `repo_dir`, consolidated at `now`. Blocking.
fn lessons_at(repo_dir: &Path, now: u64, cfg: &config::Lessons) -> Vec<Lesson> {
    let (lines, _) = read_store(&repo_dir.join(LESSONS_FILE));
    let (history, _): (Vec<HistoryLine>, _) = read_history(&repo_dir.join(HISTORY_FILE));
    consolidate(&fold(&lines), &history, now, cfg)
}

/// Each of `ids` resolved, in order, a lesson named twice kept once; the first id that
/// names no lesson, or more than one, refuses them all.
fn resolve_all<'a>(lessons: &'a [Lesson], ids: &[String]) -> Result<Vec<&'a Lesson>, String> {
    let mut chosen: Vec<&Lesson> = Vec::new();
    for id in ids {
        let lesson = resolve(lessons, id)?;
        if !chosen.iter().any(|l| l.id == lesson.id) {
            chosen.push(lesson);
        }
    }
    if chosen.is_empty() {
        return Err("no lesson named".to_string());
    }
    Ok(chosen)
}

/// Decision 19: the full id, or a prefix of at least [`PREFIX_MIN`] characters that
/// names exactly one lesson.
fn resolve<'a>(lessons: &'a [Lesson], id: &str) -> Result<&'a Lesson, String> {
    if let Some(exact) = lessons.iter().find(|l| l.id == id) {
        return Ok(exact);
    }
    let unknown = || format!("unknown lesson {}", proto::safe_text::one_line(id));
    if id.chars().count() < PREFIX_MIN {
        return Err(unknown());
    }
    let matches: Vec<&Lesson> = lessons.iter().filter(|l| l.id.starts_with(id)).collect();
    match matches.as_slice() {
        [] => Err(unknown()),
        [one] => Ok(one),
        many => Err(format!(
            "{} names {} lessons",
            proto::safe_text::one_line(id),
            many.len()
        )),
    }
}

/// Appends `lines` to `repo_dir`'s store in order.
fn write_all(repo_dir: &Path, lines: Vec<LessonLine>) -> Result<(), String> {
    let path = repo_dir.join(LESSONS_FILE);
    for line in &lines {
        append_json(&path, line)
            .map_err(|error| format!("could not append to {}: {error}", path.display()))?;
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?
}

#[cfg(test)]
#[path = "lessons_requests_tests.rs"]
mod tests;
