//! The OTLP receiver's HTTP/1.1 reading and answering (M8b decision 30), with no HTTP
//! crate. I/O: one [`Conn`] per connection, read into a [`ReadBuf`].
//!
//! Every limit is checked before the bytes it limits are kept: headers up to
//! [`OTLP_MAX_HEADERS`], a `Content-Length` or chunked body up to [`OTLP_MAX_BODY`], a
//! chunk-size line up to [`MAX_CHUNK_LINE`]. The caller bounds each request's reading
//! in time.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::server::{OTLP_MAX_BODY, OTLP_MAX_HEADERS, OTLP_READ_TIMEOUT};

/// The longest chunk-size line accepted.
const MAX_CHUNK_LINE: usize = 1024;
/// How long a refused connection is drained before it closes, so the client can read
/// the answer rather than a reset.
const LINGER: Duration = Duration::from_millis(500);
/// The consumed prefix is dropped once it is this long and at least half the buffer, so
/// every byte is moved at most once more than it is read (review minor 5).
const COMPACT_AT: usize = 64 << 10;
/// The capacity a kept-alive connection keeps between requests (review minor 7).
pub(super) const KEEP_CAPACITY: usize = 64 << 10;

/// Bytes read from a connection and not yet consumed. Consuming advances an offset
/// instead of moving what follows, so a body of many small chunks reads in linear time.
#[derive(Default)]
pub(super) struct ReadBuf {
    data: Vec<u8>,
    start: usize,
}

impl ReadBuf {
    pub(super) fn pending(&self) -> &[u8] {
        &self.data[self.start..]
    }

    pub(super) fn len(&self) -> usize {
        self.data.len() - self.start
    }

    pub(super) fn extend(&mut self, bytes: &[u8]) {
        if self.start == self.data.len() {
            self.data.clear();
            self.start = 0;
        } else if self.start >= COMPACT_AT && self.start * 2 >= self.data.len() {
            self.data.drain(..self.start);
            self.start = 0;
        }
        self.data.extend_from_slice(bytes);
    }

    /// Marks the next `n` pending bytes consumed (`n` is at most [`Self::len`]).
    pub(super) fn skip(&mut self, n: usize) {
        self.start += n.min(self.len());
    }

    /// The next `n` pending bytes, consumed (`n` is at most [`Self::len`]).
    pub(super) fn take(&mut self, n: usize) -> Vec<u8> {
        let out = self.pending()[..n].to_vec();
        self.skip(n);
        out
    }

    /// Between two requests of a kept-alive connection: drops what was consumed, and
    /// gives back what a large request needed beyond [`KEEP_CAPACITY`].
    pub(super) fn settle(&mut self) {
        self.data.drain(..self.start);
        self.start = 0;
        if self.data.capacity() > KEEP_CAPACITY {
            self.data.shrink_to(KEEP_CAPACITY.max(self.data.len()));
        }
    }
}

/// A request that was read in full.
pub(super) struct Request {
    pub method: String,
    pub path: String,
    pub content_type: String,
    pub body: Vec<u8>,
    pub keep_alive: bool,
}

/// Why a request was not read.
pub(super) enum Unread {
    /// The client closed, the read failed, or it ran out of time: close silently.
    Gone,
    /// Answer this status, then close.
    Refuse(u16),
}

/// A connection and the bytes read past the last request.
pub(super) struct Conn {
    stream: TcpStream,
    pub buf: ReadBuf,
}

impl Conn {
    pub(super) fn new(stream: TcpStream) -> Self {
        Conn {
            stream,
            buf: ReadBuf::default(),
        }
    }

    /// Reads more into the buffer; `Gone` at end of stream or on an error.
    async fn fill(&mut self) -> Result<(), Unread> {
        let mut chunk = [0u8; 16 << 10];
        match self.stream.read(&mut chunk).await {
            Ok(0) | Err(_) => Err(Unread::Gone),
            Ok(n) => {
                self.buf.extend(&chunk[..n]);
                Ok(())
            }
        }
    }

    pub(super) async fn request(&mut self) -> Result<Request, Unread> {
        let head_end = loop {
            if let Some(i) = find(self.buf.pending(), b"\r\n\r\n") {
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
        let head = self.buf.take(head_end);
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
            (false, n) => {
                let n = n.unwrap_or(0);
                self.wait_for(n).await?;
                self.buf.take(n)
            }
        };
        Ok(Request {
            method: method.to_string(),
            path: path.to_string(),
            content_type,
            body,
            keep_alive,
        })
    }

    /// Reads until at least `n` bytes are pending (`n` is at most [`OTLP_MAX_BODY`]).
    async fn wait_for(&mut self, n: usize) -> Result<(), Unread> {
        while self.buf.len() < n {
            self.fill().await?;
        }
        Ok(())
    }

    /// One line, without its CRLF, of at most `max` bytes. A line of `max` bytes whose
    /// CR has arrived without its LF waits for the LF (review I1): TCP may split a read
    /// between the two.
    async fn line(&mut self, max: usize) -> Result<Vec<u8>, Unread> {
        loop {
            let pending = self.buf.pending();
            let window = &pending[..pending.len().min(max + 2)];
            if let Some(i) = find(window, b"\r\n") {
                let line = self.buf.take(i);
                self.buf.skip(2);
                return Ok(line);
            }
            if self.buf.len() > max + 1 {
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
            self.wait_for(size).await?;
            body.extend_from_slice(&self.buf.pending()[..size]);
            self.buf.skip(size);
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

    pub(super) async fn respond(&mut self, status: u16, keep_alive: bool) -> std::io::Result<()> {
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
    pub(super) async fn refuse(mut self, status: u16) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
