//! The line lexer behind the writer (decision 30): which line starts a table header,
//! which a `key = value` (and how many lines its value spans), and where a single-line
//! value sits between its key and its comment. Leading whitespace is allowed before a
//! header or key (preflight F17).

#[derive(Debug)]
pub(super) enum Kind {
    Other,
    Header {
        path: Vec<String>,
        array: bool,
    },
    KeyVal {
        key: Vec<String>,
        /// The last line of the value; the key's own line when it fits on one.
        last: usize,
        /// The value's byte span on a single-line value.
        span: Option<(usize, usize)>,
    },
    Continuation,
}

pub(super) fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// What line `i` starts: a header, a key (and how far its value runs), or anything else.
pub(super) fn classify(lines: &[(String, &str)], i: usize) -> Kind {
    let line = lines[i].0.as_str();
    let body = line.trim_start();
    if body.is_empty() || body.starts_with('#') {
        return Kind::Other;
    }
    if body.starts_with('[') {
        return header(body).unwrap_or(Kind::Other);
    }
    let Some((key, at)) = parse_key(line, line.len() - body.len()) else {
        return Kind::Other;
    };
    let Some(rest) = line[at..].trim_start().strip_prefix('=') else {
        return Kind::Other;
    };
    let value_at = line.len() - rest.len();
    // The value ends on the first line where `x =` plus the lines so far reads as TOML.
    let mut probe = format!("x ={rest}");
    let mut last = i;
    while probe.parse::<toml::Table>().is_err() {
        last += 1;
        if last == lines.len() {
            return Kind::Other;
        }
        probe.push('\n');
        probe.push_str(&lines[last].0);
    }
    let span = (last == i).then(|| value_span(line, value_at));
    Kind::KeyVal { key, last, span }
}

fn header(body: &str) -> Option<Kind> {
    let (array, open) = if body.starts_with("[[") {
        (true, 2)
    } else {
        (false, 1)
    };
    let (path, at) = parse_key(body, open)?;
    let close = if array { "]]" } else { "]" };
    let tail = body[at..].trim_start().strip_prefix(close)?.trim_start();
    (tail.is_empty() || tail.starts_with('#')).then_some(Kind::Header { path, array })
}

/// A dotted key starting at byte `at`: its segments (quotes resolved) and where it ends.
fn parse_key(line: &str, mut at: usize) -> Option<(Vec<String>, usize)> {
    let mut path = Vec::new();
    loop {
        at += line[at..].len() - line[at..].trim_start().len();
        let rest = &line[at..];
        let (segment, used) = if let Some(r) = rest.strip_prefix('"') {
            let end = string_end(r, '"')?;
            let raw = format!("\"{}\"", &r[..end]);
            let value: toml::Value = format!("x = {raw}")
                .parse::<toml::Table>()
                .ok()?
                .remove("x")?;
            (value.as_str()?.to_string(), end + 2)
        } else if let Some(r) = rest.strip_prefix('\'') {
            let end = r.find('\'')?;
            (r[..end].to_string(), end + 2)
        } else {
            let n = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(rest.len());
            if n == 0 {
                return None;
            }
            (rest[..n].to_string(), n)
        };
        path.push(segment);
        at += used;
        let after = line[at..].trim_start();
        match after.strip_prefix('.') {
            Some(r) => at = line.len() - r.len(),
            None => return Some((path, at)),
        }
    }
}

/// The byte index in `s` of the quote closing a string whose opening quote is just
/// before `s`; a basic string's `\` escapes the next character.
fn string_end(s: &str, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quote == '"' => escaped = true,
            c if c == quote => return Some(i),
            _ => {}
        }
    }
    None
}

/// The span of a single-line value starting after byte `at`: up to a `#` outside a
/// string, less trailing whitespace, so a trailing comment and its spacing stay.
fn value_span(line: &str, at: usize) -> (usize, usize) {
    let start = at + (line[at..].len() - line[at..].trim_start().len());
    let mut i = start;
    let bytes = line.as_bytes();
    while i < line.len() {
        let rest = &line[i..];
        let triple = rest.starts_with("\"\"\"") || rest.starts_with("'''");
        match bytes[i] {
            b'#' => break,
            q @ (b'"' | b'\'') if triple => {
                let delim = if q == b'"' { "\"\"\"" } else { "'''" };
                i += 3 + rest[3..].find(delim).map_or(rest.len() - 3, |n| n + 3);
            }
            q @ (b'"' | b'\'') => {
                let end = string_end(&rest[1..], q as char).map_or(rest.len() - 1, |n| n + 1);
                i += 1 + end;
            }
            _ => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    let end = start + line[start..i.min(line.len())].trim_end().len();
    (start, end)
}
