//! Decision 32: the names of the failing tests in a red step's output, read with
//! built-in patterns for libtest, nextest, pytest and Go. Pure: the executor streams
//! the step's whole output in, line by line.
//!
//! Ruling C-6: the names are never a strict subset of the failures. A name is the
//! whole text between its markers, spaces included. When a failure line cannot be
//! read, or a runner's own summary counts more failures than were named, there are
//! no names, and the step is retried whole.

use super::FAILING_NAMES_MAX;

/// Which runner a name or a count came from; counts are compared per runner.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Runner {
    Libtest,
    Nextest,
    Pytest,
    Go,
}

const RUNNERS: [Runner; 4] = [Runner::Libtest, Runner::Nextest, Runner::Pytest, Runner::Go];

/// What one line says.
enum Seen {
    Name(Runner, String),
    /// A failure line whose name cannot be read.
    Unreadable,
    Nothing,
}

/// The failing-test names in `output_lines`, deduplicated in order of first
/// appearance and capped at [`FAILING_NAMES_MAX`] (a list that long is never retried
/// name by name: decision 33 does that for at most `RETRY_NAMES_MAX`). None found, or
/// possibly incomplete: the step is retried whole.
pub fn names(output_lines: impl Iterator<Item = String>) -> Vec<String> {
    let mut named: Vec<(Runner, String)> = Vec::new();
    let mut counted = [0u64; 4];
    let mut unreadable = false;
    let mut blocks = FailureBlocks::default();
    for line in output_lines {
        if let Some(block) = blocks.line(&line) {
            named.extend(block.into_iter().map(|n| (Runner::Libtest, n)));
        }
        for (runner, n) in summary(&line) {
            counted[runner as usize] += n;
        }
        match seen(&line) {
            Seen::Name(runner, name) => named.push((runner, name)),
            Seen::Unreadable => unreadable = true,
            Seen::Nothing => {}
        }
    }
    named.extend(blocks.finish().into_iter().map(|n| (Runner::Libtest, n)));
    if unreadable {
        return Vec::new();
    }
    for runner in RUNNERS {
        let mut distinct: Vec<&str> = named
            .iter()
            .filter(|(r, _)| *r == runner)
            .map(|(_, n)| n.as_str())
            .collect();
        distinct.sort_unstable();
        distinct.dedup();
        if (distinct.len() as u64) < counted[runner as usize] {
            return Vec::new();
        }
    }
    let mut out: Vec<String> = Vec::new();
    for (_, name) in named {
        if out.len() >= FAILING_NAMES_MAX {
            break;
        }
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// libtest's name list: the indented lines right after a `failures:` line, read only
/// from the last such block of a test binary's section (which a `test result:` line
/// ends), and only when a `---- <name> stdout ----` block came before it. A test's own
/// stdout that prints `failures:` is therefore never read as names.
#[derive(Default)]
struct FailureBlocks {
    stdout_seen: bool,
    /// The section's last block, and whether a stdout block preceded it.
    last: Option<(bool, Vec<String>)>,
    reading: bool,
}

impl FailureBlocks {
    /// Feeds one line; returns a section's names when the line ends the section.
    fn line(&mut self, line: &str) -> Option<Vec<String>> {
        if self.reading {
            match line.strip_prefix("    ").filter(|n| !n.trim().is_empty()) {
                Some(name) => {
                    if let Some((_, names)) = &mut self.last {
                        names.push(name.trim_end().to_string());
                    }
                    return None;
                }
                None => self.reading = false,
            }
        }
        if line.trim_end() == "failures:" {
            self.last = Some((self.stdout_seen, Vec::new()));
            self.reading = true;
        } else if line.starts_with("---- ") && line.trim_end().ends_with(" stdout ----") {
            self.stdout_seen = true;
        } else if line.starts_with("test result: ") {
            return Some(self.finish());
        }
        None
    }

    /// Ends a section.
    fn finish(&mut self) -> Vec<String> {
        let last = self.last.take();
        *self = FailureBlocks::default();
        match last {
            Some((true, names)) => names,
            _ => Vec::new(),
        }
    }
}

/// A failure line of one of the four runners.
fn seen(line: &str) -> Seen {
    let read = |runner, name: Option<&str>| match name.map(str::trim) {
        Some(name) if !name.is_empty() => Seen::Name(runner, name.to_string()),
        _ => Seen::Unreadable,
    };
    // libtest: `test <name> ... FAILED`.
    if let Some(rest) = line.strip_suffix(" ... FAILED") {
        return read(Runner::Libtest, rest.strip_prefix("test "));
    }
    // pytest: `FAILED <node id>[ - <message>]`, `ERROR <node id>[ - <message>]`. A
    // line that is not about a test file is someone else's.
    for prefix in ["FAILED ", "ERROR "] {
        if let Some(rest) = line.strip_prefix(prefix)
            && is_pytest_id(rest)
        {
            return read(Runner::Pytest, Some(pytest_id(rest)));
        }
    }
    // Go: `--- FAIL: <Name> (<time>)`, a subtest's indented.
    if let Some(rest) = line.trim_start().strip_prefix("--- FAIL:") {
        return read(Runner::Go, rest.rsplit_once(" (").map(|(n, _)| n));
    }
    nextest(line)
}

/// Whether `rest` starts with a pytest node id: a path to a `.py` file.
fn is_pytest_id(rest: &str) -> bool {
    let id = pytest_id(rest);
    let file = id.split("::").next().unwrap_or(id);
    file.ends_with(".py")
}

/// The node id: everything up to the ` - ` that starts the message, a ` - ` inside a
/// parametrised id's brackets not counting.
fn pytest_id(rest: &str) -> &str {
    let mut depth = 0i32;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            ' ' if depth <= 0 && rest[i..].starts_with(" - ") => return &rest[..i],
            _ => {}
        }
    }
    rest.trim_end()
}

/// nextest's terminal statuses that are not a pass: `<STATUS> [ <time>] [(<k>/<n>)]
/// <binary> <name>`, with an optional `TRY <n> ` before. `FAIL`, `TIMEOUT`, `ABORT`,
/// `SIGSEGV` and the other signals, `LEAK-FAIL`, and anything else in capitals except
/// the passing and progress ones.
fn nextest(line: &str) -> Seen {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("TRY ") {
        let Some((n, after)) = after.split_once(' ') else {
            return Seen::Nothing;
        };
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return Seen::Nothing;
        }
        rest = after.trim_start();
    }
    let Some((status, after)) = rest.split_once(" [") else {
        return Seen::Nothing;
    };
    let is_status = !status.is_empty()
        && status
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        && status.as_bytes()[0].is_ascii_uppercase();
    if !is_status
        || matches!(
            status,
            "PASS" | "SKIP" | "SLOW" | "LEAK" | "START" | "RETRY"
        )
    {
        return Seen::Nothing;
    }
    // The time: `[   0.004s]`, `[> 60.000s]`.
    let Some((time, after)) = after.split_once(']') else {
        return Seen::Nothing;
    };
    if !time.trim().trim_start_matches('>').trim().ends_with('s') {
        return Seen::Nothing;
    }
    let mut after = after.trim_start();
    if after.starts_with('(') {
        match after.split_once(')') {
            Some((_, rest)) => after = rest.trim_start(),
            None => return Seen::Unreadable,
        }
    }
    match after.split_once(' ') {
        Some((_binary, name)) if !name.trim().is_empty() => {
            Seen::Name(Runner::Nextest, name.trim().to_string())
        }
        _ => Seen::Unreadable,
    }
}

/// A runner's own count of failures on this line: libtest's `test result:` (`<n>
/// failed`), pytest's closing line (`<n> failed`, `<n> error(s)`), nextest's `Summary`
/// (`<n> failed`, `<n> timed out`).
fn summary(line: &str) -> Vec<(Runner, u64)> {
    let counts = |text: &str, words: &[&str], sep: char| -> u64 {
        text.split(sep)
            .filter_map(|part| {
                let (n, word) = part.trim().split_once(' ')?;
                let n: u64 = n.parse().ok()?;
                words.iter().any(|w| word.starts_with(w)).then_some(n)
            })
            .sum()
    };
    if let Some(rest) = line.strip_prefix("test result: ") {
        return vec![(Runner::Libtest, counts(rest, &["failed"], ';'))];
    }
    if let Some((_, rest)) = line.trim_start().split_once("Summary [")
        && let Some((_, rest)) = rest.split_once("tests run:")
    {
        return vec![(Runner::Nextest, counts(rest, &["failed", "timed out"], ','))];
    }
    // pytest: `==== 1 failed, 1 passed, 2 errors in 0.10s ====`, or without the `=`.
    let text = line.trim().trim_matches('=').trim();
    if let Some((parts, time)) = text.rsplit_once(" in ")
        && time.trim_end().ends_with('s')
        && !parts.is_empty()
        && parts.split(',').all(|part| {
            part.trim()
                .split_once(' ')
                .is_some_and(|(n, _)| n.parse::<u64>().is_ok())
        })
    {
        return vec![(Runner::Pytest, counts(parts, &["failed", "error"], ','))];
    }
    Vec::new()
}
