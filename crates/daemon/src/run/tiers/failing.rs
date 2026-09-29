//! Decision 32: the names of the failing tests in a red step's output, read with
//! built-in patterns for libtest, nextest, pytest and Go. Pure: the executor streams
//! the step's whole output in, line by line.

use super::FAILING_NAMES_MAX;

/// The failing-test names in `output_lines`, deduplicated in order of first
/// appearance and capped at [`FAILING_NAMES_MAX`]. None found: the step is retried
/// whole.
pub fn names(output_lines: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // libtest prints two `failures:` headers; only the second is followed at once by
    // the names, indented four spaces. The first is followed by a blank line and the
    // `---- <name> stdout ----` blocks, which are not read.
    let mut in_failures = false;
    for line in output_lines {
        if out.len() >= FAILING_NAMES_MAX {
            break;
        }
        let name = if in_failures {
            match line.strip_prefix("    ") {
                Some(name) if is_name(name) => Some(name.to_string()),
                _ => {
                    in_failures = false;
                    None
                }
            }
        } else {
            None
        };
        if line.trim_end() == "failures:" {
            in_failures = true;
            continue;
        }
        let name = name
            .or_else(|| libtest(&line))
            .or_else(|| nextest(&line))
            .or_else(|| pytest(&line))
            .or_else(|| go(&line));
        if let Some(name) = name
            && !out.contains(&name)
        {
            out.push(name);
        }
    }
    out
}

/// A test name: one word, no whitespace.
fn is_name(s: &str) -> bool {
    !s.is_empty() && !s.contains(char::is_whitespace)
}

/// `test <name> ... FAILED`.
fn libtest(line: &str) -> Option<String> {
    let name = line.strip_prefix("test ")?.strip_suffix(" ... FAILED")?;
    is_name(name).then(|| name.to_string())
}

/// `FAIL [ <time>] <binary> <name>` and `TIMEOUT [ <time>] <binary> <name>`, with an
/// optional `TRY <n> ` before and a `(<k>/<n>)` counter after the time.
fn nextest(line: &str) -> Option<String> {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("TRY ") {
        let (n, after) = after.split_once(' ')?;
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest = after.trim_start();
    }
    let rest = rest
        .strip_prefix("FAIL [")
        .or_else(|| rest.strip_prefix("TIMEOUT ["))?;
    let (_, rest) = rest.split_once(']')?;
    let mut rest = rest.trim_start();
    if rest.starts_with('(') {
        rest = rest.split_once(')')?.1.trim_start();
    }
    let (_binary, name) = rest.split_once(' ')?;
    let name = name.trim();
    is_name(name).then(|| name.to_string())
}

/// `FAILED <node id>`, the id ending before ` - `.
fn pytest(line: &str) -> Option<String> {
    let rest = line.strip_prefix("FAILED ")?;
    let id = rest.split(" - ").next()?.trim();
    is_name(id).then(|| id.to_string())
}

/// `--- FAIL: <Name> (<time>)`, a subtest's indented.
fn go(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("--- FAIL: ")?;
    let name = rest.split(" (").next()?.trim();
    is_name(name).then(|| name.to_string())
}
