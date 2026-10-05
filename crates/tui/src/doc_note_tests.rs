//! Milestone 9.6 task 17: the gate's note editor, pure.

use super::*;
use proto::DocGateKind;

fn view() -> EditorView {
    EditorView { width: 40, rows: 6 }
}

fn press(form: &mut DocNoteForm, code: KeyCode) -> NoteOutcome {
    form.on_key_in(KeyEvent::new(code, KeyModifiers::NONE), view())
}

fn ctrl(form: &mut DocNoteForm, c: char) -> NoteOutcome {
    form.on_key_in(
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
        view(),
    )
}

fn form(purpose: NoteFor, text: &str) -> DocNoteForm {
    DocNoteForm::new("r-3f9a".into(), (DocGateKind::Spec, 2), purpose, text)
}

#[test]
fn changes_send_their_note_and_review_flag() {
    let mut f = form(NoteFor::Changes, "");
    for c in "  split R1 ".chars() {
        press(&mut f, KeyCode::Char(c));
    }
    assert!(!f.review, "decision 15's default is no review");
    press(&mut f, KeyCode::Tab);
    let out = ctrl(&mut f, 's');
    let changes = DocGateAction::Changes {
        note: "split R1".into(),
        review: true,
    };
    assert_eq!(out, NoteOutcome::Send(changes));
    assert!(f.submitting);
    assert_eq!(press(&mut f, KeyCode::Char('x')), NoteOutcome::Stay);
    assert_eq!(
        f.text.text(),
        "  split R1 ",
        "nothing types while submitting"
    );
}

#[test]
fn rethink_and_back_ask_their_page_and_tab_is_only_the_changes_toggle() {
    let mut f = form(NoteFor::Rethink, "");
    press(&mut f, KeyCode::Tab);
    assert!(!f.review);
    assert_eq!(
        ctrl(&mut f, 's'),
        NoteOutcome::Confirm(DocGateAction::Rethink {
            note: String::new()
        })
    );
    let mut f = form(NoteFor::Back, "why");
    assert_eq!(
        ctrl(&mut f, 's'),
        NoteOutcome::Confirm(DocGateAction::Back { note: "why".into() })
    );
}

#[test]
fn an_empty_edit_is_not_sent_and_a_changed_text_asks_before_esc() {
    let mut f = form(NoteFor::Edit, "");
    assert_eq!(ctrl(&mut f, 's'), NoteOutcome::Stay);
    assert_eq!(f.error.as_deref(), Some(EMPTY_EDIT));
    let mut f = form(NoteFor::Edit, "# Spec");
    assert_eq!(press(&mut f, KeyCode::Esc), NoteOutcome::Close, "unchanged");
    let mut f = form(NoteFor::Edit, "# Spec");
    press(&mut f, KeyCode::Char('!'));
    assert_eq!(press(&mut f, KeyCode::Esc), NoteOutcome::Stay);
    assert!(f.discarding);
    assert_eq!(press(&mut f, KeyCode::Char('n')), NoteOutcome::Stay);
    assert!(!f.discarding);
    press(&mut f, KeyCode::Esc);
    assert_eq!(press(&mut f, KeyCode::Char('y')), NoteOutcome::Close);
    // The edit keeps its text whole, up to 64 KiB.
    let big = "a".repeat(EDIT_MAX_CHARS + 10);
    let f = form(NoteFor::Edit, &big);
    assert_eq!(f.text.text().chars().count(), EDIT_MAX_CHARS);
}

#[test]
fn the_report_questions_are_its_section() {
    let report =
        "# R\n\n## Questions for you\n- one?\n- two?\n\n## Appendix: the drafts\n### claude\nx";
    assert_eq!(report_questions(report), "- one?\n- two?");
    assert_eq!(report_questions("# R\n\n## Approaches\nx"), "");
}
