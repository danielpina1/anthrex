//! Fix round 1 (ruling T16-fix): one escaping layer for every piece of text the report
//! interpolates that did not come from the engine's own closed vocabulary (an enum
//! label, a sha, a branch name) — a plan's task title, a reviewer's summary and
//! findings, a check's raw tool output, and the engine's own free-text notes and
//! history lines. None of it is trusted to be single-line, `|`-free or `#`-free:
//! titles and review text are LLM-authored (the spec's planner and reviewer roles),
//! and check tails are arbitrary `check`-command output. Pure, same terms as
//! `report.rs`.

/// A table cell: a literal `|` would add a spurious column, and a literal newline
/// would split the row across lines and desynchronise every row after it. Both are
/// flattened; a cell is always exactly one line.
pub(super) fn escape_cell(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace('|', "\\|")
}

/// A `##`-heading line: a literal newline would turn the rest of the title into its
/// own (unindented, so invalid) line, and a title that itself starts with `#` would
/// read as extra leading `#`s on the heading marker once newlines are gone. Newlines
/// collapse to spaces; a genuine leading `#` is escaped so it reads as text.
pub(super) fn escape_heading(s: &str) -> String {
    let collapsed = s.replace(['\n', '\r'], " ");
    if let Some(rest) = collapsed.strip_prefix('#') {
        format!("\\#{rest}")
    } else {
        collapsed
    }
}

/// A free-text line or a list item's text: an embedded newline must not start a new
/// top-level block, and a line that starts with `#` on its own must not read as an ATX
/// heading. CommonMark never treats 4-or-more leading spaces as a heading, and a
/// continuation indented under a list item's marker (`"- "`, width 2) stays part of
/// that item rather than becoming its own block — so every line but the first is
/// indented 4 spaces, which is enough for both cases at once.
pub(super) fn continuation_indent(s: &str) -> String {
    let mut lines = s.lines();
    let mut out = lines.next().unwrap_or("").to_string();
    for line in lines {
        out.push_str("\n    ");
        out.push_str(line);
    }
    out
}

/// A fence one backtick longer than the longest run of backticks in `content`, never
/// shorter than 3: a check's tail is raw tool output and may itself contain a fenced
/// block (a test that prints Markdown, another tool's own fenced output), which a
/// fixed 3-backtick fence would let close the report's fence early.
pub(super) fn fence_for(content: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for ch in content.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_cell_flattens_newlines_and_escapes_pipes() {
        assert_eq!(escape_cell("a | b\nc"), "a \\| b c");
        assert_eq!(escape_cell("plain"), "plain");
    }

    #[test]
    fn escape_heading_only_escapes_a_genuine_leading_hash() {
        assert_eq!(escape_heading("# title\nsecond"), "\\# title second");
        assert_eq!(escape_heading("mid # not leading"), "mid # not leading");
    }

    #[test]
    fn continuation_indent_leaves_a_single_line_untouched() {
        assert_eq!(continuation_indent("one line"), "one line");
    }

    #[test]
    fn continuation_indent_indents_every_later_line() {
        assert_eq!(
            continuation_indent("first\n# looks like a heading\nthird"),
            "first\n    # looks like a heading\n    third"
        );
    }

    #[test]
    fn fence_for_is_one_longer_than_the_longest_run_and_never_shorter_than_three() {
        assert_eq!(fence_for("no backticks here"), "```");
        assert_eq!(fence_for("a `` run"), "```");
        assert_eq!(fence_for("a ``` run"), "````");
        assert_eq!(fence_for("````\nfour"), "`````");
    }
}
