use super::*;
use crate::theme::Palette;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

const P: Palette = Palette {
    accent: Color::Blue,
    truecolor: false,
};

fn text(line: &ratatui::text::Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn h(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.into(),
        word: word.into(),
        priority,
    }
}

#[test]
fn hints_drop_whole_hints_by_priority_and_keep_esc() {
    let all = [
        h("⏎", "answer", 3),
        h("m", "message", 1),
        h("o", "open", 2),
        h("esc", "back", 0),
    ];
    assert_eq!(
        text(&hints(80, &all, P)),
        "⏎ answer  m message  o open  esc back"
    );
    // 29 columns: `m message` (priority 1) goes first, whole; `esc back` (priority 0) stays.
    assert_eq!(text(&hints(29, &all, P)), "⏎ answer  o open  esc back");
    assert_eq!(text(&hints(9, &all, P)), "esc back");
    assert!(text(&hints(4, &all, P)).is_empty());
}

#[test]
fn hints_drop_the_rightmost_among_equal_priorities() {
    let all = [h("a", "one", 1), h("b", "two", 1), h("esc", "back", 0)];
    assert_eq!(text(&hints(80, &all, P)), "a one  b two  esc back");
    assert_eq!(text(&hints(16, &all, P)), "a one  esc back");
}

#[test]
fn hint_keys_are_accent_and_words_muted() {
    let line = hints(80, &[h("y", "accept", 1), h("esc", "back", 0)], P);
    let key = line.spans.iter().find(|s| s.content == "y").unwrap();
    let word = line.spans.iter().find(|s| s.content == "accept").unwrap();
    assert_eq!(
        key.style.fg,
        crate::theme::role(crate::theme::Role::Accent, P).fg
    );
    assert_eq!(
        word.style.fg,
        crate::theme::role(crate::theme::Role::Muted, P).fg
    );
}

#[test]
fn run_name_is_the_goal_cut_then_the_short_id() {
    assert_eq!(
        run_name("Add mul()", "add-mul-0723", 40),
        "Add mul() · 0723"
    );
    assert_eq!(
        run_name("Add a multiply function to crate a", "add-a-0723", 20),
        "Add a multip… · 0723"
    );
}

#[test]
fn kit_widgets_sanitise_their_inputs() {
    let hostile = crate::safe_text::tests::hostile_text();
    let rows = labelled_rows(&[("asked".into(), hostile.clone())], 60, P);
    let hint = hints(80, &[h(&hostile, &hostile, 1)], P);
    for line in rows.iter().chain(std::iter::once(&hint)) {
        assert_eq!(crate::safe_text::tests::first_hostile(&text(line)), None);
    }
    assert_eq!(
        crate::safe_text::tests::first_hostile(&run_name(&hostile, "x-0001", 40)),
        None
    );
    assert_eq!(
        crate::safe_text::tests::first_hostile(&choice(&hostile)),
        None
    );
    let labels = labelled_rows(&[(hostile.clone(), "v".into())], 60, P);
    assert_eq!(
        crate::safe_text::tests::first_hostile(&text(&labels[0])),
        None
    );
}

#[test]
fn labelled_rows_align_the_label_column() {
    let rows = labelled_rows(
        &[
            ("task".into(), "t1 add".into()),
            ("asked".into(), "one two three four five six".into()),
        ],
        20,
        P,
    );
    let lines: Vec<String> = rows.iter().map(text).collect();
    // The longest label is `asked` (5): values start at column 7.
    assert_eq!(lines[0], "task   t1 add");
    assert_eq!(lines[1], "asked  one two three");
    assert_eq!(lines[2], "       four five six");
    assert!(lines.iter().all(|l| l.chars().count() <= 20));
}

#[test]
fn labelled_rows_break_a_word_longer_than_the_column() {
    let rows = labelled_rows(&[("k".into(), "abcdefghijkl".into())], 8, P);
    let lines: Vec<String> = rows.iter().map(text).collect();
    assert_eq!(lines, ["k  abcde", "   fghij", "   kl"]);
}

#[test]
fn dialog_area_is_at_most_64_columns_and_centred() {
    let big = dialog_area(Rect::new(0, 0, 120, 40), 10);
    assert_eq!((big.width, big.height), (64, 12));
    assert_eq!((big.x, big.y), (28, 14));
    let small = dialog_area(Rect::new(0, 0, 80, 24), 10);
    assert_eq!((small.width, small.height), (64, 12));
    assert_eq!((small.x, small.y), (8, 6));
    let narrow = dialog_area(Rect::new(0, 0, 40, 10), 30);
    assert_eq!((narrow.width, narrow.height), (40, 10));
    assert_eq!((narrow.x, narrow.y), (0, 0));
}

#[test]
fn destructive_frame_titles_in_failed() {
    let draw = |destructive| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        dialog_frame("discard", destructive, P).render(buf.area, &mut buf);
        buf
    };
    let failed = crate::theme::role(crate::theme::Role::Failed, P).fg;
    let accent = crate::theme::role(crate::theme::Role::Accent, P).fg;
    let title_cell = |buf: &Buffer| buf[(2, 0)].clone();
    let d = draw(true);
    assert_eq!(title_cell(&d).symbol(), "d");
    assert_eq!(title_cell(&d).fg, failed.unwrap());
    let n = draw(false);
    assert_eq!(title_cell(&n).fg, accent.unwrap());
    // One accented border either way.
    assert_eq!(d[(0, 1)].fg, accent.unwrap());
}

#[test]
fn scroll_marks_read_up_and_down_n_more() {
    assert_eq!(
        scroll_marks(3, 7, false),
        (Some("↑ 3 more".into()), Some("↓ 7 more".into()))
    );
    assert_eq!(
        scroll_marks(3, 7, true),
        (Some("^ 3 more".into()), Some("v 7 more".into()))
    );
    assert_eq!(scroll_marks(0, 0, false), (None, None));
}

#[test]
fn choice_wraps_in_guillemets() {
    assert_eq!(choice("info"), "‹ info ›");
}
