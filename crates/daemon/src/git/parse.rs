//! Pure parser for the output of
//! `git --no-optional-locks status --porcelain=v2 --branch --untracked-files=normal -z`.
//!
//! No process spawning, no filesystem, no async: [`crate::git::probe`] runs the command
//! and hands its stdout to [`parse_porcelain_v2_z`].
//!
//! `-z` NUL-terminates every record instead of newline-terminating it, and paths are
//! written verbatim (no quoting, no escaping), so a path containing a newline cannot
//! desynchronise the parser the way it could without `-z`. A `2 ` (renamed/copied)
//! record is the one exception to "one record, one NUL": its payload is two
//! NUL-separated paths, the new path then the original, so it consumes two tokens from
//! the split. Every other record type (`1 `, `u `, `? `, `! `) carries one path field
//! and is exactly one token.

use proto::Head;

/// Counts merged from ordinary, renamed, unmerged and untracked records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Counts {
    pub dirty: u32,
    pub untracked: u32,
    pub conflicts: u32,
}

/// One probe's worth of parsed branch and change-count information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub head: Head,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub counts: Counts,
}

/// What the `# branch.*` header lines have said so far.
#[derive(Default)]
struct Header {
    /// The full commit oid, when `branch.oid` named one rather than `(initial)`.
    oid: Option<String>,
    /// `branch.oid` was `(initial)`: there is no commit yet.
    unborn: bool,
    head: Option<HeadField>,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
}

enum HeadField {
    Detached,
    Branch(String),
}

impl Header {
    /// Combines `branch.oid` and `branch.head` into the [`Head`] they describe, or
    /// `None` when there is no head to describe.
    ///
    /// A `branch.head` line always accompanies `branch.oid` in real git output; a
    /// missing one only happens here when the input was cut between the two. There is
    /// then no branch name, and every way of inventing one is worse than saying
    /// nothing: an empty `Head::Unborn` is what the bottom bar used to render as a bare
    /// ` (unborn)`, a leading space with no name in front of it. `None` puts this in
    /// the same case as a probe that timed out before any header arrived — no head
    /// means no [`proto::GitState`], and the registry republishes the root's last known
    /// state as stale rather than showing a wrong one.
    fn resolve_head(&self) -> Option<Head> {
        match &self.head {
            Some(HeadField::Detached) => {
                let oid = self.oid.as_deref().unwrap_or("");
                Some(Head::Detached(oid.chars().take(7).collect()))
            }
            Some(HeadField::Branch(name)) => Some(if self.unborn {
                Head::Unborn(name.clone())
            } else {
                Head::Branch(name.clone())
            }),
            None => None,
        }
    }
}

/// Parses one probe's stdout. Truncated input is not an error: whatever complete
/// records precede the cut are kept, and the field says so is [`crate::git::probe`]'s
/// job (it sets `GitState::stale`), not this function's. Input is rejected in exactly
/// two cases: no `# branch.*` records at all, since even a clean repository emits them;
/// and a cut that landed between `# branch.oid` and `# branch.head`, which leaves no
/// head to report (see [`Header::resolve_head`]).
pub fn parse_porcelain_v2_z(output: &[u8]) -> Option<Parsed> {
    if output.is_empty() {
        return None;
    }

    let mut tokens: Vec<&[u8]> = output.split(|byte| *byte == 0).collect();
    // A well-formed -z stream ends with a record's own NUL, so split() reports one
    // trailing empty token that is a splitting artifact, not a record; an incomplete
    // stream instead ends with a fragment git never terminated. Either way, the last
    // token is not a record to parse, so drop it.
    tokens.pop();

    let mut header = Header::default();
    let mut counts = Counts::default();
    let mut saw_branch_record = false;

    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        if token.is_empty() {
            continue;
        }
        if let Some(line) = token.strip_prefix(b"# ") {
            saw_branch_record |= apply_header(line, &mut header);
        } else if token.starts_with(b"1 ") {
            if xy_is_dirty(token) {
                counts.dirty += 1;
            }
        } else if token.starts_with(b"2 ") {
            if xy_is_dirty(token) {
                counts.dirty += 1;
            }
            // The original path: a second NUL-separated field of this same record, not
            // a record of its own. Consume it whether or not this record counted as
            // dirty, so the next token is realigned either way.
            tokens.next();
        } else if token.starts_with(b"u ") {
            counts.conflicts += 1;
        } else if token.starts_with(b"? ") {
            counts.untracked += 1;
        } else if token.starts_with(b"! ") {
            // Ignored files are not part of GitState; nothing to count.
        }
        // Any other prefix is a record type this parser does not know. It occupies
        // exactly one token, like every record type but `2 `, so skipping the token is
        // enough: there is no extra path field to consume.
    }

    if !saw_branch_record {
        return None;
    }

    let head = header.resolve_head()?;
    Some(Parsed {
        head,
        upstream: header.upstream,
        ahead: header.ahead,
        behind: header.behind,
        counts,
    })
}

/// Applies one `# ...` header line (the bytes after `"# "`). Returns whether it was a
/// recognised `branch.*` record, so the caller can tell "no branch info at all," which
/// rejects the input, from "saw some," which does not.
fn apply_header(line: &[u8], header: &mut Header) -> bool {
    let Some(rest) = line.strip_prefix(b"branch.") else {
        return false;
    };
    if let Some(value) = rest.strip_prefix(b"oid ") {
        header.unborn = value == b"(initial)";
        header.oid = (!header.unborn).then(|| String::from_utf8_lossy(value).into_owned());
    } else if let Some(value) = rest.strip_prefix(b"head ") {
        header.head = Some(if value == b"(detached)" {
            HeadField::Detached
        } else {
            HeadField::Branch(String::from_utf8_lossy(value).into_owned())
        });
    } else if let Some(value) = rest.strip_prefix(b"upstream ") {
        header.upstream = Some(String::from_utf8_lossy(value).into_owned());
    } else if let Some(value) = rest.strip_prefix(b"ab ")
        && let Some((ahead, behind)) = parse_ab(value)
    {
        header.ahead = ahead;
        header.behind = behind;
    }
    true
}

/// Reads the `XY` status pair from a `1 ` or `2 ` record (the two bytes right after the
/// two-byte record marker) and reports whether it counts toward `dirty`: spec §3.2,
/// design decision 6 — staged and unstaged are merged into one count, so a record
/// counts when either half differs from `HEAD`, i.e. `X != '.' || Y != '.'`. A record
/// too short to hold `XY` (truncated input) is not counted.
fn xy_is_dirty(token: &[u8]) -> bool {
    match (token.get(2), token.get(3)) {
        (Some(&x), Some(&y)) => x != b'.' || y != b'.',
        _ => false,
    }
}

/// Parses a `branch.ab` value of the form `+<ahead> -<behind>`.
fn parse_ab(value: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(value).ok()?;
    let (ahead, behind) = text.split_once(' ')?;
    let ahead = ahead.strip_prefix('+')?.parse().ok()?;
    let behind = behind.strip_prefix('-')?.parse().ok()?;
    Some((ahead, behind))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Joins records with NUL, including a trailing one, matching real `-z` output
    /// where every record (including the last) is NUL-terminated.
    fn stream(records: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for record in records {
            out.extend_from_slice(record);
            out.push(0);
        }
        out
    }

    #[test]
    fn parses_a_clean_branch_with_upstream() {
        let input = stream(&[
            b"# branch.oid abcdef1234567890",
            b"# branch.head main",
            b"# branch.upstream origin/main",
            b"# branch.ab +0 -0",
        ]);
        let parsed = parse_porcelain_v2_z(&input).expect("branch header present");
        assert_eq!(parsed.head, Head::Branch("main".into()));
        assert_eq!(parsed.upstream, Some("origin/main".into()));
        assert_eq!(parsed.ahead, 0);
        assert_eq!(parsed.behind, 0);
        assert_eq!(parsed.counts, Counts::default());
    }

    #[test]
    fn counts_ahead_and_behind() {
        let input = stream(&[
            b"# branch.oid abcdef1234567890",
            b"# branch.head main",
            b"# branch.upstream origin/main",
            b"# branch.ab +2 -1",
        ]);
        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.ahead, 2);
        assert_eq!(parsed.behind, 1);
    }

    #[test]
    fn no_upstream_means_no_branch_ab() {
        let input = stream(&[b"# branch.oid abcdef1234567890", b"# branch.head main"]);
        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.upstream, None);
        assert_eq!(parsed.ahead, 0);
        assert_eq!(parsed.behind, 0);
    }

    #[test]
    fn detached_head_uses_the_short_oid() {
        let input = stream(&[
            b"# branch.oid 1a2b3c4d5e6f7890",
            b"# branch.head (detached)",
        ]);
        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.head, Head::Detached("1a2b3c4".into()));
    }

    #[test]
    fn unborn_branch_is_recognised() {
        let input = stream(&[b"# branch.oid (initial)", b"# branch.head main"]);
        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.head, Head::Unborn("main".into()));
    }

    #[test]
    fn ordinary_entries_count_once_each() {
        let input = stream(&[
            b"# branch.oid abcdef1234567890",
            b"# branch.head main",
            b"1 M. N... 100644 100644 100644 aaaa bbbb staged.txt",
            b"1 .M N... 100644 100644 100644 aaaa bbbb unstaged.txt",
            b"1 MM N... 100644 100644 100644 aaaa bbbb both.txt",
        ]);
        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.counts.dirty, 3);
        assert_eq!(parsed.counts.untracked, 0);
        assert_eq!(parsed.counts.conflicts, 0);
    }

    #[test]
    fn unchanged_xy_does_not_count_as_dirty() {
        // XY == ".." (real git never emits this for a "1 " record, since a file with no
        // difference from HEAD is not reported at all, but the parser must not rely on
        // that and must key off the XY field rather than merely on the record existing).
        let input = stream(&[
            b"# branch.oid abcdef1234567890",
            b"# branch.head main",
            b"1 .. N... 100644 100644 100644 aaaa bbbb unchanged.txt",
            b"1 M. N... 100644 100644 100644 aaaa bbbb staged.txt",
        ]);
        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.counts.dirty, 1);
    }

    #[test]
    fn rename_entries_consume_both_paths() {
        // The "2 " record's payload is "<path>\0<origPath>", two tokens; the "? "
        // record right after it must still parse as its own, separate record.
        let mut input = stream(&[b"# branch.oid abcdef1234567890", b"# branch.head main"]);
        input.extend_from_slice(b"2 R. N... 100644 100644 100644 aaaa bbbb R100 new.txt\0old.txt");
        input.push(0);
        input.extend_from_slice(&stream(&[b"? next.txt"]));

        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.counts.dirty, 1);
        assert_eq!(parsed.counts.untracked, 1);
        assert_eq!(parsed.head, Head::Branch("main".into()));
    }

    #[test]
    fn unmerged_and_untracked_are_counted_separately() {
        let input = stream(&[
            b"# branch.oid abcdef1234567890",
            b"# branch.head main",
            b"u UU N... 100644 100644 100644 100644 aaaa bbbb cccc both-modified.txt",
            b"u AA N... 100644 100644 100644 100644 aaaa bbbb cccc both-added.txt",
            b"? one.txt",
            b"? two.txt",
            b"? three.txt",
        ]);
        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.counts.conflicts, 2);
        assert_eq!(parsed.counts.untracked, 3);
        assert_eq!(parsed.counts.dirty, 0);
    }

    #[test]
    fn a_path_containing_a_newline_does_not_desynchronise() {
        let input = stream(&[
            b"# branch.oid abcdef1234567890",
            b"# branch.head main",
            b"? weird\nfile.txt",
        ]);
        let parsed = parse_porcelain_v2_z(&input).unwrap();
        assert_eq!(parsed.counts.untracked, 1);
    }

    #[test]
    fn truncated_output_parses_what_it_has() {
        let mut input = stream(&[
            b"# branch.oid abcdef1234567890",
            b"# branch.head main",
            b"? one.txt",
            b"? two.txt",
        ]);
        // A third untracked record, cut off mid-path: no terminating NUL follows.
        input.extend_from_slice(b"? three-cut-o");

        let parsed = parse_porcelain_v2_z(&input).expect("complete records precede the cut");
        assert_eq!(parsed.head, Head::Branch("main".into()));
        assert_eq!(parsed.counts.untracked, 2);
    }

    #[test]
    fn a_cut_between_the_oid_and_the_head_is_none() {
        // The one input that reaches `resolve_head`'s `None` arm: a probe cut after
        // `# branch.oid` but before `# branch.head`. There is no branch name in it, and
        // the fallback used to be an empty `Head::Unborn`, which the bottom bar
        // rendered as a bare " (unborn)" — a leading space with no name.
        let mut input = stream(&[b"# branch.oid abcdef1234567890"]);
        input.extend_from_slice(b"# branch.he");

        assert_eq!(parse_porcelain_v2_z(&input), None);
    }

    #[test]
    fn empty_output_is_none() {
        assert_eq!(parse_porcelain_v2_z(&[]), None);
    }
}
