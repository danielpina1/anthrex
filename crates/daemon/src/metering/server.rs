//! The OTLP/HTTP-JSON receiver (M8b decision 30). I/O.
//!
//! A minimal HTTP/1.1 server on `127.0.0.1` for one route, `POST /v1/metrics`, with no
//! HTTP crate ([`super::http`] reads and answers). It never blocks the daemon and never
//! trusts its input:
//!
//! - every connection is its own task, and at most [`OTLP_BASE_CONNECTIONS`] plus the
//!   live orchestrators are served at once (a connection beyond that first closes one
//!   that never presented a valid token, then waits up to [`OTLP_SLOT_WAIT`] for a slot,
//!   then is closed), so one slow client never delays another, memory stays bounded,
//!   and a local process holding slots cannot starve an orchestrator (milestone 9
//!   decision 14b);
//! - only points carrying their run's token (`Authorization: Bearer <token>`,
//!   [`UsageSink::token`]) are metered; the rest are dropped and counted per run
//!   (decision 14a);
//! - each request is read within [`OTLP_READ_TIMEOUT`], headers up to
//!   [`OTLP_MAX_HEADERS`] and a `Content-Length` or chunked body up to
//!   [`OTLP_MAX_BODY`]; anything larger is `413` and the connection is closed;
//! - the body is parsed, applied to the ledger and its totals posted on
//!   `spawn_blocking`, under the ledger lock, so totals are posted in order;
//! - only runs the daemon has now and that have not ended are metered
//!   ([`UsageSink::is_live`]); a run that went is evicted from the ledger (review I3);
//! - it makes no outbound call and never logs a body (ruling R-T1-5): exports carry the
//!   user's account identity.
//!
//! Answers: `200 {}`; `400` for a malformed request or body; `404` for another path;
//! `405` for another method; `415` for another content type (logged once per daemon);
//! `413` for oversize. After each accepted request, the ledger's new total for every
//! `(run, "orchestrator")` it touched goes to [`UsageSink::post`], once per run.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::TokenUsage;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::http::{Conn, Request, Unread};
use super::otlp::{ORCHESTRATOR, OtlpLedger, UsagePoint, parse_metrics};
use slots::{ConnState, Slots};
use token::authorize;

#[path = "slots.rs"]
mod slots;
#[path = "token.rs"]
mod token;

pub const OTLP_MAX_BODY: usize = 4 << 20;
pub const OTLP_MAX_HEADERS: usize = 16 << 10;
pub const OTLP_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections served at once with no live orchestrator (milestone 9 decision 14b: the
/// cap is this plus [`UsageSink::live_orchestrators`]). Bounds memory at about this many
/// bodies.
pub const OTLP_BASE_CONNECTIONS: usize = 8;
/// How long a connection beyond the cap waits for a slot before it is
/// closed unanswered. New connections wait in the listen backlog meanwhile.
pub const OTLP_SLOT_WAIT: Duration = Duration::from_secs(1);
/// M9.10 review: a connection younger than this is never closed to make room, so a real
/// orchestrator's new connection is not closed before its first POST marks it tokened.
pub const OTLP_EVICT_GRACE: Duration = Duration::from_secs(2);
/// `<data_dir>/otlp.addr`: `http://127.0.0.1:<port>`, mode 0600, removed at shutdown.
pub const ADDR_FILE: &str = "otlp.addr";

/// Where the receiver's totals go, and which runs it may meter (review I3, I4). The
/// daemon's is the run service; every method is cheap and never waits on the engine
/// or the window manager.
pub trait UsageSink: Send + Sync + 'static {
    /// Whether `run_id` is a run the daemon has now and that has not ended.
    fn is_live(&self, run_id: &str) -> bool;
    /// Changes whenever the live runs change, so the ledger evicts the runs that went.
    fn live_generation(&self) -> u64;
    /// The ledger's new total for `(run_id, "orchestrator")`. The sink coalesces: the
    /// engine sees at most one pending total per run, the latest. Called under the
    /// ledger lock, so totals arrive in order: it must stay cheap, never block, and
    /// never take the ledger, the engine or the window manager's lock.
    fn post(&self, run_id: String, usage: TokenUsage);
    /// Milestone 9 decision 14a: the live run's OTLP token, `None` when it has none.
    fn token(&self, run_id: &str) -> Option<String>;
    /// Decision 14b: the live runs with an orchestrator, which widen the connection cap.
    fn live_orchestrators(&self) -> usize;
}

/// The bound receiver. Its task stops when the shutdown token is cancelled, and removes
/// the address file as it stops.
pub struct OtlpServer {
    pub addr: SocketAddr,
    task: JoinHandle<()>,
}

impl OtlpServer {
    /// Waits for the accept loop to stop and the address file to be removed.
    pub async fn stopped(self) {
        let _ = self.task.await;
    }
}

/// The ledger and the live-run generation it last evicted for.
#[derive(Default)]
struct Ledger {
    ledger: OtlpLedger,
    generation: Option<u64>,
    /// Decision 14a: points dropped for a missing or wrong token, per live run.
    drops: BTreeMap<String, u64>,
}

struct Shared {
    ledger: Mutex<Ledger>,
    sink: Arc<dyn UsageSink>,
    /// `415` is logged once per daemon.
    logged_type: AtomicBool,
}

/// Binds `127.0.0.1:<port>` (0 picks a free port), writes [`ADDR_FILE`] in `data_dir`,
/// and serves until `shutdown` is cancelled.
pub async fn bind(
    port: u16,
    data_dir: &Path,
    sink: Arc<dyn UsageSink>,
    shutdown: CancellationToken,
) -> std::io::Result<OtlpServer> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let addr = listener.local_addr()?;
    let path = data_dir.join(ADDR_FILE);
    let text = format!("http://{addr}");
    let written = path.clone();
    tokio::task::spawn_blocking(move || write_addr(&written, &text))
        .await
        .map_err(std::io::Error::other)??;
    let shared = Arc::new(Shared {
        ledger: Mutex::new(Ledger::default()),
        sink,
        logged_type: AtomicBool::new(false),
    });
    let task = tokio::spawn(accept_loop(listener, shared, shutdown, path));
    Ok(OtlpServer { addr, task })
}

/// The daemon's wiring (`lifecycle::run`): with `metering.otlp`, binds the receiver with
/// the run service as its sink. A bind failure is logged and metering is off for this
/// daemon; it never stops the daemon. With `otlp` off, a stale address file from an
/// earlier daemon is removed.
pub async fn start(
    metering: &config::Metering,
    data_dir: &Path,
    runs: Arc<crate::run::driver::RunService>,
    shutdown: CancellationToken,
) -> Option<OtlpServer> {
    if !metering.otlp {
        let stale = data_dir.join(ADDR_FILE);
        let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(stale)).await;
        return None;
    }
    match bind(metering.otlp_port, data_dir, runs, shutdown).await {
        Ok(server) => {
            tracing::info!(addr = %server.addr, "OTLP receiver listening");
            Some(server)
        }
        Err(error) => {
            tracing::error!(%error, port = metering.otlp_port, "the OTLP receiver could not bind; orchestrator metering is off");
            None
        }
    }
}

/// Written through a temporary file and a rename, readable by the user only.
fn write_addr(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("addr.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(&tmp, path)
}

async fn accept_loop(
    listener: TcpListener,
    shared: Arc<Shared>,
    shutdown: CancellationToken,
    path: PathBuf,
) {
    let slots = Arc::new(Slots::default());
    let mut cap = (None, OTLP_BASE_CONNECTIONS);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    // Decision 14b: the cap is read again when the live runs change.
                    let generation = shared.sink.live_generation();
                    if cap.0 != Some(generation) {
                        cap = (Some(generation), OTLP_BASE_CONNECTIONS + shared.sink.live_orchestrators());
                    }
                    let slot = tokio::select! {
                        _ = shutdown.cancelled() => break,
                        slot = slots.acquire(cap.1) => slot,
                    };
                    // No slot in time: the connection is dropped, which closes it.
                    let Some(slot) = slot else { continue };
                    let shared = shared.clone();
                    let stop = shutdown.clone();
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = stop.cancelled() => {}
                            _ = slot.state.close.cancelled() => {}
                            _ = serve(stream, &shared, &slot.state) => {}
                        }
                        drop(slot);
                    });
                }
                Err(error) => {
                    // Out of descriptors, say: back off instead of spinning.
                    tracing::warn!(%error, "the OTLP receiver could not accept a connection");
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                    }
                }
            },
        }
    }
    drop(listener);
    let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(path)).await;
}

/// One connection: requests are read and answered until the client closes, a request
/// is refused, or a read takes longer than [`OTLP_READ_TIMEOUT`].
async fn serve(stream: TcpStream, shared: &Arc<Shared>, state: &ConnState) {
    let mut conn = Conn::new(stream);
    loop {
        let read = tokio::time::timeout(OTLP_READ_TIMEOUT, conn.request()).await;
        let request = match read {
            Err(_) | Ok(Err(Unread::Gone)) => return,
            Ok(Err(Unread::Refuse(status))) => {
                conn.refuse(status).await;
                return;
            }
            Ok(Ok(request)) => request,
        };
        let keep_alive = request.keep_alive;
        let status = answer(request, shared, state).await;
        let keep = status == 200 && keep_alive;
        if !keep {
            conn.refuse(status).await;
            return;
        }
        if conn.keep(status).await.is_err() {
            return;
        }
    }
}

/// The status for a request read in full; for `200`, its points are in the ledger and
/// the orchestrator totals they touched are posted.
async fn answer(request: Request, shared: &Arc<Shared>, state: &ConnState) -> u16 {
    if request.path.split('?').next() != Some("/v1/metrics") {
        return 404;
    }
    if request.method != "POST" {
        return 405;
    }
    let media = request.content_type.split(';').next().unwrap_or("").trim();
    if !media.eq_ignore_ascii_case("application/json") {
        if !shared.logged_type.swap(true, Ordering::Relaxed) {
            let shown: String = media.chars().take(64).collect();
            tracing::warn!(content_type = %shown, "the OTLP receiver takes only application/json (OTEL_EXPORTER_OTLP_PROTOCOL=http/json); answered 415");
        }
        return 415;
    }
    let body = request.body;
    let presented = request.authorization;
    let metering = shared.clone();
    let metered = tokio::task::spawn_blocking(move || {
        parse_metrics(&body).map(|points| {
            let (points, tokened) = authorize(&metering, points, presented.as_deref());
            record(&metering, points);
            tokened
        })
    })
    .await;
    let Ok(Ok(tokened)) = metered else {
        return 400;
    };
    if tokened {
        state.tokened.store(true, Ordering::SeqCst);
    }
    200
}

/// On a blocking thread, all under the ledger lock (re-review Important 1): evicts the
/// runs that went since the last request, drops the points of runs that are not live,
/// applies the rest, and posts the new orchestrator total of every run they touched to
/// [`UsageSink::post`]. Returns what it posted.
///
/// Holding the lock from the generation read to the apply means no other request can
/// evict a run between this one's live check and its apply. A run evicted as not live
/// never passes the filter again, since a run that ended never becomes live again, so
/// its total is never rebuilt from a later request's points alone.
///
/// Posting under the lock too (fix round 2) means totals reach the sink in the order
/// the ledger computed them, so the latest post, which the sink keeps, is the highest.
/// Lock order: `ledger` → the sink's `live` (in `is_live`) and `ledger` → its pending
/// map (in `post`); the sink never takes the ledger, so no cycle is possible.
fn record(shared: &Shared, mut points: Vec<UsagePoint>) -> Vec<(String, TokenUsage)> {
    let sink = &shared.sink;
    let mut held = crate::lock(&shared.ledger);
    let generation = sink.live_generation();
    if held.generation != Some(generation) {
        held.ledger.retain_runs(|run| sink.is_live(run));
        held.generation = Some(generation);
    }
    let live: BTreeSet<String> = points
        .iter()
        .map(|p| p.run_id.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|run| sink.is_live(run))
        .map(str::to_string)
        .collect();
    points.retain(|p| live.contains(&p.run_id));
    let touched = held.ledger.apply(&points);
    let totals: Vec<(String, TokenUsage)> = touched
        .into_iter()
        .filter(|(_, role)| role == ORCHESTRATOR)
        .map(|(run, role)| {
            let total = held.ledger.total(&run, &role);
            (run, total)
        })
        .collect();
    for (run_id, usage) in &totals {
        sink.post(run_id.clone(), *usage);
    }
    drop(held);
    totals
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
