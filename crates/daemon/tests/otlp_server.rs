//! M8b.15: the OTLP/HTTP-JSON receiver over real TCP on `127.0.0.1:0` (decision 30).

use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use daemon::metering::server::{ADDR_FILE, OTLP_MAX_BODY, OTLP_MAX_HEADERS, OTLP_READ_TIMEOUT};
use daemon::metering::{OtlpServer, bind};
use proto::TokenUsage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
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

struct Receiver {
    _dir: tempfile::TempDir,
    server: OtlpServer,
    sink: UnboundedReceiver<(String, TokenUsage)>,
    shutdown: CancellationToken,
}

async fn start() -> Receiver {
    let dir = tempfile::tempdir().unwrap();
    let (tx, sink) = unbounded_channel();
    let shutdown = CancellationToken::new();
    let server = bind(0, dir.path(), tx, shutdown.clone())
        .await
        .expect("the receiver binds");
    Receiver {
        _dir: dir,
        server,
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
    let refused = |answer: Option<(u16, String)>| matches!(answer.map(|a| a.0), None | Some(413));
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
    assert!(refused(answer.clone()), "{answer:?}");
    // A chunk size that does not fit a number.
    let huge_chunk = b"POST /v1/metrics HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffffffffffff\r\n";
    let (_, answer) = send(addr, huge_chunk).await;
    assert!(matches!(answer.map(|a| a.0), None | Some(400) | Some(413)));
    // Headers over their cap.
    let mut long = b"POST /v1/metrics HTTP/1.1\r\nX-Pad: ".to_vec();
    long.extend(std::iter::repeat_n(b'a', OTLP_MAX_HEADERS + 1));
    let (_, answer) = send(addr, &long).await;
    assert!(refused(answer.clone()), "{answer:?}");
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
