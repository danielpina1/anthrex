//! Milestone 9.15 decision 12: the distill worker. One tokio task per daemon, started
//! beside `set_adaptation` (`profile/service.rs::wire`), serving the open distill
//! entries of every repository's `lessons.jsonl` one call at a time.
//!
//! It wakes on [`LessonService::wake`] (the `QueueDistill` executor's), and at start
//! scans `<data_dir>/repos/*/lessons.jsonl` for entries left open (a `queued` line with
//! no `distilled` and no `gave_up`). With `[lessons] enabled = false` or
//! `distill = false` in the daemon's config (the review's config gate), neither the scan
//! nor a wake serves anything: open entries are kept as they are, neither called for
//! nor closed, so a daemon started later with distillation on distills them. Otherwise,
//! per repository, oldest entry first:
//! - with the deciders off, the entry is closed `gave_up { reason: "deciders are off" }`
//!   at once, with no attempt (ruling C29: it is never distilled later);
//! - an entry whose last attempt failed waits [`DISTILL_RETRY_SECS`] (attempts 2 and 3),
//!   counted from that attempt's line, so a restart keeps the schedule; after the third
//!   failure it is closed `gave_up` with the last fallback reason;
//! - a repository with `max_distills_per_day` `attempt` lines in the last
//!   [`DAY_SECS`] is skipped until the oldest of them ages out (ruling C19: the cap
//!   never holds another repository); `max_distills_per_day = 0` never calls;
//! - otherwise the `distill` decider is asked with `decide_within`, bounded by
//!   [`DISTILL_BOUND`], and the worker appends `attempt { n, error, usage }`, then
//!   `distilled` on an answer. The call's route is a `role_route` line with no run,
//!   `distill/<queued record id>/<n>`, in the repository's `history.jsonl`.
//!
//! The decider receives only the entry's dossier, which the engine redacted when it
//! queued it and which is cleaned again here; the decider's answer is cleaned and
//! redacted by `decider::distill::parse`. Every file read and write runs on
//! `spawn_blocking`, every append under the repository's `LessonLocks` entry, and no
//! other lock is taken (AGENTS.md rules 2 and 3).

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use proto::{
    AttemptLine, DeciderSource, DistilledLine, GaveUpLine, HistoryLine, LESSONS_VERSION,
    LessonLine, QueuedLine, RoleRoutingInput,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::LESSONS_FILE;
use super::store::{LessonLocks, append_once, read_store};
use crate::decider::call::{Routed, decide_within};
use crate::decider::fallback::OFF_REASON;
use crate::decider::{DeciderAnswer, DeciderContext, DeciderRequest, Decision, DistillInput};

/// One distill call's bound, its routing probe included (decision 12).
pub const DISTILL_BOUND: Duration = Duration::from_secs(120);
/// The waits before attempts 2 and 3, from the failed attempt's line (decision 12).
pub const DISTILL_RETRY_SECS: [u64; 2] = [60, 300];
/// Attempts per entry before it is closed `gave_up`.
pub const DISTILL_ATTEMPTS: u32 = 3;
/// The daily cap's window (`max_distills_per_day`).
pub const DAY_SECS: u64 = 86_400;
/// How long a repository whose store could not be written waits before it is served
/// again, so a full disk never turns into a loop of decider calls.
pub const WRITE_FAILED_WAIT_SECS: u64 = 60;

/// A clock in Unix seconds.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
/// A decider call's future: its route (`None` when the probe outlasted the bound or the
/// deciders are off) and its decision.
pub type DecideFuture = Pin<Box<dyn Future<Output = (Option<Routed>, Decision)> + Send>>;
/// The decider call, given the request and the repository's project (for its
/// `models.toml`).
pub type DecideFn = Arc<dyn Fn(DeciderRequest, Option<PathBuf>) -> DecideFuture + Send + Sync>;

/// The worker's timing (ruling C6): a test injects short bounds, zero retries and a
/// clock of its own, so no test sleeps for them.
#[derive(Clone)]
pub struct ServiceTiming {
    pub bound: Duration,
    pub retry: [Duration; 2],
    pub now: Clock,
}

impl Default for ServiceTiming {
    fn default() -> Self {
        ServiceTiming {
            bound: DISTILL_BOUND,
            retry: DISTILL_RETRY_SECS.map(Duration::from_secs),
            now: Arc::new(crate::run::driver::unix_now),
        }
    }
}

/// What the worker serves with: the daemon's decider context, its data directory, its
/// `[lessons]` table and the one lessons lock map (ruling C9).
pub struct ServiceParts {
    pub deciders: DeciderContext,
    pub data_dir: PathBuf,
    pub config: config::Lessons,
    pub locks: Arc<LessonLocks>,
}

/// The running worker's handle.
pub struct LessonService {
    tx: mpsc::UnboundedSender<PathBuf>,
    cancel: CancellationToken,
    passes: Arc<AtomicU64>,
}

impl LessonService {
    /// Starts the worker with decision 12's timing. Needs a tokio runtime.
    pub fn start(parts: ServiceParts) -> Arc<Self> {
        Self::start_with(parts, ServiceTiming::default())
    }

    /// [`Self::start`] with `timing`; the calls go to `decide_within` on the parts'
    /// decider context.
    pub fn start_with(parts: ServiceParts, timing: ServiceTiming) -> Arc<Self> {
        let ctx = parts.deciders.clone();
        let bound = timing.bound;
        let decide: DecideFn = Arc::new(move |request, project| {
            let ctx = ctx.clone();
            Box::pin(
                async move { decide_within(&ctx, (&request, project.as_deref()), bound).await },
            )
        });
        Self::start_seamed(parts, timing, decide)
    }

    /// [`Self::start_with`] with the call itself injected (ruling C6's seam).
    pub fn start_seamed(parts: ServiceParts, timing: ServiceTiming, decide: DecideFn) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let passes = Arc::new(AtomicU64::new(0));
        let worker = Worker {
            parts,
            timing,
            decide,
            cancel: cancel.clone(),
            passes: passes.clone(),
        };
        tokio::spawn(worker.run(rx));
        Arc::new(LessonService { tx, cancel, passes })
    }

    /// Asks the worker to serve `repo_dir`'s open entries. Never blocks.
    pub fn wake(&self, repo_dir: PathBuf) {
        let _ = self.tx.send(repo_dir);
    }

    /// Stops the worker; a call in flight is dropped, which kills its process.
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    /// How many passes over its repositories the worker has finished: a pass ends when
    /// every entry due was served and the worker is about to wait.
    pub fn passes(&self) -> u64 {
        self.passes.load(Ordering::SeqCst)
    }
}

impl Drop for LessonService {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct Worker {
    parts: ServiceParts,
    timing: ServiceTiming,
    decide: DecideFn,
    cancel: CancellationToken,
    passes: Arc<AtomicU64>,
}

/// One open entry as the worker schedules it.
struct Entry {
    queued: QueuedLine,
    attempts: u32,
    /// The last attempt's `at` and error.
    last: Option<(u64, Option<String>)>,
}

/// What serving a repository left: nothing open, or entries due at a later time.
enum Served {
    Done,
    Later(u64),
    Stopped,
}

impl Worker {
    async fn run(self, mut rx: mpsc::UnboundedReceiver<PathBuf>) {
        let mut repos = tokio::select! {
            _ = self.cancel.cancelled() => return,
            found = self.scan() => found,
        };
        loop {
            while let Ok(repo) = rx.try_recv() {
                repos.insert(repo);
            }
            let mut next: Option<u64> = None;
            for repo in repos.clone() {
                match self.serve(&repo).await {
                    Served::Done => {
                        repos.remove(&repo);
                    }
                    Served::Later(at) => next = Some(next.map_or(at, |n| n.min(at))),
                    Served::Stopped => return,
                }
            }
            self.passes.fetch_add(1, Ordering::SeqCst);
            let wait = next.map(|at| Duration::from_secs(at.saturating_sub(self.now()).max(1)));
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                woken = rx.recv() => match woken {
                    Some(repo) => {
                        repos.insert(repo);
                    }
                    None => return,
                },
                _ = sleep_for(wait) => {}
            }
        }
    }

    fn now(&self) -> u64 {
        (self.timing.now)()
    }

    /// Whether the daemon's `[lessons]` table lets the worker distill at all.
    fn distilling(&self) -> bool {
        self.parts.config.enabled && self.parts.config.distill
    }

    /// Decision 12's start scan: every repository data directory with a store; none
    /// when distillation is off.
    async fn scan(&self) -> BTreeSet<PathBuf> {
        if !self.distilling() {
            return BTreeSet::new();
        }
        let repos = self.parts.data_dir.join("repos");
        let found = tokio::task::spawn_blocking(move || {
            let Ok(entries) = std::fs::read_dir(&repos) else {
                return BTreeSet::new();
            };
            (entries.flatten())
                .map(|entry| entry.path())
                .filter(|dir| dir.join(LESSONS_FILE).is_file())
                .collect()
        })
        .await;
        found.unwrap_or_default()
    }

    /// Serves `repo`'s due entries, one call at a time, until none is due. With
    /// distillation off, nothing: the entries stay open, untouched.
    async fn serve(&self, repo: &Path) -> Served {
        loop {
            if self.cancel.is_cancelled() {
                return Served::Stopped;
            }
            if !self.distilling() {
                return Served::Done;
            }
            let lines = read(repo).await;
            let now = self.now();
            let open = open_entries(&lines);
            if open.is_empty() {
                return Served::Done;
            }
            if self.parts.deciders.mode == proto::DeciderMode::Off {
                for entry in &open {
                    let line = gave_up(&entry.queued.record_id, OFF_REASON, now);
                    if !self.append(repo, line).await {
                        return Served::Later(now + WRITE_FAILED_WAIT_SECS);
                    }
                }
                return Served::Done;
            }
            // An entry whose third attempt was written but not its `gave_up` (a stop
            // between the two) is closed first.
            if let Some(entry) = open.iter().find(|e| e.attempts >= DISTILL_ATTEMPTS) {
                let reason = last_reason(entry);
                let line = gave_up(&entry.queued.record_id, &reason, now);
                if !self.append(repo, line).await {
                    return Served::Later(now + WRITE_FAILED_WAIT_SECS);
                }
                continue;
            }
            let mut later: Option<u64> = None;
            let mut due = None;
            for entry in open {
                let at = self.due(&entry);
                if at <= now {
                    due = Some(entry);
                    break;
                }
                later = Some(later.map_or(at, |l| l.min(at)));
            }
            let Some(entry) = due else {
                return later.map_or(Served::Done, Served::Later);
            };
            let max = u64::from(self.parts.config.max_distills_per_day);
            if max == 0 {
                return Served::Done;
            }
            if let Some(reopen) = capped(&lines, now, max) {
                return Served::Later(later.map_or(reopen, |l| l.min(reopen)));
            }
            match self.attempt(repo, entry, now).await {
                Some(true) => {}
                Some(false) => return Served::Later(now + WRITE_FAILED_WAIT_SECS),
                None => return Served::Stopped,
            }
        }
    }

    /// When `entry`'s next attempt is due: at once for the first, else its last
    /// attempt's `at` plus the retry wait.
    fn due(&self, entry: &Entry) -> u64 {
        match (entry.attempts, &entry.last) {
            (0, _) | (_, None) => 0,
            (n, Some((at, _))) => {
                let wait = self.timing.retry[(n as usize - 1).min(1)];
                at.saturating_add(wait.as_secs())
            }
        }
    }

    /// One call for `entry`, and its lines. `Some(false)`: a line was not written.
    /// `None`: the worker was stopped during the call.
    async fn attempt(&self, repo: &Path, entry: Entry, now: u64) -> Option<bool> {
        let queued = &entry.queued;
        let n = entry.attempts + 1;
        let request = DeciderRequest::Distill(DistillInput {
            cause: queued.cause,
            dossier: super::dossier::clean(&queued.dossier),
        });
        let project = project_of(repo).await;
        let (routed, decision) = tokio::select! {
            _ = self.cancel.cancelled() => return None,
            done = (self.decide)(request, project) => done,
        };
        let at = self.now().max(now);
        if let Some(routed) = &routed {
            if let Some(line) = routed.moved_line() {
                tracing::info!("{line}");
            }
            record_route(repo, queued, n, (routed, &decision), at).await;
        }
        let answer = match (&decision.source, &decision.answer) {
            (DeciderSource::Decider, DeciderAnswer::Distill { items, dropped }) => {
                Some((items.clone(), *dropped))
            }
            _ => None,
        };
        let error = match &answer {
            Some(_) => None,
            None => Some(
                (decision.fallback_reason.clone())
                    .unwrap_or_else(|| "the decider's answer does not match its request".into()),
            ),
        };
        let id = &queued.record_id;
        let line = LessonLine::Attempt(AttemptLine {
            v: LESSONS_VERSION,
            record_id: format!("{id}/attempt/{n}"),
            at,
            queued: id.clone(),
            n: u8::try_from(n).unwrap_or(u8::MAX),
            error: error.clone(),
            usage: decision.usage,
        });
        if !self.append(repo, line).await {
            return Some(false);
        }
        let closing = match (answer, error) {
            (Some((items, dropped)), _) => {
                tracing::info!(
                    "lessons: distilled {id}: {} lessons, {dropped} dropped",
                    items.len()
                );
                Some(LessonLine::Distilled(DistilledLine {
                    v: LESSONS_VERSION,
                    record_id: format!("{id}/distilled"),
                    at,
                    queued: id.clone(),
                    items,
                    dropped,
                    usage: decision.usage,
                }))
            }
            (None, Some(reason)) if n >= DISTILL_ATTEMPTS => Some(gave_up(id, &reason, at)),
            (None, Some(reason)) => {
                tracing::info!("lessons: distill attempt {n} of {id} fell back: {reason}");
                None
            }
            (None, None) => None,
        };
        match closing {
            Some(line) => Some(self.append(repo, line).await),
            None => Some(true),
        }
    }

    /// `line` appended once under the repository's lock; false (and a warning) when
    /// it could not be.
    async fn append(&self, repo: &Path, line: LessonLine) -> bool {
        if let LessonLine::GaveUp(g) = &line {
            tracing::info!("lessons: gave up on {}: {}", g.queued, g.reason);
        }
        let id = line.record_id().to_string();
        match append_once(&self.parts.locks, repo, line).await {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(
                    "lessons: the line {id} was not written to {}: {error}",
                    repo.display()
                );
                false
            }
        }
    }
}

/// `None` waits forever.
async fn sleep_for(wait: Option<Duration>) {
    match wait {
        Some(wait) => tokio::time::sleep(wait).await,
        None => std::future::pending().await,
    }
}

/// The store's lines, read on `spawn_blocking`; an unreadable store is empty.
async fn read(repo: &Path) -> Vec<LessonLine> {
    let path = repo.join(LESSONS_FILE);
    let read = tokio::task::spawn_blocking(move || read_store(&path)).await;
    let (lines, problems) = read.unwrap_or_default();
    for problem in problems.iter().take(1) {
        tracing::warn!("lessons: {}", proto::safe_text::one_line(problem));
    }
    lines
}

/// The open entries, oldest first (then by record id), with their attempts.
fn open_entries(lines: &[LessonLine]) -> Vec<Entry> {
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut closed: BTreeSet<String> = BTreeSet::new();
    for line in lines {
        match line {
            LessonLine::Queued(q) => {
                entries.entry(q.record_id.clone()).or_insert(Entry {
                    queued: q.clone(),
                    attempts: 0,
                    last: None,
                });
            }
            LessonLine::Attempt(a) => {
                if let Some(entry) = entries.get_mut(&a.queued) {
                    entry.attempts += 1;
                    entry.last = Some((a.at, a.error.clone()));
                }
            }
            LessonLine::Distilled(d) => {
                closed.insert(d.queued.clone());
            }
            LessonLine::GaveUp(g) => {
                closed.insert(g.queued.clone());
            }
            _ => {}
        }
    }
    let mut open: Vec<Entry> = (entries.into_values())
        .filter(|e| !closed.contains(&e.queued.record_id))
        .collect();
    open.sort_by(|a, b| {
        (a.queued.at, &a.queued.record_id).cmp(&(b.queued.at, &b.queued.record_id))
    });
    open
}

/// The last attempt's reason, for a `gave_up` written after a stop.
fn last_reason(entry: &Entry) -> String {
    (entry.last.as_ref())
        .and_then(|(_, error)| error.clone())
        .unwrap_or_else(|| "the decider gave no answer".into())
}

/// Ruling C19: when the repository has `max` attempts within the last day, the time
/// the oldest of them ages out.
fn capped(lines: &[LessonLine], now: u64, max: u64) -> Option<u64> {
    let recent: Vec<u64> = (lines.iter())
        .filter_map(|l| match l {
            LessonLine::Attempt(a) if now.saturating_sub(a.at) < DAY_SECS => Some(a.at),
            _ => None,
        })
        .collect();
    let oldest = recent.iter().min()?;
    (recent.len() as u64 >= max).then(|| oldest.saturating_add(DAY_SECS).max(now + 1))
}

fn gave_up(queued: &str, reason: &str, at: u64) -> LessonLine {
    LessonLine::GaveUp(GaveUpLine {
        v: LESSONS_VERSION,
        record_id: format!("{queued}/gave_up"),
        at,
        queued: queued.to_string(),
        reason: reason.to_string(),
    })
}

/// The repository's project, from its stored profile's meta, for the call's route
/// (`models.toml`); `None` routes on the global table. On `spawn_blocking`.
async fn project_of(repo: &Path) -> Option<PathBuf> {
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
async fn record_route(
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

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
