//! Milestone 9.3 (task M9.3.9a): the editor's keys, one test per row of KG §1.2, the
//! cut buffer, the cap and the hostile paste. Every test drives `on_editor_key` the way
//! the goal and iterate dialogs will, at a drawn width and a page.

use super::super::TextArea;
use super::EditorKey;
use crate::run_edit::TEXT_MAX_CHARS;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn ctrl_code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

/// Wide enough that nothing wraps; a page of four rows (five drawn, less one).
const W: u16 = 80;
const PAGE: usize = 4;

fn press(area: &mut TextArea, k: KeyEvent) -> EditorKey {
    area.on_editor_key(k, W, PAGE)
}

fn type_str(area: &mut TextArea, s: &str) {
    for c in s.chars() {
        let k = if c == '\n' {
            key(KeyCode::Enter)
        } else {
            key(KeyCode::Char(c))
        };
        assert_eq!(press(area, k), EditorKey::Edited, "{c:?}");
    }
}

/// The cursor as `(logical line, grapheme column)`, both from 0, worked out here
/// rather than through `position`.
fn at(area: &TextArea) -> (usize, usize) {
    let before: String = area.text().graphemes(true).take(area.cursor()).collect();
    let line = before.matches('\n').count();
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .graphemes(true)
        .count();
    (line, column)
}

/// A text of `n` lines `line0` … `line<n-1>`, the cursor at the end.
fn lines(n: usize) -> TextArea {
    let text: Vec<String> = (0..n).map(|i| format!("line{i}")).collect();
    TextArea::editor(&text.join("\n"))
}

#[test]
fn editor_a_character_inserts_it() {
    let mut a = TextArea::editor("");
    type_str(&mut a, "hé");
    assert_eq!(
        press(
            &mut a,
            KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT)
        ),
        EditorKey::Edited
    );
    assert_eq!(a.text(), "héX");
    assert_eq!(a.cursor(), 3);
    // In the middle of the text, at the cursor.
    press(&mut a, key(KeyCode::Left));
    type_str(&mut a, "!");
    assert_eq!(a.text(), "hé!X");
}

#[test]
fn editor_enter_inserts_a_newline() {
    let mut a = TextArea::editor("");
    type_str(&mut a, "ab");
    assert_eq!(press(&mut a, key(KeyCode::Enter)), EditorKey::Edited);
    type_str(&mut a, "cd");
    assert_eq!(press(&mut a, ctrl('j')), EditorKey::Edited, "Ctrl-J too");
    assert_eq!(a.text(), "ab\ncd\n");
    assert_eq!(at(&a), (2, 0));
}

#[test]
fn editor_backspace_and_delete_delete_before_and_after_the_cursor() {
    let mut a = TextArea::editor("ab\ncd");
    press(&mut a, key(KeyCode::Left));
    assert_eq!(press(&mut a, key(KeyCode::Backspace)), EditorKey::Edited);
    assert_eq!(a.text(), "ab\nd");
    press(&mut a, key(KeyCode::Backspace));
    assert_eq!(
        a.text(),
        "abd",
        "Backspace at a line's start joins the lines"
    );
    assert_eq!(press(&mut a, key(KeyCode::Delete)), EditorKey::Edited);
    assert_eq!(a.text(), "ab");
    // At the ends there is nothing to delete; the key is still the editor's.
    assert_eq!(press(&mut a, key(KeyCode::Delete)), EditorKey::Moved);
    press(&mut a, ctrl_code(KeyCode::Home));
    assert_eq!(press(&mut a, key(KeyCode::Backspace)), EditorKey::Moved);
    assert_eq!(a.text(), "ab");
}

#[test]
fn editor_left_and_right_move_by_grapheme() {
    // `e` + a combining acute is one grapheme.
    let mut a = TextArea::editor("ae\u{301}b");
    assert_eq!(a.cursor(), 3);
    assert_eq!(press(&mut a, key(KeyCode::Left)), EditorKey::Moved);
    assert_eq!(a.cursor(), 2);
    press(&mut a, key(KeyCode::Left));
    assert_eq!(a.cursor(), 1, "over the whole cluster");
    press(&mut a, key(KeyCode::Left));
    press(&mut a, key(KeyCode::Left));
    assert_eq!(a.cursor(), 0, "stops at the start");
    press(&mut a, key(KeyCode::Right));
    assert_eq!(a.cursor(), 1);
    for _ in 0..5 {
        press(&mut a, key(KeyCode::Right));
    }
    assert_eq!(a.cursor(), 3, "stops at the end");
}

#[test]
fn editor_up_and_down_move_by_drawn_row() {
    // `abcdefghij` drawn 6 columns wide wraps as `abcde` / `fghij` (one short of the
    // width, as the kit draws it).
    let mut a = TextArea::editor("abcdefghij\nxy");
    press(&mut a, ctrl_code(KeyCode::Home));
    for _ in 0..2 {
        press(&mut a, key(KeyCode::Right));
    }
    assert_eq!(
        a.on_editor_key(key(KeyCode::Down), 6, PAGE),
        EditorKey::Moved
    );
    assert_eq!(a.cursor(), 7, "a drawn row down, the same column");
    a.on_editor_key(key(KeyCode::Down), 6, PAGE);
    assert_eq!(at(&a), (1, 2), "clamped to the end of the shorter row");
    a.on_editor_key(key(KeyCode::Up), 6, PAGE);
    assert_eq!(a.cursor(), 7);
    a.on_editor_key(key(KeyCode::Up), 6, PAGE);
    assert_eq!(a.cursor(), 2);
}

#[test]
fn editor_up_on_the_first_row_stays_put() {
    let mut a = TextArea::editor("one\ntwo");
    press(&mut a, key(KeyCode::Up));
    assert_eq!(at(&a), (0, 3));
    // The key is the editor's: the dialog does not take it to move the focus.
    assert_eq!(press(&mut a, key(KeyCode::Up)), EditorKey::Moved);
    assert_eq!(at(&a), (0, 3));
}

#[test]
fn editor_down_on_the_last_row_stays_put() {
    let mut a = TextArea::editor("one\ntwo");
    press(&mut a, key(KeyCode::Left));
    assert_eq!(press(&mut a, key(KeyCode::Down)), EditorKey::Moved);
    assert_eq!(at(&a), (1, 2));
}

#[test]
fn editor_home_and_end_reach_the_ends_of_the_logical_line() {
    // One logical line wrapped at 6 columns: Home and End ignore the wrap.
    let mut a = TextArea::editor("first\nabcdefghij\nlast");
    press(&mut a, key(KeyCode::Up));
    assert_eq!(at(&a), (1, 4));
    assert_eq!(
        a.on_editor_key(key(KeyCode::End), 6, PAGE),
        EditorKey::Moved
    );
    assert_eq!(at(&a), (1, 10));
    a.on_editor_key(key(KeyCode::Home), 6, PAGE);
    assert_eq!(at(&a), (1, 0));
}

#[test]
fn editor_ctrl_a_and_ctrl_e_are_home_and_end_of_the_logical_line() {
    let mut a = TextArea::editor("first\nabcdefghij\nlast");
    press(&mut a, key(KeyCode::Up));
    assert_eq!(a.on_editor_key(ctrl('e'), 6, PAGE), EditorKey::Moved);
    assert_eq!(at(&a), (1, 10));
    assert_eq!(a.on_editor_key(ctrl('a'), 6, PAGE), EditorKey::Moved);
    assert_eq!(at(&a), (1, 0));
    assert_eq!(a.text(), "first\nabcdefghij\nlast", "nothing inserted");
}

#[test]
fn editor_pgdn_moves_by_the_visible_rows_less_one() {
    let mut a = lines(12);
    press(&mut a, ctrl_code(KeyCode::Home));
    press(&mut a, key(KeyCode::Right));
    assert_eq!(press(&mut a, key(KeyCode::PageDown)), EditorKey::Moved);
    assert_eq!(at(&a), (PAGE, 1), "four rows down, the same column");
    press(&mut a, key(KeyCode::PageDown));
    assert_eq!(at(&a), (2 * PAGE, 1));
    press(&mut a, key(KeyCode::PageDown));
    assert_eq!(at(&a), (11, 1), "stops on the last row");
    press(&mut a, key(KeyCode::PageDown));
    assert_eq!(at(&a), (11, 1));
    // Drawn rows, not logical lines: `abcdefghij` is two rows at 6 columns.
    let mut w = TextArea::editor("abcdefghij\nx\ny");
    press(&mut w, ctrl_code(KeyCode::Home));
    w.on_editor_key(key(KeyCode::PageDown), 6, 2);
    assert_eq!(at(&w), (1, 0));
}

#[test]
fn editor_pgup_moves_by_the_visible_rows_less_one() {
    let mut a = lines(12);
    assert_eq!(press(&mut a, key(KeyCode::PageUp)), EditorKey::Moved);
    assert_eq!(
        at(&a),
        (11 - PAGE, 5),
        "the same column, clamped to `line7`"
    );
    press(&mut a, key(KeyCode::PageUp));
    press(&mut a, key(KeyCode::PageUp));
    assert_eq!(at(&a), (0, 5), "stops on the first row, clamped");
    // A page of 0 (a one-row area) still moves a row.
    let mut b = lines(3);
    b.on_editor_key(key(KeyCode::PageUp), W, 0);
    assert_eq!(at(&b).0, 1);
}

#[test]
fn editor_ctrl_home_and_ctrl_end_reach_the_ends() {
    let mut a = lines(5);
    press(&mut a, key(KeyCode::Up));
    press(&mut a, key(KeyCode::Left));
    assert_eq!(press(&mut a, ctrl_code(KeyCode::Home)), EditorKey::Moved);
    assert_eq!(a.cursor(), 0);
    assert_eq!(press(&mut a, ctrl_code(KeyCode::End)), EditorKey::Moved);
    assert_eq!(a.cursor(), a.text().graphemes(true).count());
    assert_eq!(at(&a), (4, 5));
}

#[test]
fn editor_ctrl_k_cuts_the_current_logical_line() {
    let mut a = TextArea::editor("one\ntwo\nthree");
    press(&mut a, key(KeyCode::Up));
    assert_eq!(at(&a), (1, 3));
    assert_eq!(press(&mut a, ctrl('k')), EditorKey::Edited);
    assert_eq!(a.text(), "one\nthree", "the line and its newline");
    assert_eq!(at(&a), (1, 0), "the cursor at the start of the next line");
    // The last line has no newline of its own; nano's cut still holds one, and this
    // Ctrl-K right after the first appends to it.
    press(&mut a, ctrl('k'));
    assert_eq!(a.text(), "one\n");
    assert_eq!(at(&a), (1, 0));
    // An empty last line: nothing to cut.
    assert_eq!(press(&mut a, ctrl('k')), EditorKey::Moved);
    assert_eq!(a.text(), "one\n");
    press(&mut a, ctrl('u'));
    assert_eq!(a.text(), "one\ntwo\nthree\n", "both cuts, in order");
}

#[test]
fn consecutive_ctrl_k_append_and_another_key_starts_a_new_cut() {
    let mut a = TextArea::editor("a\nb\nc\nd");
    press(&mut a, ctrl_code(KeyCode::Home));
    press(&mut a, ctrl('k'));
    press(&mut a, ctrl('k'));
    assert_eq!(a.text(), "c\nd");
    // Paste the two lines back below `c`: one buffer, appended.
    press(&mut a, key(KeyCode::Down));
    press(&mut a, key(KeyCode::Home));
    assert_eq!(press(&mut a, ctrl('u')), EditorKey::Edited);
    assert_eq!(a.text(), "c\na\nb\nd");
    // Down came between the Ctrl-Ks and this one: it replaces the buffer.
    press(&mut a, ctrl_code(KeyCode::Home));
    press(&mut a, ctrl('k'));
    assert_eq!(a.text(), "a\nb\nd");
    press(&mut a, ctrl_code(KeyCode::End));
    press(&mut a, key(KeyCode::Enter));
    press(&mut a, ctrl('u'));
    assert_eq!(a.text(), "a\nb\nd\nc\n", "only `c`: the old cut is gone");
    // A Ctrl-U also ends a run of Ctrl-Ks: a new cut after it replaces the buffer.
    press(&mut a, ctrl_code(KeyCode::Home));
    press(&mut a, ctrl('k'));
    press(&mut a, ctrl('u'));
    press(&mut a, ctrl('k'));
    press(&mut a, ctrl('u'));
    press(&mut a, ctrl('u'));
    assert_eq!(a.text(), "a\nb\nb\nd\nc\n");
    // So does a key that is not the editor's (Tab to the options and back).
    press(&mut a, ctrl('k'));
    assert_eq!(press(&mut a, key(KeyCode::Tab)), EditorKey::Unhandled);
    press(&mut a, ctrl('k'));
    press(&mut a, ctrl_code(KeyCode::End));
    press(&mut a, ctrl('u'));
    // Appended, the buffer would be `d\nc\n`.
    assert_eq!(a.text(), "a\nb\nb\nc\n", "only `c`: Tab ended the run");
}

#[test]
fn ctrl_u_pastes_the_cut_at_the_cursor_within_the_cap() {
    let mut a = TextArea::editor("head\nbody");
    press(&mut a, ctrl_code(KeyCode::Home));
    press(&mut a, ctrl('k'));
    press(&mut a, key(KeyCode::Right));
    press(&mut a, key(KeyCode::Right));
    press(&mut a, ctrl('u'));
    assert_eq!(a.text(), "bohead\ndy", "at the cursor, mid-line");
    assert_eq!(at(&a), (1, 0), "the cursor after the pasted text");
    // An empty buffer pastes nothing.
    let mut e = TextArea::editor("x");
    assert_eq!(press(&mut e, ctrl('u')), EditorKey::Moved);
    assert_eq!(e.text(), "x");
    // Through the capped insert: a 6-character cap leaves room for one more.
    let mut c = TextArea::with_cap("ab\ncd", 6);
    press(&mut c, ctrl_code(KeyCode::Home));
    press(&mut c, ctrl('k'));
    assert_eq!(c.text(), "cd");
    press(&mut c, ctrl('u'));
    press(&mut c, ctrl('u'));
    assert_eq!(c.text(), "ab\nacd");
    assert_eq!(c.text().chars().count(), 6);
    assert!(c.at_cap(), "the second paste stopped at the cap");
}

#[test]
fn a_paste_keeps_its_breaks_and_drops_controls() {
    // Pinning: the same cleaning `TextArea::on_paste` does today.
    let mut a = TextArea::editor("");
    a.on_editor_paste("one\r\ntwo\x1b[31m\u{7}\rthree\tx\u{2028}y");
    assert_eq!(a.text(), "one\ntwo[31m\nthree x\ny");
    assert_eq!(a.cursor(), a.text().graphemes(true).count());
    let mut old = TextArea::new();
    old.on_paste("one\r\ntwo\x1b[31m\u{7}\rthree\tx\u{2028}y");
    assert_eq!(old.text(), a.text());
}

#[test]
fn a_hostile_paste_is_stored_without_its_carriers() {
    let mut a = TextArea::editor("");
    a.on_editor_paste("x\u{200D}y\u{202E}z");
    assert_eq!(a.text(), "xyz");
    // Typed one by one, the same.
    let mut t = TextArea::editor("");
    for c in "x\u{200D}y\u{202E}z".chars() {
        press(&mut t, key(KeyCode::Char(c)));
    }
    assert_eq!(t.text(), "xyz");
    // And every hostile character of the literal lists (the kept breaks aside).
    let mut h = TextArea::editor("");
    h.on_editor_paste(&crate::safe_text::tests::hostile_text());
    let stored = h.text().replace('\n', "");
    assert_eq!(crate::safe_text::tests::first_hostile(&stored), None);
}

#[test]
fn typing_and_pasting_stop_at_the_cap_and_say_so() {
    let cap = proto::GOAL_MAX_CHARS;
    let mut a = TextArea::editor(&"y".repeat(cap - 2));
    assert!(!a.at_cap());
    type_str(&mut a, "ab");
    assert_eq!(a.text().chars().count(), cap);
    assert!(!a.at_cap(), "reaching the cap exactly is not past it");
    assert_eq!(press(&mut a, key(KeyCode::Char('c'))), EditorKey::Moved);
    assert_eq!(a.text().chars().count(), cap);
    assert!(a.at_cap(), "a character past the cap");
    assert!(a.text().ends_with("ab"));
    // The next key that inserts nothing past the cap clears it.
    press(&mut a, key(KeyCode::Left));
    assert!(!a.at_cap());
    press(&mut a, key(KeyCode::Enter));
    assert!(a.at_cap(), "a newline past the cap");
    press(&mut a, key(KeyCode::Backspace));
    assert!(!a.at_cap());
    assert_eq!(a.text().chars().count(), cap - 1);
    // A paste stops at the cap too, keeping what fits.
    a.on_editor_paste("12345");
    assert_eq!(a.text().chars().count(), cap);
    assert!(a.at_cap());
    assert!(
        a.text().ends_with("y1b"),
        "{:?}",
        &a.text()[a.text().len() - 4..]
    );
    a.on_editor_paste("");
    assert!(!a.at_cap(), "a paste that fits clears it");
    // The editor's cap is the daemon's, in characters (not bytes): 4-byte characters.
    let mut w = TextArea::editor("");
    w.on_editor_paste(&"😀".repeat(cap + 1));
    assert_eq!(w.text().chars().count(), cap);
    assert!(w.at_cap());
    assert_eq!(w.limit(), cap);
    assert_eq!(TextArea::new().limit(), TEXT_MAX_CHARS);
}

#[test]
fn editor_tab_shift_tab_ctrl_s_and_esc_are_the_dialogs() {
    let mut a = TextArea::editor("goal");
    for k in [
        key(KeyCode::Tab),
        key(KeyCode::BackTab),
        ctrl('s'),
        key(KeyCode::Esc),
        ctrl('c'),
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
        key(KeyCode::F(2)),
    ] {
        assert_eq!(press(&mut a, k), EditorKey::Unhandled, "{k:?}");
    }
    assert_eq!(a.text(), "goal");
    assert_eq!(a.cursor(), 4);
}

#[test]
fn the_position_is_the_one_based_logical_line_and_grapheme_column() {
    let mut a = TextArea::editor("ab\nce\u{301}f");
    assert_eq!(a.position(), (2, 4));
    press(&mut a, key(KeyCode::Left));
    assert_eq!(a.position(), (2, 3), "the combining mark is one column");
    press(&mut a, ctrl_code(KeyCode::Home));
    assert_eq!(a.position(), (1, 1));
    assert_eq!(TextArea::editor("").position(), (1, 1));
    // A wrapped line is still one logical line.
    let mut w = TextArea::editor("abcdefghij");
    w.on_editor_key(key(KeyCode::Left), 6, PAGE);
    assert_eq!(w.position(), (1, 10));
}

#[test]
fn the_existing_text_area_keys_are_unchanged() {
    // Pinning: `on_key` and `on_key_in` keep their own keys; the editor's live only in
    // `on_editor_key` (decision 4). `text_area_tests.rs` and
    // `text_area_polish_tests.rs` pass untouched beside this.
    let mut a = TextArea::from_text("one\ntwo");
    assert!(!a.on_key(key(KeyCode::Enter)), "Enter is the form's");
    assert!(!a.on_key(ctrl('k')));
    assert!(!a.on_key(ctrl('u')));
    assert!(!a.on_key(ctrl('a')));
    assert!(!a.on_key(ctrl('e')));
    assert!(!a.on_key(key(KeyCode::PageUp)));
    assert_eq!(a.text(), "one\ntwo");
    assert_eq!(a.cursor(), 7);
    // Ctrl-Home is plain Home there: the logical line's start, not the text's.
    assert!(a.on_key(ctrl_code(KeyCode::Home)));
    assert_eq!(a.cursor(), 4);
    assert!(a.on_key(key(KeyCode::Up)));
    assert!(
        !a.on_key(key(KeyCode::Up)),
        "on the first row Up is the form's"
    );
    assert!(a.on_key(ctrl('j')));
    assert_eq!(a.text(), "\none\ntwo");
    // The new fields stay at their defaults on the old paths, so equality is as before.
    let mut b = TextArea::new();
    b.on_paste(&"y".repeat(TEXT_MAX_CHARS + 1));
    assert_eq!(b, TextArea::from_text(&"y".repeat(TEXT_MAX_CHARS)));
}
