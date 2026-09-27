//! The OTLP/HTTP-JSON receiver (M8b decision 30). I/O.
//!
//! A minimal HTTP/1.1 server on `127.0.0.1` for one route, `POST /v1/metrics`, with no
//! HTTP crate. It never blocks the daemon and never trusts its input:
//!
//! - every connection is its own task, and at most [`OTLP_MAX_CONNECTIONS`] are served
//!   at once (a connection beyond that is closed at once), so one slow client never
//!   delays another and memory stays bounded;
//! - each request is read within [`OTLP_READ_TIMEOUT`], headers up to
//!   [`OTLP_MAX_HEADERS`] and a `Content-Length` or chunked body up to
//!   [`OTLP_MAX_BODY`]; anything larger is `413` and the connection is closed;
//! - the body is parsed on `spawn_blocking`, and the ledger's lock is held only to apply
//!   the points;
//! - it makes no outbound call and never logs a body (ruling R-T1-5): exports carry the
//!   user's account identity.
//!
//! Answers: `200 {}`; `400` for a malformed request or body; `404` for another path;
//! `405` for another method; `415` for another content type (logged once per daemon);
//! `413` for oversize. After each accepted request, the ledger's new total for every
//! `(run, "orchestrator")` it touched goes to the sink.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::TokenUsage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::otlp::{OtlpLedger, parse_metrics};

pub const OTLP_MAX_BODY: usize = 4 << 20;
pub const OTLP_MAX_HEADERS: usize = 16 << 10;
pub const OTLP_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections served at once. Bounds memory at about this many bodies.
pub const OTLP_MAX_CONNECTIONS: usize = 8;
/// `<data_dir>/otlp.addr`: `http://127.0.0.1:<port>`, removed at shutdown.
pub const ADDR_FILE: &str = "otlp.addr";
/// The role whose totals reach the engine; any other is kept in the ledger only.
const ORCHESTRATOR: &str = "orchestrator";
/// The longest chunk-size line accepted.
const MAX_CHUNK_LINE: usize = 1024;
/// How long a refused connection is drained before it closes, so the client can read
/// the answer rather than a reset.
const LINGER: Duration = Duration::from_millis(500);

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

struct Shared {
    ledger: Mutex<OtlpLedger>,
    sink: UnboundedSender<(String, TokenUsage)>,
    /// `415` is logged once per daemon.
    logged_type: AtomicBool,
}

/// Binds `127.0.0.1:<port>` (0 picks a free port), writes [`ADDR_FILE`] in `data_dir`,
/// and serves until `shutdown` is cancelled.
pub async fn bind(
    port: u16,
    data_dir: &Path,
    sink: UnboundedSender<(String, TokenUsage)>,
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
        ledger: Mutex::new(OtlpLedger::default()),
        sink,
        logged_type: AtomicBool::new(false),
    });
    let task = tokio::spawn(accept_loop(listener, shared, shutdown, path));
    Ok(OtlpServer { addr, task })
}

/// The daemon's wiring (`lifecycle::run`): with `metering.otlp`, binds the receiver and
/// forwards every orchestrator total to `runs`. A bind failure is logged and metering is
/// off for this daemon; it never stops the daemon. With `otlp` off, a stale address
/// file from an earlier daemon is removed.
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
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let server = match bind(metering.otlp_port, data_dir, tx, shutdown).await {
        Ok(server) => server,
        Err(error) => {
            tracing::error!(%error, port = metering.otlp_port, "the OTLP receiver could not bind; orchestrator metering is off");
            return None;
        }
    };
    tracing::info!(addr = %server.addr, "OTLP receiver listening");
    tokio::spawn(async move {
        while let Some((run_id, usage)) = rx.recv().await {
            runs.orchestrator_usage(run_id, usage);
        }
    });
    Some(server)
}

fn write_addr(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("addr.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

async fn accept_loop(
    listener: TcpListener,
    shared: Arc<Shared>,
    shutdown: CancellationToken,
    path: PathBuf,
) {
    let slots = Arc::new(Semaphore::new(OTLP_MAX_CONNECTIONS));
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    // Over the cap the connection is dropped, which closes it.
                    let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
                    let shared = shared.clone();
                    let stop = shutdown.clone();
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = stop.cancelled() => {}
                            _ = serve(stream, &shared) => {}
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

/// A request that was read in full.
struct Request {
    method: String,
    path: String,
    content_type: String,
    body: Vec<u8>,
    keep_alive: bool,
}

/// Why a request was not read.
enum Unread {
    /// The client closed, the read failed, or it ran out of time: close silently.
    Gone,
    /// Answer this status, then close.
    Refuse(u16),
}

/// One connection: requests are read and answered until the client closes, a request
/// is refused, or a read takes longer than [`OTLP_READ_TIMEOUT`].
async fn serve(stream: TcpStream, shared: &Shared) {
    let mut conn = Conn {
        stream,
        buf: Vec::new(),
    };
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
        let status = answer(request, shared).await;
        let keep = status == 200 && keep_alive;
        if !keep {
            conn.refuse(status).await;
            return;
        }
        if conn.respond(status, true).await.is_err() {
            return;
        }
    }
}

/// The status for a request read in full; for `200`, its points are in the ledger and
/// the orchestrator totals they touched are sent.
async fn answer(request: Request, shared: &Shared) -> u16 {
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
    let Ok(Ok(points)) = tokio::task::spawn_blocking(move || parse_metrics(&body)).await else {
        return 400;
    };
    let totals: Vec<(String, TokenUsage)> = {
        let mut ledger = crate::lock(&shared.ledger);
        ledger
            .apply(&points)
            .into_iter()
            .filter(|(_, role)| role == ORCHESTRATOR)
            .map(|(run, role)| {
                let total = ledger.total(&run, &role);
                (run, total)
            })
            .collect()
    };
    for total in totals {
        let _ = shared.sink.send(total);
    }
    200
}

/// A connection and the bytes read past the last request.
struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Conn {
    /// Reads more into the buffer; `Gone` at end of stream or on an error.
    async fn fill(&mut self) -> Result<(), Unread> {
        let mut chunk = [0u8; 16 << 10];
        match self.stream.read(&mut chunk).await {
            Ok(0) | Err(_) => Err(Unread::Gone),
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                Ok(())
            }
        }
    }

    async fn request(&mut self) -> Result<Request, Unread> {
        let head_end = loop {
            if let Some(i) = find(&self.buf, b"\r\n\r\n") {
                if i + 4 > OTLP_MAX_HEADERS {
                    return Err(Unread::Refuse(413));
                }
                break i + 4;
            }
            if self.buf.len() > OTLP_MAX_HEADERS {
                return Err(Unread::Refuse(413));
            }
            self.fill().await?;
        };
        let head: Vec<u8> = self.buf.drain(..head_end).collect();
        let head = std::str::from_utf8(&head).map_err(|_| Unread::Refuse(400))?;
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let (Some(method), Some(path), Some(version), None) =
            (first.next(), first.next(), first.next(), first.next())
        else {
            return Err(Unread::Refuse(400));
        };
        if !version.starts_with("HTTP/1.") {
            return Err(Unread::Refuse(400));
        }
        let mut content_type = String::new();
        let mut length: Option<usize> = None;
        let mut chunked = false;
        let mut keep_alive = version == "HTTP/1.1";
        for line in lines.filter(|l| !l.is_empty()) {
            let (name, value) = line.split_once(':').ok_or(Unread::Refuse(400))?;
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-type" => content_type = value.to_string(),
                "content-length" => {
                    let n: usize = value.parse().map_err(|_| Unread::Refuse(400))?;
                    if length.is_some_and(|l| l != n) {
                        return Err(Unread::Refuse(400));
                    }
                    length = Some(n);
                }
                "transfer-encoding" => {
                    if !value.eq_ignore_ascii_case("chunked") {
                        return Err(Unread::Refuse(400));
                    }
                    chunked = true;
                }
                "connection" => {
                    let value = value.to_ascii_lowercase();
                    if value.contains("close") {
                        keep_alive = false;
                    } else if value.contains("keep-alive") {
                        keep_alive = true;
                    }
                }
                _ => {}
            }
        }
        let body = match (chunked, length) {
            (true, Some(_)) => return Err(Unread::Refuse(400)),
            (true, None) => self.chunked_body().await?,
            (false, Some(n)) if n > OTLP_MAX_BODY => return Err(Unread::Refuse(413)),
            (false, n) => self.exact(n.unwrap_or(0)).await?,
        };
        Ok(Request {
            method: method.to_string(),
            path: path.to_string(),
            content_type,
            body,
            keep_alive,
        })
    }

    /// The next `n` bytes (`n` is at most [`OTLP_MAX_BODY`]).
    async fn exact(&mut self, n: usize) -> Result<Vec<u8>, Unread> {
        while self.buf.len() < n {
            self.fill().await?;
        }
        Ok(self.buf.drain(..n).collect())
    }

    /// One line, without its CRLF, of at most `max` bytes.
    async fn line(&mut self, max: usize) -> Result<Vec<u8>, Unread> {
        loop {
            if let Some(i) = find(&self.buf, b"\r\n") {
                if i > max {
                    return Err(Unread::Refuse(400));
                }
                let line = self.buf.drain(..i + 2).take(i).collect();
                return Ok(line);
            }
            if self.buf.len() > max {
                return Err(Unread::Refuse(400));
            }
            self.fill().await?;
        }
    }

    /// A chunked body, its total held to [`OTLP_MAX_BODY`]; trailers are read and
    /// dropped, held to [`OTLP_MAX_HEADERS`].
    async fn chunked_body(&mut self) -> Result<Vec<u8>, Unread> {
        let mut body = Vec::new();
        loop {
            let line = self.line(MAX_CHUNK_LINE).await?;
            let line = std::str::from_utf8(&line).map_err(|_| Unread::Refuse(400))?;
            let size = line.split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size, 16).map_err(|_| Unread::Refuse(400))?;
            if size == 0 {
                break;
            }
            if size > OTLP_MAX_BODY - body.len() {
                return Err(Unread::Refuse(413));
            }
            let chunk = self.exact(size).await?;
            body.extend_from_slice(&chunk);
            if !self.line(0).await?.is_empty() {
                return Err(Unread::Refuse(400));
            }
        }
        let mut trailers = 0;
        loop {
            let line = self.line(OTLP_MAX_HEADERS).await?;
            trailers += line.len() + 2;
            if trailers > OTLP_MAX_HEADERS {
                return Err(Unread::Refuse(413));
            }
            if line.is_empty() {
                return Ok(body);
            }
        }
    }

    async fn respond(&mut self, status: u16, keep_alive: bool) -> std::io::Result<()> {
        let reason = match status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            405 => "Method Not Allowed",
            413 => "Payload Too Large",
            415 => "Unsupported Media Type",
            _ => "Error",
        };
        let connection = if keep_alive { "keep-alive" } else { "close" };
        let text = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: {connection}\r\n\r\n{{}}"
        );
        let write = self.stream.write_all(text.as_bytes());
        match tokio::time::timeout(OTLP_READ_TIMEOUT, write).await {
            Ok(result) => result,
            Err(elapsed) => Err(std::io::Error::other(elapsed)),
        }
    }

    /// Answers `status` and closes: the write side is shut, and what the client still
    /// sends is read and dropped for at most [`LINGER`], so it sees the answer rather
    /// than a reset.
    async fn refuse(mut self, status: u16) {
        if self.respond(status, false).await.is_err() {
            return;
        }
        let _ = self.stream.shutdown().await;
        let drain = async {
            let mut sink = [0u8; 16 << 10];
            while matches!(self.stream.read(&mut sink).await, Ok(n) if n > 0) {}
        };
        let _ = tokio::time::timeout(LINGER, drain).await;
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
