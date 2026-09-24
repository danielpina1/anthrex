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

/// Escapes the block-start marker CommonMark (or its GFM table extension) would read
/// at the beginning of `line` (after up to 3 leading spaces, its own indentation
/// budget): a heading (`#`), a blockquote (`>`), a bullet list or thematic break
/// (`-`, `+`, `*`), a fenced code block (`` ` `` or `~`), a setext-heading underline
/// (`=`), a table row or delimiter (`|`), or an ordered list marker (1-9 digits then
/// `.` or `)`). A digit itself cannot be backslash-escaped in CommonMark (only ASCII
/// punctuation can), so an ordered marker is neutralised by escaping its delimiter
/// instead — `1\.` is inert where `\1.` would not be. Block-start detection reads raw
/// characters ahead of any inline (escape) processing, so a leading `\` in the source
/// is enough on its own to fail every one of these checks, whatever character follows
/// it.
fn escape_block_start(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;
    while i < 3 && chars.get(i) == Some(&' ') {
        i += 1;
    }
    let Some(&c) = chars.get(i) else {
        return line.to_string();
    };
    if "#>-+*=~`|".contains(c) {
        let mut out: String = chars[..i].iter().collect();
        out.push('\\');
        out.extend(&chars[i..]);
        return out;
    }
    let start = i;
    while i < chars.len() && chars[i].is_ascii_digit() && i - start < 9 {
        i += 1;
    }
    if i > start && matches!(chars.get(i), Some('.') | Some(')')) {
        let mut out: String = chars[..i].iter().collect();
        out.push('\\');
        out.extend(&chars[i..]);
        return out;
    }
    line.to_string()
}

/// Text that will sit directly inside a `"- "` list item, `prefix` first on the same
/// physical line (a timestamp, a `<file>:<line>: ` locator — already flattened to one
/// line by the caller): every line, the first included, has its own leading
/// block-start marker escaped, and every line after the first is indented to the
/// item's content column (2, matching `"- "`'s width) so it stays part of the same
/// item. Round 1's `continuation_indent` alone was not enough here: CommonMark only
/// requires 2 columns to stay *inside* the item, and up to 3 more are still read as a
/// block start, so its 4-space indent left 2 spare columns block-start rules still
/// see (re-review 1, I1).
pub(super) fn list_item_text(prefix: &str, text: &str) -> String {
    let mut lines = text.lines();
    let first = format!("{prefix}{}", lines.next().unwrap_or(""));
    let mut out = escape_block_start(&first);
    for line in lines {
        out.push_str("\n  ");
        out.push_str(&escape_block_start(line));
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
    fn escape_block_start_escapes_every_named_marker() {
        assert_eq!(escape_block_start("# heading"), "\\# heading");
        assert_eq!(escape_block_start("> quote"), "\\> quote");
        assert_eq!(escape_block_start("- bullet"), "\\- bullet");
        assert_eq!(escape_block_start("+ bullet"), "\\+ bullet");
        assert_eq!(escape_block_start("* bullet"), "\\* bullet");
        assert_eq!(escape_block_start("--- break"), "\\--- break");
        assert_eq!(escape_block_start("=== setext"), "\\=== setext");
        assert_eq!(escape_block_start("``` fence"), "\\``` fence");
        assert_eq!(escape_block_start("~~~ fence"), "\\~~~ fence");
        assert_eq!(escape_block_start("1. ordered"), "1\\. ordered");
        assert_eq!(escape_block_start("42) ordered"), "42\\) ordered");
        assert_eq!(escape_block_start("plain text"), "plain text");
        assert_eq!(escape_block_start("mid # not leading"), "mid # not leading");
        // Up to 3 leading spaces are still a block start in CommonMark.
        assert_eq!(
            escape_block_start("   # indented heading"),
            "   \\# indented heading"
        );
        // 4+ leading spaces would already be an indented code block, out of scope here.
        assert_eq!(escape_block_start(""), "");
    }

    #[test]
    fn list_item_text_escapes_the_first_line_too() {
        assert_eq!(
            list_item_text("", "# looks like a heading"),
            "\\# looks like a heading"
        );
        assert_eq!(
            list_item_text("evil.rs:1: ", "# looks like a heading"),
            "evil.rs:1: # looks like a heading"
        );
    }

    #[test]
    fn list_item_text_indents_and_escapes_every_continuation_line() {
        assert_eq!(
            list_item_text("", "first\n# heading\n```\nfence\n1. ordered"),
            "first\n  \\# heading\n  \\```\n  fence\n  1\\. ordered"
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
