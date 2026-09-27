//! M8b.15: the OTLP/HTTP-JSON receiver over real TCP on `127.0.0.1:0` (decision 30).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use daemon::metering::server::{
    ADDR_FILE, OTLP_MAX_BODY, OTLP_MAX_CONNECTIONS, OTLP_MAX_HEADERS, OTLP_READ_TIMEOUT,
    OTLP_SLOT_WAIT,
};
use daemon::metering::{OtlpServer, UsageSink, bind};
use proto::TokenUsage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

/// M8b.1 item 5's recorded body: run `r-fix`, role `orchestrator`.
const FIXTURE: &[u8] = include_bytes!("fixtures/otlp/claude-2.1.280-metrics.json");

/// Bounds every wait in this file except the slow-client test's. Well above
/// `OTLP_READ_TIMEOUT`, the longest the server may legitimately take to answer or
/// close anything.
fn wait() -> Duration {
    OTLP_READ_TIMEOUT + Duration::from_secs(20)
}

/// The slow-client bound (the brief's 2 s). It must stay under `OTLP_READ_TIMEOUT`: a
/// server that served connections one at a time would answer the second request only
/// after the stalled one timed out, so any bound at or above that would prove nothing.
/// Measured in `docs/timing-budgets.md` (the `a_slow_client_does_not_block_another` row).
const SLOW_BOUND: Duration = Duration::from_secs(2);

fn fixture_usage() -> TokenUsage {
    TokenUsage {
        input: 10,
        output: 170,
        cache_read: 13_689,
        cache_write: 12_503,
    }
}

/// The run service's stand-in: a settable set of live runs, and every post recorded.
struct TestSink {
    live: Mutex<HashSet<String>>,
    generation: AtomicU64,
    posts: UnboundedSender<(String, TokenUsage)>,
}

impl TestSink {
    fn set_live(&self, runs: &[&str]) {
        *self.live.lock().unwrap() = runs.iter().map(|r| r.to_string()).collect();
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

impl UsageSink for TestSink {
    fn is_live(&self, run_id: &str) -> bool {
        self.live.lock().unwrap().contains(run_id)
    }

    fn live_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn post(&self, run_id: String, usage: TokenUsage) {
        let _ = self.posts.send((run_id, usage));
    }
}

struct Receiver {
    _dir: tempfile::TempDir,
    server: OtlpServer,
    control: Arc<TestSink>,
    sink: UnboundedReceiver<(String, TokenUsage)>,
    shutdown: CancellationToken,
}

/// A receiver whose only live run is the fixture's, `r-fix`.
async fn start() -> Receiver {
    let dir = tempfile::tempdir().unwrap();
    let (posts, sink) = unbounded_channel();
    let control = Arc::new(TestSink {
        live: Mutex::new(HashSet::from(["r-fix".to_string()])),
        generation: AtomicU64::new(0),
        posts,
    });
    let shutdown = CancellationToken::new();
    let server = bind(0, dir.path(), control.clone(), shutdown.clone())
        .await
        .expect("the receiver binds");
    Receiver {
        _dir: dir,
        server,
        control,
        sink,
        shutdown,
    }
}

fn request(path: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn chunked(body: &[u8], size: usize) -> Vec<u8> {
    let mut out = b"POST /v1/metrics HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for chunk in body.chunks(size) {
        out.extend_from_slice(format!("{:x};ext=1\r\n", chunk.len()).as_bytes());
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"0\r\nX-Trailer: 1\r\n\r\n");
    out
}

/// Reads one response: `Some((status, body))`, or `None` when the server closed the
/// connection first.
async fn response(stream: &mut TcpStream) -> Option<(u16, String)> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match stream.read(&mut byte).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => buf.extend_from_slice(&byte[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let status: u16 = head.split(' ').nth(1)?.parse().ok()?;
    let length: usize = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < length {
        match stream.read(&mut byte).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => body.extend_from_slice(&byte[..n]),
        }
    }
    Some((status, String::from_utf8_lossy(&body).to_string()))
}

async fn send(addr: SocketAddr, bytes: &[u8]) -> (TcpStream, Option<(u16, String)>) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    // A refused request may be answered before the whole body is written.
    let _ = stream.write_all(bytes).await;
    let answer = tokio::time::timeout(wait(), response(&mut stream))
        .await
        .expect("answered or closed in time");
    (stream, answer)
}

async fn next_total(sink: &mut UnboundedReceiver<(String, TokenUsage)>) -> (String, TokenUsage) {
    tokio::time::timeout(wait(), sink.recv())
        .await
        .expect("a total within the deadline")
        .expect("the sink is open")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_with_content_length_is_accepted_and_totals_reach_the_sink() {
    let mut rx = start().await;
    let (mut stream, answer) = send(
        rx.server.addr,
        &request("/v1/metrics", "application/json", FIXTURE),
    )
    .await;
    assert_eq!(answer, Some((200, "{}".to_string())));
    assert_eq!(
        next_total(&mut rx.sink).await,
        ("r-fix".into(), fixture_usage())
    );
    // The same connection is kept alive, and the delta points add.
    stream
        .write_all(&request(
            "/v1/metrics",
            "application/json; charset=utf-8",
            FIXTURE,
        ))
        .await
        .unwrap();
    let again = tokio::time::timeout(wait(), response(&mut stream))
        .await
        .unwrap();
    assert_eq!(again, Some((200, "{}".to_string())));
    let doubled = fixture_usage();
    let doubled = TokenUsage {
        input: doubled.input * 2,
        output: doubled.output * 2,
        cache_read: doubled.cache_read * 2,
        cache_write: doubled.cache_write * 2,
    };
    assert_eq!(next_total(&mut rx.sink).await, ("r-fix".into(), doubled));
    // A body with no point for a run sends nothing.
    let (_, empty) = send(
        rx.server.addr,
        &request("/v1/metrics", "application/json", b"{}"),
    )
    .await;
    assert_eq!(empty, Some((200, "{}".to_string())));
    assert!(rx.sink.try_recv().is_err());
    rx.shutdown.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chunked_post_is_accepted() {
    let mut rx = start().await;
    let (_, answer) = send(rx.server.addr, &chunked(FIXTURE, 1000)).await;
    assert_eq!(answer, Some((200, "{}".to_string())));
    assert_eq!(
        next_total(&mut rx.sink).await,
        ("r-fix".into(), fixture_usage())
    );
    rx.shutdown.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_path_is_404_and_protobuf_is_415() {
    let mut rx = start().await;
    let addr = rx.server.addr;
    let (_, answer) = send(addr, &request("/v1/traces", "application/json", FIXTURE)).await;
    assert_eq!(answer.map(|a| a.0), Some(404));
    let (_, answer) = send(
        addr,
        &request("/v1/metrics", "application/x-protobuf", FIXTURE),
    )
    .await;
    assert_eq!(answer.map(|a| a.0), Some(415));
    let (_, answer) = send(
        addr,
        &request("/v1/metrics", "application/json", b"{\"resourceMetrics\":"),
    )
    .await;
    assert_eq!(answer.map(|a| a.0), Some(400), "malformed JSON");
    let (_, answer) = send(addr, b"GET /v1/metrics HTTP/1.1\r\nHost: x\r\n\r\n").await;
    assert_eq!(answer.map(|a| a.0), Some(405));
    let (_, answer) = send(addr, b"garbage\r\n\r\n").await;
    assert!(matches!(answer.map(|a| a.0), None | Some(400)));
    assert!(
        rx.sink.try_recv().is_err(),
        "nothing refused reaches the sink"
    );
    rx.shutdown.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversized_body_is_refused() {
    let mut rx = start().await;
    let addr = rx.server.addr;
    // Review I2: each oversize case is answered `413`, never left to the read timeout.
    // A declared length over the cap is refused before any of it is read.
    let head = format!(
        "POST /v1/metrics HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        OTLP_MAX_BODY + 1
    );
    let (_, answer) = send(addr, head.as_bytes()).await;
    assert_eq!(answer.map(|a| a.0), Some(413));
    // A chunked body that grows past the cap.
    let big = vec![b' '; OTLP_MAX_BODY + 1];
    let (_, answer) = send(addr, &chunked(&big, 1 << 20)).await;
    assert_eq!(answer.map(|a| a.0), Some(413));
    // A chunk size that does not fit a number.
    let huge_chunk = b"POST /v1/metrics HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffffffffffff\r\n";
    let (_, answer) = send(addr, huge_chunk).await;
    assert!(matches!(answer.map(|a| a.0), None | Some(400) | Some(413)));
    // Headers over their cap.
    let mut long = b"POST /v1/metrics HTTP/1.1\r\nX-Pad: ".to_vec();
    long.extend(std::iter::repeat_n(b'a', OTLP_MAX_HEADERS + 1));
    let (_, answer) = send(addr, &long).await;
    assert_eq!(answer.map(|a| a.0), Some(413));
    // The receiver still serves.
    let (_, answer) = send(addr, &request("/v1/metrics", "application/json", FIXTURE)).await;
    assert_eq!(answer, Some((200, "{}".to_string())));
    assert_eq!(
        next_total(&mut rx.sink).await,
        ("r-fix".into(), fixture_usage())
    );
    rx.shutdown.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_client_does_not_block_another() {
    const { assert!(SLOW_BOUND.as_secs() < OTLP_READ_TIMEOUT.as_secs()) };
    let mut rx = start().await;
    let addr = rx.server.addr;
    let mut stalled = TcpStream::connect(addr).await.unwrap();
    stalled
        .write_all(b"POST /v1/metrics HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Ty")
        .await
        .unwrap();
    let started = Instant::now();
    let mut second = TcpStream::connect(addr).await.unwrap();
    second
        .write_all(&request("/v1/metrics", "application/json", FIXTURE))
        .await
        .unwrap();
    let answer = tokio::time::timeout(SLOW_BOUND, response(&mut second))
        .await
        .expect("the second client is answered while the first stalls");
    let elapsed = started.elapsed();
    assert_eq!(answer, Some((200, "{}".to_string())));
    eprintln!("second client answered in {elapsed:?}");
    assert_eq!(next_total(&mut rx.sink).await.1, fixture_usage());
    // The stalled client is closed after `OTLP_READ_TIMEOUT`, not served.
    let closed = tokio::time::timeout(wait(), response(&mut stalled)).await;
    assert!(
        matches!(closed, Ok(None) | Ok(Some((408, _)))),
        "{closed:?}"
    );
    rx.shutdown.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_address_file_is_written_and_removed_at_shutdown() {
    let rx = start().await;
    let path = rx._dir.path().join(ADDR_FILE);
    let addr = rx.server.addr;
    assert!(addr.ip().is_loopback() && addr.is_ipv4(), "{addr}");
    assert_ne!(addr.port(), 0);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("http://127.0.0.1:{}", addr.port())
    );
    // Review minor 6: readable by the user only.
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    rx.shutdown.cancel();
    tokio::time::timeout(wait(), rx.server.stopped())
        .await
        .expect("the receiver stops");
    assert!(!Path::new(&path).exists(), "otlp.addr is removed");
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "the port is closed"
    );
}

/// An export with one resource per `(run, role)`, each with `points` delta `input`
/// points of 1 token.
fn export(resources: &[(String, &str)], points: usize) -> Vec<u8> {
    let attr = |k: &str, v: &str| serde_json::json!({"key": k, "value": {"stringValue": v}});
    let data: Vec<_> = (0..points)
        .map(|i| {
            serde_json::json!({
                "attributes": [attr("type", "input"), attr("session.id", &format!("s{i}"))],
                "asDouble": 1,
            })
        })
        .collect();
    let resources: Vec<_> = resources
        .iter()
        .map(|(run, role)| {
            serde_json::json!({
                "resource": {"attributes": [attr("anthrex.run", run), attr("anthrex.role", role)]},
                "scopeMetrics": [{"metrics": [{"name": "claude_code.token.usage",
                    "sum": {"aggregationTemporality": 1, "dataPoints": data}}]}],
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({ "resourceMetrics": resources })).unwrap()
}

fn input(n: u64) -> TokenUsage {
    TokenUsage {
        input: n,
        ..TokenUsage::default()
    }
}

/// Review I1: the CRLF after a chunk's data may arrive split across two reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chunk_whose_crlf_is_split_across_reads_is_accepted() {
    let rx = start().await;
    let mut stream = TcpStream::connect(rx.server.addr).await.unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .write_all(b"POST /v1/metrics HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r")
        .await
        .unwrap();
    stream.flush().await.unwrap();
    // Only so the two writes reach the server as two reads; the answer is the check.
    tokio::time::sleep(Duration::from_millis(200)).await;
    stream.write_all(b"\n0\r\n\r\n").await.unwrap();
    let answer = tokio::time::timeout(wait(), response(&mut stream))
        .await
        .unwrap();
    assert_eq!(answer, Some((200, "{}".to_string())));
    rx.shutdown.cancel();
}

/// Review I3: only runs the daemon has are metered. 1100 junk run ids, as workers and
/// as orchestrators, reach neither the sink nor the ledger's caps, and a real run is
/// still metered after them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn junk_run_ids_never_crowd_out_a_live_run() {
    let mut rx = start().await;
    let addr = rx.server.addr;
    for role in ["worker", "orchestrator"] {
        let junk: Vec<(String, &str)> = (0..1100).map(|i| (format!("r-junk-{i}"), role)).collect();
        let (_, answer) = send(
            addr,
            &request("/v1/metrics", "application/json", &export(&junk, 1)),
        )
        .await;
        assert_eq!(answer.map(|a| a.0), Some(200));
    }
    let (_, answer) = send(addr, &request("/v1/metrics", "application/json", FIXTURE)).await;
    assert_eq!(answer.map(|a| a.0), Some(200));
    assert_eq!(
        next_total(&mut rx.sink).await,
        ("r-fix".into(), fixture_usage()),
        "no junk total reached the sink, and the live run was metered"
    );
    assert!(rx.sink.try_recv().is_err());
    rx.shutdown.cancel();
}

/// Review I3: a run that is no longer live is evicted; its points are dropped, and its
/// totals are gone if it ever came back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_that_ends_is_evicted() {
    let mut rx = start().await;
    let addr = rx.server.addr;
    let body = export(&[("r-fix".to_string(), "orchestrator")], 1);
    send(addr, &request("/v1/metrics", "application/json", &body)).await;
    send(addr, &request("/v1/metrics", "application/json", &body)).await;
    assert_eq!(next_total(&mut rx.sink).await.1, input(1));
    assert_eq!(next_total(&mut rx.sink).await.1, input(2));
    rx.control.set_live(&[]);
    let (_, answer) = send(addr, &request("/v1/metrics", "application/json", &body)).await;
    assert_eq!(answer.map(|a| a.0), Some(200));
    assert!(rx.sink.try_recv().is_err(), "an ended run reaches no sink");
    rx.control.set_live(&["r-fix"]);
    send(addr, &request("/v1/metrics", "application/json", &body)).await;
    assert_eq!(
        next_total(&mut rx.sink).await.1,
        input(1),
        "evicted, so it starts over"
    );
    rx.shutdown.cancel();
}

/// Review I4: one POST touching one run with many points posts one total.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_post_with_many_points_posts_one_total_per_run() {
    let mut rx = start().await;
    let body = export(&[("r-fix".to_string(), "orchestrator")], 500);
    let (_, answer) = send(
        rx.server.addr,
        &request("/v1/metrics", "application/json", &body),
    )
    .await;
    assert_eq!(answer.map(|a| a.0), Some(200));
    assert_eq!(next_total(&mut rx.sink).await, ("r-fix".into(), input(500)));
    assert!(rx.sink.try_recv().is_err(), "one total, not one per point");
    rx.shutdown.cancel();
}

/// Review minor 3: another role of a live run is kept in the ledger, never posted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_role_reaches_no_sink() {
    let mut rx = start().await;
    let body = export(&[("r-fix".to_string(), "worker")], 3);
    let (_, answer) = send(
        rx.server.addr,
        &request("/v1/metrics", "application/json", &body),
    )
    .await;
    assert_eq!(answer.map(|a| a.0), Some(200));
    let (_, answer) = send(
        rx.server.addr,
        &request("/v1/metrics", "application/json", b"{}"),
    )
    .await;
    assert_eq!(answer.map(|a| a.0), Some(200));
    assert!(rx.sink.try_recv().is_err());
    rx.shutdown.cancel();
}

/// Opens `OTLP_MAX_CONNECTIONS` connections that send nothing, each holding a slot.
/// Never more than 64, so a cap raised far past its value fails the test rather than
/// the machine's descriptor limit.
async fn hold_every_slot(addr: SocketAddr) -> Vec<TcpStream> {
    let mut held = Vec::new();
    for _ in 0..OTLP_MAX_CONNECTIONS.min(64) {
        held.push(TcpStream::connect(addr).await.unwrap());
    }
    held
}

/// Review minor 3 and the 9th-connection ruling: with every slot held, a further
/// connection waits `OTLP_SLOT_WAIT` for one, then is closed unanswered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn past_the_connection_cap_a_connection_is_closed_after_the_slot_wait() {
    const { assert!(OTLP_SLOT_WAIT.as_secs() < OTLP_READ_TIMEOUT.as_secs()) };
    let rx = start().await;
    let _held = hold_every_slot(rx.server.addr).await;
    let started = Instant::now();
    let (_, answer) = send(
        rx.server.addr,
        &request("/v1/metrics", "application/json", FIXTURE),
    )
    .await;
    assert_eq!(answer, None, "closed, not served");
    // Closed by the slot wait, well before any held connection's read timeout frees one.
    assert!(
        started.elapsed() < OTLP_READ_TIMEOUT,
        "{:?}",
        started.elapsed()
    );
    rx.shutdown.cancel();
}

/// The 9th-connection ruling: a slot freed during the wait serves the waiting one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_waiting_for_a_slot_is_served_when_one_frees() {
    let mut rx = start().await;
    let mut held = hold_every_slot(rx.server.addr).await;
    let mut waiting = TcpStream::connect(rx.server.addr).await.unwrap();
    waiting
        .write_all(&request("/v1/metrics", "application/json", FIXTURE))
        .await
        .unwrap();
    // Well inside the slot wait, so the connection is waiting when a slot frees; the
    // answer is the check (`docs/timing-budgets.md`, from M8b.15's fix round).
    tokio::time::sleep(OTLP_SLOT_WAIT / 10).await;
    drop(held.pop());
    let answer = tokio::time::timeout(wait(), response(&mut waiting))
        .await
        .unwrap();
    assert_eq!(answer, Some((200, "{}".to_string())));
    assert_eq!(next_total(&mut rx.sink).await.1, fixture_usage());
    rx.shutdown.cancel();
}
