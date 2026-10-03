//! Milestone 9.3 task 10a: the iterate dialog's model (decision 32): the editor's keys,
//! Ctrl-S's request, and Esc's confirm page (decision 8).

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const VIEW: EditorView = EditorView {
    width: 72,
    rows: 17,
};

fn key(form: &mut IterateForm, code: KeyCode, modifiers: KeyModifiers) -> IterateOutcome {
    form.on_key_in(KeyEvent::new(code, modifiers), VIEW)
}

fn typed(text: &str) -> IterateForm {
    let mut form = IterateForm::new("r-20261001-3f9a".into(), 2);
    form.on_paste_in(text, VIEW);
    form
}

#[test]
fn enter_is_a_newline_and_ctrl_s_sends_the_trimmed_request() {
    let mut form = typed("also add b");
    assert_eq!(
        key(&mut form, KeyCode::Enter, KeyModifiers::NONE),
        IterateOutcome::Stay
    );
    key(&mut form, KeyCode::Char('c'), KeyModifiers::NONE);
    // Tab and Shift-Tab do nothing: there are no options.
    key(&mut form, KeyCode::Tab, KeyModifiers::NONE);
    key(&mut form, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(form.text.text(), "also add b\nc");
    form.on_paste_in("  \n", VIEW);
    let want = RunRequest::Iterate {
        run: "r-20261001-3f9a".into(),
        goal: "also add b\nc".into(),
    };
    assert_eq!(
        key(&mut form, KeyCode::Char('s'), KeyModifiers::CONTROL),
        IterateOutcome::Submit(want)
    );
    assert!(form.submitting && form.error.is_none());
    // While submitting only Esc and Ctrl-C act: a second Ctrl-S sends nothing.
    assert_eq!(
        key(&mut form, KeyCode::Char('s'), KeyModifiers::CONTROL),
        IterateOutcome::Stay
    );
    assert_eq!(
        key(&mut form, KeyCode::Char('c'), KeyModifiers::CONTROL),
        IterateOutcome::Close
    );
}

#[test]
fn ctrl_s_on_a_blank_request_says_so() {
    let mut form = typed(" \n ");
    assert_eq!(
        key(&mut form, KeyCode::Char('s'), KeyModifiers::CONTROL),
        IterateOutcome::Stay
    );
    assert_eq!(form.error.as_deref(), Some(EMPTY_REQUEST));
    assert!(!form.submitting);
}

#[test]
fn esc_on_text_asks_and_only_a_plain_y_closes() {
    let mut empty = typed("");
    assert_eq!(
        key(&mut empty, KeyCode::Esc, KeyModifiers::NONE),
        IterateOutcome::Close
    );
    let mut form = typed("more");
    for (code, modifiers) in [
        (KeyCode::Char('Y'), KeyModifiers::SHIFT),
        (KeyCode::Char('y'), KeyModifiers::CONTROL),
        (KeyCode::Esc, KeyModifiers::NONE),
        (KeyCode::Char('s'), KeyModifiers::CONTROL),
    ] {
        assert_eq!(
            key(&mut form, KeyCode::Esc, KeyModifiers::NONE),
            IterateOutcome::Stay
        );
        assert!(form.discarding);
        // The page takes the paste nowhere.
        form.on_paste_in("x", VIEW);
        assert_eq!(key(&mut form, code, modifiers), IterateOutcome::Stay);
        assert!(!form.discarding && !form.submitting, "{code:?} goes back");
        assert_eq!(form.text.text(), "more");
    }
    key(&mut form, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(
        key(&mut form, KeyCode::Char('y'), KeyModifiers::NONE),
        IterateOutcome::Close
    );
}

#[test]
fn the_text_is_capped_in_characters() {
    let mut form = typed(&"é".repeat(proto::GOAL_MAX_CHARS + 5));
    assert_eq!(form.text.text().chars().count(), proto::GOAL_MAX_CHARS);
    assert!(form.text.at_cap());
    key(&mut form, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(form.text.text().chars().count(), proto::GOAL_MAX_CHARS);
}
