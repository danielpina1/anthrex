//! Milestone 9.6 task 17: a document's styled rows.

use super::*;
use crate::theme::Palette;

fn p() -> Palette {
    Palette {
        accent: crate::theme::DEFAULT_ACCENT,
        truecolor: false,
        ascii: false,
    }
}

fn text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn headings_are_bold_requirements_accented_and_code_dim() {
    let doc = "# Title\n\nR12 Reset by mail.\nRx is not one\n```\nlet a = 1;\n```\nplain";
    let lines = doc_lines(doc, 40, p());
    let texts: Vec<String> = lines.iter().map(text).collect();
    assert_eq!(
        texts,
        [
            "# Title",
            "",
            "R12 Reset by mail.",
            "Rx is not one",
            "```",
            "let a = 1;",
            "```",
            "plain"
        ]
    );
    assert!(lines[0].style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(lines[2].spans[0].content, "R12");
    assert_eq!(lines[2].spans[0].style, role(Role::Accent, p()));
    assert_eq!(lines[3].spans.len(), 1, "Rx is no requirement");
    for line in &lines[4..7] {
        assert_eq!(line.style, role(Role::Muted, p()), "{line:?}");
    }
    assert_eq!(lines[7].style, Style::default());
}

#[test]
fn rows_never_pass_the_width_and_lines_are_capped() {
    let long = format!("{}\n  - an indented item that wraps around", "y".repeat(95));
    for row in doc_lines(&long, 30, p()) {
        assert!(text(&row).width() <= 30, "{row:?}");
    }
    let wrapped: Vec<String> = doc_lines("  - an indented item that wraps", 16, p())
        .iter()
        .map(text)
        .collect();
    assert_eq!(wrapped, ["  - an indented", "  item that", "  wraps"]);
    let many = "l\n".repeat(MAX_LINES + 7);
    let lines = doc_lines(&many, 30, p());
    assert_eq!(lines.len(), MAX_LINES + 1);
    assert_eq!(text(lines.last().unwrap()), "[cut: 7 more lines]");
    let one = "z".repeat(MAX_LINE_CHARS + 50);
    let rows: usize = doc_lines(&one, 100, p()).len();
    assert_eq!(rows, (MAX_LINE_CHARS + 1).div_ceil(100));
}

#[test]
fn the_diff_colours_removed_and_added_lines() {
    let lines = diff_lines("@@ -1,1 +1,1 @@\n-old\n+new\n same", 20, p());
    assert_eq!(lines[0].style, role(Role::Muted, p()));
    assert_eq!(lines[1].style, role(Role::Failed, p()));
    assert_eq!(lines[2].style, role(Role::Done, p()));
    assert_eq!(lines[3].style, Style::default());
    assert_eq!(text(&lines[3]), " same");
}
