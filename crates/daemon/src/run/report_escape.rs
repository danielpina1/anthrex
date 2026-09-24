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

/// Fix round 3 (ruling T16-R3, re-review 2): round 2's `escape_block_start` named
/// markers one construct at a time (`#`, `>`, `-`, …) and missed three more ways in —
/// a bare `===`/`---` first line setext-promoting the line above it (NB1, not covered
/// by any list-item mechanism at all: it hits a *plain* line), a leading tab (NB2:
/// `escape_block_start`'s column counter only advanced past literal spaces, so a tab
/// looked like "no indentation budget left" and the loop gave up before it ever
/// reached the marker check), and `<` opening a real HTML block (NB3: simply absent
/// from the marker set). `normalize_line` replaces it with one rule instead of a
/// hand-maintained list: CommonMark lets *any* ASCII punctuation character be
/// backslash-escaped, and every block-start marker in CommonMark (headings, quotes,
/// lists, thematic breaks, fences, setext underlines, HTML blocks, table rows) is
/// punctuation-led except the ordered-list marker, which is digit-led and needs its
/// own rule since a digit cannot itself be escaped.
///
/// Steps, applied to one line: expand tabs to spaces (so indentation is counted in
/// real CommonMark columns, not raw characters — NB2), strip all leading whitespace
/// (a normalized line's indentation is always supplied by its caller afterwards, so
/// none of the original survives to be miscounted), then backslash-escape the first
/// remaining character if it is ASCII punctuation, or an ordered marker's delimiter
/// (`.`/`)`) if the line starts with 1-9 digits followed by one. A line empty after
/// trimming stays empty.
fn normalize_line(line: &str) -> String {
    let expanded = line.replace('\t', "    ");
    let trimmed = expanded.trim_start_matches(' ');
    if trimmed.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars[0].is_ascii_punctuation() {
        let mut out = String::with_capacity(chars.len() + 1);
        out.push('\\');
        out.extend(&chars);
        return out;
    }
    if chars[0].is_ascii_digit() {
        let mut i = 0usize;
        while i < chars.len() && chars[i].is_ascii_digit() && i < 9 {
            i += 1;
        }
        if i < chars.len() && matches!(chars[i], '.' | ')') {
            let mut out: String = chars[..i].iter().collect();
            out.push('\\');
            out.extend(&chars[i..]);
            return out;
        }
    }
    trimmed.to_string()
}

/// A free-text line, first line included: `Run.goal`, a review round's `summary`,
/// `merged without approval`'s reason. Every line is normalized; every line after the
/// first is then indented 4 spaces, enough to stay out of block-start range for a
/// plain (non-list-item) line. `Goal: `/`merged without approval: ` already prefix
/// their first line with safe static text, so normalizing it is a no-op there; a
/// review summary has no such prefix, so normalizing its first line is what fixes
/// NB1 (round 2's `continuation_indent` only touched *continuation* lines).
pub(super) fn plain_text_line(s: &str) -> String {
    let mut lines = s.lines();
    let mut out = normalize_line(lines.next().unwrap_or(""));
    for line in lines {
        out.push_str("\n    ");
        out.push_str(&normalize_line(line));
    }
    out
}

/// Text that will sit directly inside a `"- "` list item, `prefix` first on the same
/// physical line (a timestamp, a `<file>:<line>: ` locator — already flattened to one
/// line by the caller): every line, the first included, is normalized, and every line
/// after the first is indented to the item's content column (2, matching `"- "`'s
/// width) so it stays part of the same item.
pub(super) fn list_item_text(prefix: &str, text: &str) -> String {
    let mut lines = text.lines();
    let first = format!("{prefix}{}", lines.next().unwrap_or(""));
    let mut out = normalize_line(&first);
    for line in lines {
        out.push_str("\n  ");
        out.push_str(&normalize_line(line));
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
    fn plain_text_line_leaves_a_single_line_untouched() {
        assert_eq!(plain_text_line("one line"), "one line");
    }

    #[test]
    fn plain_text_line_indents_every_later_line() {
        assert_eq!(
            plain_text_line("first\n# looks like a heading\nthird"),
            "first\n    \\# looks like a heading\n    third"
        );
    }

    #[test]
    fn plain_text_line_escapes_a_hostile_first_line_too() {
        // NB1: a bare `===`/`---` first line, with nothing static in front of it,
        // would setext-promote whatever line precedes it.
        assert_eq!(plain_text_line("===\nrest"), "\\===\n    rest");
        assert_eq!(plain_text_line("---"), "\\---");
    }

    #[test]
    fn normalize_line_escapes_every_named_marker() {
        assert_eq!(normalize_line("# heading"), "\\# heading");
        assert_eq!(normalize_line("> quote"), "\\> quote");
        assert_eq!(normalize_line("- bullet"), "\\- bullet");
        assert_eq!(normalize_line("+ bullet"), "\\+ bullet");
        assert_eq!(normalize_line("* bullet"), "\\* bullet");
        assert_eq!(normalize_line("--- break"), "\\--- break");
        assert_eq!(normalize_line("=== setext"), "\\=== setext");
        assert_eq!(normalize_line("``` fence"), "\\``` fence");
        assert_eq!(normalize_line("~~~ fence"), "\\~~~ fence");
        assert_eq!(normalize_line("| table |"), "\\| table |");
        assert_eq!(normalize_line("<div>"), "\\<div>");
        assert_eq!(normalize_line("1. ordered"), "1\\. ordered");
        assert_eq!(normalize_line("42) ordered"), "42\\) ordered");
        assert_eq!(normalize_line("plain text"), "plain text");
        assert_eq!(normalize_line("mid # not leading"), "mid # not leading");
        assert_eq!(normalize_line(""), "");
    }

    #[test]
    fn normalize_line_strips_leading_whitespace_before_checking_the_marker() {
        // Round 2's `escape_block_start` only advanced past literal spaces when
        // counting a line's indentation budget: a tab looked like "no budget left"
        // and the loop stopped before ever reaching the marker check (NB2).
        // `normalize_line` strips every kind of leading whitespace outright, so a
        // tab can no longer hide a marker from the check at all.
        assert_eq!(
            normalize_line("   # indented heading"),
            "\\# indented heading"
        );
        assert_eq!(
            normalize_line("\t# tab-indented heading"),
            "\\# tab-indented heading"
        );
        assert_eq!(normalize_line(" \t # mixed"), "\\# mixed");
        // A line that is only whitespace stays empty, not a backslash on its own.
        assert_eq!(normalize_line("   "), "");
        assert_eq!(normalize_line("\t\t"), "");
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
    fn list_item_text_neutralises_a_tab_and_an_angle_bracket() {
        assert_eq!(
            list_item_text("", "first\n\t# tab heading\n<div>"),
            "first\n  \\# tab heading\n  \\<div>"
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
