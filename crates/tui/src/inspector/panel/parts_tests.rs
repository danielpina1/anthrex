//! 9.19 follow-up 1: a field wrapped by its ` · ` parts (`wrap_parts`), the run's
//! spend row.

use super::*;

const SPEND: &str = "tokens 75 (all roles) · cache hits 71% of input · worker tool calls 12";

#[test]
fn parts_pack_whole_and_join_back() {
    assert_eq!(wrap_parts(SPEND, 70, usize::MAX, false), [SPEND]);
    assert_eq!(
        wrap_parts(SPEND, 50, usize::MAX, false),
        [
            "tokens 75 (all roles) · cache hits 71% of input ·",
            "worker tool calls 12"
        ]
    );
    // From the widest word (`roles)`, six columns) up; a narrower row breaks words.
    for width in 6..=SPEND.len() {
        let lines = wrap_parts(SPEND, width, usize::MAX, false);
        assert_eq!(lines.join(" "), SPEND, "{width}");
        assert!(
            lines.iter().all(|l| l.width() <= width),
            "{width}: {lines:?}"
        );
    }
}

#[test]
fn a_part_wider_than_the_row_wraps_by_its_words() {
    assert_eq!(
        wrap_parts(SPEND, 12, usize::MAX, false),
        [
            "tokens 75",
            "(all roles)",
            "·",
            "cache hits",
            "71% of input",
            "·",
            "worker tool",
            "calls 12"
        ]
    );
}

#[test]
fn ascii_parts_break_at_the_folded_separator() {
    let ascii = "tokens 75 (all roles) - cache hits 71% of input - worker tool calls 12";
    assert_eq!(
        wrap_parts(ascii, 30, usize::MAX, true),
        [
            "tokens 75 (all roles) -",
            "cache hits 71% of input -",
            "worker tool calls 12"
        ]
    );
    // The count (`panel_rows`, plain) and the ASCII draw agree.
    assert_eq!(
        wrap_parts(ascii, 30, usize::MAX, false).len(),
        wrap_parts(ascii, 30, usize::MAX, true).len()
    );
}

/// Past the rows left, the rest joins the last row and is cut there: one row reads as
/// the unwrapped field's cut.
#[test]
fn too_few_rows_cut_the_rest_on_the_last() {
    assert_eq!(
        wrap_parts(SPEND, 30, 1, false),
        [crate::ui::tree_view::truncate_in(SPEND, 30, false)]
    );
    assert_eq!(
        wrap_parts(SPEND, 30, 2, false),
        [
            "tokens 75 (all roles) ·".to_owned(),
            crate::ui::tree_view::truncate_in(
                "cache hits 71% of input · worker tool calls 12",
                30,
                false
            )
        ]
    );
    assert!(wrap_parts(SPEND, 0, 3, false).is_empty());
    assert!(wrap_parts(SPEND, 30, 0, false).is_empty());
}
