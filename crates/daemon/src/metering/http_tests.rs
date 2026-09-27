//! The OTLP receiver's HTTP reading (M8b.15 and its reviews).

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn consuming_moves_an_offset_and_settling_gives_memory_back() {
    let mut buf = ReadBuf::default();
    buf.extend(&vec![b'a'; 4 << 20]);
    buf.extend(b"next");
    let data = buf.data.as_ptr();
    buf.skip(1);
    assert_eq!(buf.take(3), b"aaa");
    // Consuming moved nothing.
    assert_eq!((buf.data.as_ptr(), buf.start), (data, 4));
    buf.skip((4 << 20) - 4);
    assert_eq!(buf.pending(), b"next");
    buf.settle();
    assert_eq!(buf.pending(), b"next");
    assert!(
        buf.data.capacity() <= KEEP_CAPACITY,
        "{}",
        buf.data.capacity()
    );
}

#[test]
fn a_fully_consumed_buffer_is_reused_and_a_long_prefix_is_dropped() {
    let mut buf = ReadBuf::default();
    buf.extend(b"abc");
    buf.skip(3);
    buf.extend(b"de");
    assert_eq!((buf.start, buf.data.as_slice()), (0, b"de".as_slice()));
    let mut buf = ReadBuf::default();
    buf.extend(&vec![b'x'; COMPACT_AT + 10]);
    buf.skip(COMPACT_AT);
    buf.extend(b"y");
    assert_eq!(buf.start, 0);
    assert_eq!(buf.pending(), b"xxxxxxxxxxy");
}

/// What reading one request gave: its body, or the status it was refused with (`None`
/// when the connection was simply closed).
type Read = Result<Vec<u8>, Option<u16>>;

fn outcome(read: Result<Request, Unread>) -> Read {
    match read {
        Ok(request) => Ok(request.body),
        Err(Unread::Refuse(status)) => Err(Some(status)),
        Err(Unread::Gone) => Err(None),
    }
}

/// A real loopback connection whose client sends `bytes` and then stays open, the
/// server's side of it, and the client (its stream once every byte is sent). The
/// client writes on its own task, so a body larger than the socket's buffers is sent
/// while the server reads it.
async fn connected(bytes: Vec<u8>) -> (Conn, tokio::task::JoinHandle<tokio::net::TcpStream>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::spawn(async move {
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client.write_all(&bytes).await.unwrap();
        client
    });
    let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("the client connects")
        .unwrap();
    (Conn::new(stream), client)
}

async fn read_one(bytes: &[u8]) -> Read {
    let (mut conn, client) = connected(bytes.to_vec()).await;
    let read = tokio::time::timeout(Duration::from_secs(5), conn.request())
        .await
        .expect("the request is read or refused at once");
    client.abort();
    outcome(read)
}

const POST: &str = "POST /v1/metrics HTTP/1.1\r\nContent-Type: application/json\r\n";

/// Review minor 2 (mutant H): two different lengths are refused; the same one twice is
/// not.
#[tokio::test]
async fn two_different_content_lengths_are_refused() {
    let two = format!("{POST}Content-Length: 3\r\nContent-Length: 2\r\n\r\n{{}}");
    assert_eq!(read_one(two.as_bytes()).await, Err(Some(400)));
    let same = format!("{POST}Content-Length: 2\r\nContent-Length: 2\r\n\r\n{{}}");
    assert_eq!(read_one(same.as_bytes()).await, Ok(b"{}".to_vec()));
}

/// Review minor 2 (mutant I): a length together with chunked is refused, never read
/// as either.
#[tokio::test]
async fn a_length_together_with_chunked_is_refused() {
    let both = format!(
        "{POST}Content-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{{}}\r\n0\r\n\r\n"
    );
    assert_eq!(read_one(both.as_bytes()).await, Err(Some(400)));
}

/// Review minor 2 (mutant J): trailers are capped in total, not only line by line.
#[tokio::test]
async fn trailers_past_the_header_cap_in_total_are_refused() {
    let line = format!("X-Trailer: {}\r\n", "a".repeat(1000));
    let within = format!("{POST}Transfer-Encoding: chunked\r\n\r\n2\r\n{{}}\r\n0\r\n{line}\r\n");
    assert_eq!(read_one(within.as_bytes()).await, Ok(b"{}".to_vec()));
    let many = line.repeat(OTLP_MAX_HEADERS / line.len() + 1);
    assert!(many.lines().all(|l| l.len() < OTLP_MAX_HEADERS));
    let past = format!("{POST}Transfer-Encoding: chunked\r\n\r\n2\r\n{{}}\r\n0\r\n{many}\r\n");
    assert_eq!(read_one(past.as_bytes()).await, Err(Some(413)));
}

/// Review minor 2 (mutant P): only HTTP/1.x is read.
#[tokio::test]
async fn another_protocol_version_is_refused() {
    for version in ["SPDY/3", "HTTP/2.0", "http/1.1"] {
        let request = format!("POST /v1/metrics {version}\r\nContent-Length: 2\r\n\r\n{{}}");
        assert_eq!(
            read_one(request.as_bytes()).await,
            Err(Some(400)),
            "{version}"
        );
    }
    let old = "POST /v1/metrics HTTP/1.0\r\nContent-Length: 2\r\n\r\n{}";
    assert_eq!(read_one(old.as_bytes()).await, Ok(b"{}".to_vec()));
}

/// Review minor 2 (mutant F): a kept-alive answer gives back what a large request's
/// buffer grew to, and the next request is read from where the last one ended.
#[tokio::test]
async fn a_kept_alive_answer_settles_the_buffer() {
    let body = format!("{{\"pad\":\"{}\"}}", "a".repeat(1 << 20));
    let mut bytes = format!("{POST}Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes();
    bytes.extend(format!("{POST}Content-Length: 2\r\n\r\n{{}}").as_bytes());
    let (mut conn, client) = connected(bytes).await;
    let first = tokio::time::timeout(Duration::from_secs(5), conn.request())
        .await
        .unwrap();
    let mut client = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("the client sent every byte")
        .unwrap();
    assert_eq!(outcome(first).map(|b| b.len()), Ok(body.len()));
    conn.keep(200).await.unwrap();
    assert!(
        conn.buf.data.capacity() <= KEEP_CAPACITY,
        "a kept-alive connection holds {} bytes",
        conn.buf.data.capacity()
    );
    let mut answer = [0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(5), client.read(&mut answer))
        .await
        .expect("the answer arrives")
        .unwrap();
    let answer = String::from_utf8_lossy(&answer[..n]);
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    assert!(answer.contains("Connection: keep-alive"), "{answer}");
    let second = tokio::time::timeout(Duration::from_secs(5), conn.request())
        .await
        .unwrap();
    assert_eq!(outcome(second), Ok(b"{}".to_vec()));
}

/// A generous bound on scanning a head, and a trailer line, that each arrive a byte at
/// a time (`docs/timing-budgets.md`). The linear scan takes about 6 ms; the
/// quadratic one it replaced took seconds in a debug build.
const DRIP_BOUND: Duration = Duration::from_millis(500);

/// Review minor 3: each read resumes the search where the last one stopped, so a head
/// or a trailer dripped a byte per read costs linear time, not quadratic.
#[test]
fn a_head_or_a_trailer_arriving_a_byte_at_a_time_is_scanned_in_linear_time() {
    let mut head = POST.as_bytes().to_vec();
    let pad = format!("X-Pad: {}\r\n", "a".repeat(58));
    while head.len() + pad.len() + 2 <= OTLP_MAX_HEADERS {
        head.extend(pad.as_bytes());
    }
    head.extend(b"\r\n");
    assert!(head.len() > OTLP_MAX_HEADERS - 128, "{}", head.len());
    let trailer = [vec![b'a'; OTLP_MAX_HEADERS], b"\r\n".to_vec()].concat();
    let start = std::time::Instant::now();
    let (mut buf, mut scanned, mut end) = (ReadBuf::default(), 0, None);
    for byte in &head {
        buf.extend(&[*byte]);
        end = buf.head_end(&mut scanned).ok().flatten();
        if end.is_some() {
            break;
        }
    }
    assert_eq!(end, Some(head.len()));
    let (mut buf, mut scanned, mut end) = (ReadBuf::default(), 0, None);
    for byte in &trailer {
        buf.extend(&[*byte]);
        end = buf.line_end(OTLP_MAX_HEADERS, &mut scanned).ok().flatten();
        if end.is_some() {
            break;
        }
    }
    assert_eq!(end, Some(OTLP_MAX_HEADERS));
    let elapsed = start.elapsed();
    eprintln!("scanning the dripped head and trailer took {elapsed:?}");
    assert!(elapsed < DRIP_BOUND, "{elapsed:?}");
}

/// The resumed search still finds a terminator split across reads, and still refuses
/// what is too long.
#[test]
fn a_resumed_scan_finds_a_split_terminator_and_keeps_the_caps() {
    let (mut buf, mut scanned) = (ReadBuf::default(), 0);
    for part in [&b"GET / HTTP/1.1\r"[..], b"\n\r", b"\n"] {
        assert_eq!(buf.head_end(&mut scanned).ok().flatten(), None);
        buf.extend(part);
    }
    assert_eq!(buf.head_end(&mut scanned).ok().flatten(), Some(18));
    let (mut buf, mut scanned) = (ReadBuf::default(), 0);
    buf.extend(b"abc\r");
    assert_eq!(buf.line_end(3, &mut scanned).ok().flatten(), None);
    buf.extend(b"\n");
    assert_eq!(buf.line_end(3, &mut scanned).ok().flatten(), Some(3));
    let (mut buf, mut scanned) = (ReadBuf::default(), 0);
    buf.extend(b"abcde");
    assert!(matches!(
        buf.line_end(3, &mut scanned),
        Err(Unread::Refuse(400))
    ));
    let (mut buf, mut scanned) = (ReadBuf::default(), 0);
    buf.extend(&vec![b'a'; OTLP_MAX_HEADERS + 1]);
    assert!(matches!(
        buf.head_end(&mut scanned),
        Err(Unread::Refuse(413))
    ));
}

/// M8b.15 re-review 2, m2: a line longer than its cap is refused even when it arrives
/// whole, CRLF included, in one read; only the first `max + 2` bytes are searched.
#[tokio::test]
async fn an_over_long_line_arriving_whole_in_one_read_is_refused() {
    let (mut buf, mut scanned) = (ReadBuf::default(), 0);
    buf.extend(b"abcde\r\n");
    assert!(matches!(
        buf.line_end(3, &mut scanned),
        Err(Unread::Refuse(400))
    ));
    // A chunk-size line of 1100 bytes ("0…02"), sent with its whole request at once.
    let size = format!("{}2", "0".repeat(1099));
    let request = format!("{POST}Transfer-Encoding: chunked\r\n\r\n{size}\r\n{{}}\r\n0\r\n\r\n");
    assert_eq!(read_one(request.as_bytes()).await, Err(Some(400)));
}
