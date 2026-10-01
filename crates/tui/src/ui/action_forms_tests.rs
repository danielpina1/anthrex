//! Milestone 9.0.6 task 11: the input forms, drawn (decision 15; review focus 5).

use crate::actions_request::ActionTarget;
use crate::app::actions::ActionStep;
use crate::app::actions::forms::{ActionForm, Brief};
use crate::app::{App, Modal};
use crate::settings::UiSettings;
use crate::tree::run_fixtures::{RUN_ID, three_task_fixture};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{
    ActionInfo, ActionKind, BaseMovedInfo, BlockInfo, BlockReason, DaemonMsg, RunReply, TaskState,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

fn action(kind: ActionKind, label: &str) -> ActionInfo {
    ActionInfo {
        needs: kind.needs(),
        destructive: kind.destructive(),
        effect: format!("{label}: the effect"),
        label: label.into(),
        refused_why: None,
        kind,
    }
}

/// The three-task run with `t1` blocked on `question`, every input kind listed, and
/// the base moved when `moved`.
fn app_with_blocked_question(question: &str, moved: bool) -> App {
    let (mut snap, windows) = three_task_fixture();
    let run = &mut snap.runs[0];
    run.actions = vec![
        action(ActionKind::Resume, "resume halted run"),
        action(ActionKind::Promote, "promote"),
    ];
    if moved {
        run.base_moved = Some(BaseMovedInfo {
            from: "b".repeat(40),
            to: "a".repeat(40),
            commits: vec![],
            total: 3,
        });
    }
    let t1 = &mut run.tasks[1];
    t1.state = TaskState::Blocked;
    t1.block = Some(BlockInfo {
        reason: BlockReason::Question,
        text: question.into(),
    });
    t1.actions = vec![
        action(ActionKind::Answer, "answer"),
        action(ActionKind::Message, "message"),
        action(ActionKind::Override, "override"),
    ];
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn open(app: &mut App, target: ActionTarget, kind: ActionKind) {
    app.open_actions((RUN_ID.into(), target), Some(kind));
    press(app, KeyCode::Enter);
}

fn task_t1() -> ActionTarget {
    ActionTarget::Task("t1".into())
}

fn form_mut(app: &mut App) -> &mut ActionForm {
    match app.modal.as_mut() {
        Some(Modal::Action(flow)) => match &mut flow.step {
            ActionStep::Form(form) => form,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

fn draw(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn screen(app: &App, width: u16, height: u16) -> String {
    let buffer = draw(app, width, height);
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The dialog's rows (its frame included), trailing spaces trimmed.
fn dialog_rows(buffer: &Buffer, width: u16, height: u16) -> Vec<String> {
    let w = width.min(64);
    let x0 = (width - w) / 2;
    (0..height)
        .map(|y| {
            (x0..x0 + w)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .filter(|row| {
            row.starts_with('┌')
                || row.starts_with('│')
                || row.starts_with('└')
                || row.starts_with('+')
                || row.starts_with('|')
        })
        .map(|row| row.trim_end().to_string())
        .collect()
}

/// The dialog's interior lines, borders and padding stripped.
fn inner(rows: &[String]) -> Vec<String> {
    rows.iter()
        .filter(|r| r.starts_with('│') || r.starts_with('|'))
        .map(|r| {
            r.trim_start_matches(['│', '|'])
                .trim_end_matches(['│', '|'])
                .strip_prefix(' ')
                .unwrap_or("")
                .trim_end()
                .to_string()
        })
        .collect()
}

fn lines_of(app: &App, w: u16, h: u16) -> Vec<String> {
    inner(&dialog_rows(&draw(app, w, h), w, h))
}

const BLANKS: [&str; 5] = ["", "", "", "", ""];

fn rows(head: &[&str], tail: &[&str]) -> Vec<String> {
    head.iter().chain(tail).map(|s| s.to_string()).collect()
}

const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];

#[test]
fn the_answer_form_rows() {
    let mut app = app_with_blocked_question("which db?\nsecond line", false);
    open(&mut app, task_t1(), ActionKind::Answer);
    let hints = "⏎ continue · ^J newline · esc back";
    for (w, h) in SIZES {
        let rows_ = inner(&dialog_rows(&draw(&app, w, h), w, h));
        let top = dialog_rows(&draw(&app, w, h), w, h);
        assert_eq!(
            top[0], "┌ answer ──────────────────────────────────────────────────────┐",
            "{w}x{h}"
        );
        let mut want = rows(
            &[
                "task    t1 spawn",
                "asked   which db?",
                "        second line",
                "brief   loading…",
                "answer",
            ],
            &BLANKS,
        );
        want.push(String::new());
        want.push(hints.into());
        assert_eq!(rows_, want, "{w}x{h} loading");
    }
    if let ActionForm::Answer(f) = form_mut(&mut app) {
        f.brief = Brief::Ready(vec!["one".into(), "two".into(), "three".into()]);
    }
    type_text(&mut app, "hello");
    for (w, h) in SIZES {
        let mut want = rows(
            &[
                "task    t1 spawn",
                "asked   which db?",
                "        second line",
                "brief   one",
                "        two",
                "        three",
                "answer  hello",
            ],
            &BLANKS,
        );
        want.push(String::new());
        want.push(hints.into());
        assert_eq!(lines_of(&app, w, h), want, "{w}x{h} ready");
    }
    if let ActionForm::Answer(f) = form_mut(&mut app) {
        f.brief = Brief::Failed("no such task t1".into());
    }
    assert_eq!(lines_of(&app, 80, 24)[3], "brief   no such task t1");
    let failed = crate::theme::role(crate::theme::Role::Failed, app.palette()).fg;
    let buffer = draw(&app, 80, 24);
    let y = (0..24)
        .find(|&y| buffer[(18, y)].symbol() == "n" && buffer[(19, y)].symbol() == "o")
        .unwrap();
    assert_eq!(buffer[(18, y)].fg, failed.unwrap());
}

#[test]
fn a_long_question_is_cut_to_five_rows() {
    let question = (1..=9)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = app_with_blocked_question(&question, false);
    open(&mut app, task_t1(), ActionKind::Answer);
    let lines = lines_of(&app, 80, 24);
    assert_eq!(
        &lines[1..6],
        [
            "asked   line 1",
            "        line 2",
            "        line 3",
            "        line 4",
            "        line 5 …"
        ]
    );
    assert_eq!(lines[6], "brief   loading…");
}

#[test]
fn the_answer_box_wraps_at_60_columns() {
    let mut app = app_with_blocked_question("q", false);
    open(&mut app, task_t1(), ActionKind::Answer);
    app.on_paste(
        "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron".into(),
    );
    for (w, h) in SIZES {
        let lines = lines_of(&app, w, h);
        assert!(
            lines
                .iter()
                .all(|l| unicode_width::UnicodeWidthStr::width(l.as_str()) <= 60),
            "{lines:#?}"
        );
        let at = lines.iter().position(|l| l.starts_with("answer")).unwrap();
        assert_eq!(
            &lines[at..at + 3],
            [
                "answer  alpha beta gamma delta epsilon zeta eta theta iota",
                "        kappa lambda mu nu xi omicron",
                "",
            ],
            "{w}x{h}"
        );
    }
}

#[test]
fn a_long_answer_scrolls_with_marks_and_keeps_its_height() {
    let mut app = app_with_blocked_question("q", false);
    open(&mut app, task_t1(), ActionKind::Answer);
    let height = lines_of(&app, 80, 24).len();
    for i in 0..7 {
        type_text(&mut app, &format!("row {i}"));
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    }
    let lines = lines_of(&app, 80, 24);
    assert_eq!(lines.len(), height, "the dialog does not grow");
    let at = lines.iter().position(|l| l.starts_with("answer")).unwrap();
    assert_eq!(lines[at], "answer  ↑ 4 more");
    assert_eq!(lines[at + 1], "        row 4");
}

#[test]
fn the_message_form_rows() {
    let mut app = app_with_blocked_question("q", false);
    open(&mut app, task_t1(), ActionKind::Message);
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "hi");
    for (w, h) in SIZES {
        let mut want = rows(&["to      t1", "kind    ‹ change ›", "message hi"], &BLANKS);
        want.push(String::new());
        want.push("⏎ continue · tab kind · ^J newline · esc back".into());
        assert_eq!(lines_of(&app, w, h), want, "{w}x{h}");
    }
}

#[test]
fn the_override_form_rows() {
    let mut app = app_with_blocked_question("q", false);
    open(&mut app, task_t1(), ActionKind::Override);
    for (w, h) in SIZES {
        assert_eq!(
            lines_of(&app, w, h),
            ["reason", "", "", "", "", "⏎ continue · esc back"],
            "{w}x{h}"
        );
    }
    press(&mut app, KeyCode::Enter);
    let lines = lines_of(&app, 80, 24);
    assert_eq!(lines[4], "        type a reason first");
    let failed = crate::theme::role(crate::theme::Role::Failed, app.palette()).fg;
    let buffer = draw(&app, 80, 24);
    let y = (0..24)
        .find(|&y| buffer[(18, y)].symbol() == "t" && buffer[(19, y)].symbol() == "y")
        .unwrap();
    assert_eq!(buffer[(18, y)].fg, failed.unwrap());
}

#[test]
fn the_resume_form_rows() {
    let mut app = app_with_blocked_question("q", true);
    open(&mut app, ActionTarget::Run, ActionKind::Resume);
    for (w, h) in SIZES {
        assert_eq!(
            lines_of(&app, w, h),
            [
                "rebaseline  ‹ on ›",
                "base moved bbbbbbb → aaaaaaa (3 commits); rebaselining",
                "re-runs tier 3 against the new base",
                "",
                "⏎ continue · tab toggle · esc back",
            ],
            "{w}x{h}"
        );
    }
    press(&mut app, KeyCode::Tab);
    assert_eq!(lines_of(&app, 80, 24)[0], "rebaseline  ‹ off ›");
    let mut app = app_with_blocked_question("q", false);
    open(&mut app, ActionTarget::Run, ActionKind::Resume);
    assert_eq!(
        lines_of(&app, 80, 24),
        [
            "rebaseline  ‹ off ›",
            "",
            "⏎ continue · tab toggle · esc back"
        ]
    );
}

#[test]
fn the_promote_form_rows() {
    let mut app = app_with_blocked_question("q", false);
    open(&mut app, ActionTarget::Run, ActionKind::Promote);
    for (w, h) in SIZES {
        assert_eq!(
            lines_of(&app, w, h),
            [
                "orchestrator  ‹ configured ›",
                "",
                "⏎ continue · tab next · esc back"
            ],
            "{w}x{h}"
        );
    }
}

#[test]
fn a_forms_page_shows_what_was_typed() {
    let mut app = app_with_blocked_question("q", false);
    open(&mut app, task_t1(), ActionKind::Message);
    type_text(&mut app, "use the new api");
    press(&mut app, KeyCode::Enter);
    for (w, h) in SIZES {
        let rows_ = dialog_rows(&draw(&app, w, h), w, h);
        assert_eq!(
            rows_[0], "┌ message ─────────────────────────────────────────────────────┐",
            "{w}x{h}"
        );
        assert_eq!(
            inner(&rows_),
            [
                "message: the effect",
                "",
                "to       t1",
                "kind     info",
                "message  use the new api",
                "",
                "⏎ message · esc back",
            ],
            "{w}x{h}"
        );
    }
}

#[test]
fn forms_draw_in_ascii_mode() {
    let mut app = app_with_blocked_question("q", true);
    app.settings.badges.ascii = true;
    open(&mut app, ActionTarget::Run, ActionKind::Resume);
    let rows_ = dialog_rows(&draw(&app, 80, 24), 80, 24);
    assert!(rows_.iter().all(|r| r.is_ascii()), "{rows_:#?}");
    assert_eq!(
        inner(&rows_),
        [
            "rebaseline  < on >",
            "base moved bbbbbbb -> aaaaaaa (3 commits); rebaselining",
            "re-runs tier 3 against the new base",
            "",
            "enter continue - tab toggle - esc back",
        ]
    );
    press(&mut app, KeyCode::Esc);
    let mut app = app_with_blocked_question("which db?", false);
    app.settings.badges.ascii = true;
    open(&mut app, task_t1(), ActionKind::Answer);
    for i in 0..7 {
        type_text(&mut app, &format!("row {i}"));
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    }
    let rows_ = dialog_rows(&draw(&app, 80, 24), 80, 24);
    assert!(rows_.iter().all(|r| r.is_ascii()), "{rows_:#?}");
    assert!(rows_.iter().any(|r| r.contains("^ 4 more")), "{rows_:#?}");
}

/// Review focus 5: agent text in the menu and the forms renders inert.
#[test]
fn menu_and_forms_render_no_hostile_character() {
    let hostile = crate::safe_text::tests::hostile_text();
    let mut app = app_with_blocked_question(&hostile, true);
    // The menu itself: a hostile title, label and refusal.
    {
        let (mut snap, _) = three_task_fixture();
        let run = &mut snap.runs[0];
        run.goal = hostile.clone();
        let t1 = &mut run.tasks[1];
        t1.title = hostile.clone();
        t1.state = TaskState::Blocked;
        t1.block = Some(BlockInfo {
            reason: BlockReason::Question,
            text: hostile.clone(),
        });
        let mut refused = action(ActionKind::Retry, &hostile);
        refused.refused_why = Some(hostile.clone());
        t1.actions = vec![
            action(ActionKind::Answer, "answer"),
            action(ActionKind::Message, "message"),
            action(ActionKind::Override, "override"),
            refused,
        ];
        run.actions = vec![
            action(ActionKind::Resume, &hostile),
            action(ActionKind::Promote, "promote"),
        ];
        run.base_moved = Some(BaseMovedInfo {
            from: format!("a{}b", '\u{202e}'),
            to: format!("\u{1b}[2J{}", "c".repeat(9)),
            commits: vec![],
            total: 1,
        });
        app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    }
    let clean = |app: &App, what: &str| {
        // The buffer drops control characters on its own, so the spans are checked too.
        if let Some(Modal::Action(flow)) = &app.modal
            && let ActionStep::Form(form) = &flow.step
        {
            for line in crate::ui::action_forms::body(form, 64, app.palette()) {
                for span in line.spans {
                    let found = crate::safe_text::tests::first_hostile(&span.content);
                    assert_eq!(found, None, "{what}: {:?}", span.content);
                }
            }
        }
        for (w, h) in SIZES {
            let text = screen(app, w, h);
            assert_eq!(
                crate::safe_text::tests::first_hostile(&text),
                None,
                "{what} {w}x{h}"
            );
        }
    };
    app.open_actions((RUN_ID.into(), task_t1()), Some(ActionKind::Retry));
    clean(&app, "menu");
    // The answer form, its brief (ready, then refused), what was pasted, its page.
    app.open_actions((RUN_ID.into(), task_t1()), Some(ActionKind::Answer));
    press(&mut app, KeyCode::Enter);
    clean(&app, "answer, loading");
    if let ActionForm::Answer(f) = form_mut(&mut app) {
        f.brief = Brief::Ready(vec![hostile.clone(), hostile.clone(), hostile.clone()]);
    }
    app.on_paste(hostile.clone());
    clean(&app, "answer, ready");
    if let ActionForm::Answer(f) = form_mut(&mut app) {
        f.brief = Brief::Failed(hostile.clone());
    }
    clean(&app, "answer, refused");
    press(&mut app, KeyCode::Enter);
    clean(&app, "answer page");
    // Override, message, resume (hostile shas) and promote (a hostile model name).
    for kind in [ActionKind::Override, ActionKind::Message] {
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        app.open_actions((RUN_ID.into(), task_t1()), Some(kind));
        press(&mut app, KeyCode::Enter);
        app.on_paste(hostile.clone());
        clean(&app, "form");
        press(&mut app, KeyCode::Enter);
        clean(&app, "page");
    }
    app.open_actions((RUN_ID.into(), ActionTarget::Run), Some(ActionKind::Resume));
    press(&mut app, KeyCode::Enter);
    clean(&app, "resume");
    press(&mut app, KeyCode::Enter);
    clean(&app, "resume page");
    app.open_actions(
        (RUN_ID.into(), ActionTarget::Run),
        Some(ActionKind::Promote),
    );
    press(&mut app, KeyCode::Enter);
    if let ActionForm::Promote(f) = form_mut(&mut app) {
        f.options
            .push(crate::app::actions::forms::PromoteOption::roster(
                proto::Runtime::Codex,
                &hostile,
            ));
        f.at = 1;
    }
    clean(&app, "promote");
    press(&mut app, KeyCode::Enter);
    clean(&app, "promote page");
}
