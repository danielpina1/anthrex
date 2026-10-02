//! M9.0.7.13: the goal field's placeholder and the task edit form's brief as a text
//! area (decision 35), reached by the keys a user presses.

use super::super::gate::{form as edit_form, gate, open_form as open_edit, tap};
use super::super::goal_form::{app_with_cache, open_form as open_goal, roster};
use super::super::*;
use crate::run_edit::EditField;
use crate::theme::{Palette, Role, role};
use crate::ui::audit;
use proto::{PlanEdit, RunRequest};

fn draw(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    audit::draw(app, w, h)
}

fn muted() -> ratatui::style::Color {
    role(Role::Muted, Palette::PLAIN).fg.expect("a colour")
}

const PLACEHOLDER: &str = "what should the run achieve?";

#[test]
fn the_goal_field_shows_its_placeholder_while_empty() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = app_with_cache(roster());
        app.set_terminal_size(w, h);
        open_goal(&mut app);
        let buffer = draw(&app, w, h);
        let found = audit::find(&buffer, PLACEHOLDER);
        let &(x, y) = found
            .first()
            .unwrap_or_else(|| panic!("{w}x{h}:\n{}", audit::rows(&buffer).join("\n")));
        // On the goal's first row, after its label; muted past the cursor's cell.
        assert!(audit::rows(&buffer)[usize::from(y)].contains("goal"));
        assert_eq!(buffer[(x + 1, y)].fg, muted(), "{w}x{h}");

        assert!(press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE).is_empty());
        let buffer = draw(&app, w, h);
        assert!(
            audit::find(&buffer, PLACEHOLDER).is_empty(),
            "gone once typed"
        );
        assert!(!audit::find(&buffer, "goal              x").is_empty());
    }
    // In ASCII too.
    let mut app = app_with_cache(roster());
    app.settings.badges.ascii = true;
    app.set_terminal_size(80, 24);
    open_goal(&mut app);
    let buffer = draw(&app, 80, 24);
    assert!(!audit::find(&buffer, PLACEHOLDER).is_empty());
}

/// The brief's four text rows in the value column, the first on the label's row.
fn brief_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    let rows = audit::rows(buffer);
    let at = rows
        .iter()
        .position(|r| r.contains("brief      "))
        .unwrap_or_else(|| panic!("no brief row:\n{}", rows.join("\n")));
    let byte = rows[at].find("brief").unwrap();
    // The label column is 11 wide (`ui::dialog::LABEL_WIDTH`).
    let column = rows[at][..byte].chars().count() + 11;
    rows[at..at + 4]
        .iter()
        .map(|r| {
            // The value column: the dialog's 60 interior columns less the label's 13.
            let cells: String = r.chars().skip(column).take(47).collect();
            cells.trim_end().to_string()
        })
        .collect()
}

#[test]
fn the_edit_brief_is_four_rows_and_sends_newlines() {
    let mut app = gate();
    open_edit(&mut app, "t1");
    super::super::gate::focus(&mut app, EditField::Brief);
    let buffer = draw(&app, 80, 24);
    assert_eq!(brief_rows(&buffer), vec!["Line one", "Line two", "", ""]);
    assert!(audit::find(&buffer, "↵").is_empty(), "no newline mark");

    // Ctrl-J is a newline; three more lines fill and scroll the four rows.
    for c in ['x', 'y', 'z'] {
        assert!(press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL).is_empty());
        assert!(tap(&mut app, KeyCode::Char(c)).is_empty());
    }
    let buffer = draw(&app, 80, 24);
    let rows = audit::rows(&buffer).join("\n");
    assert!(rows.contains("↑ 1 more"), "{rows}");
    assert_eq!(brief_rows(&buffer)[1..], ["x", "y", "z"], "{rows}");

    // Up moves a row inside the brief, not to the field above.
    assert!(tap(&mut app, KeyCode::Up).is_empty());
    assert_eq!(edit_form(&app).focus, EditField::Brief);
    let effects = tap(&mut app, KeyCode::Enter);
    let [
        Effect::Send(proto::ClientMsg::RunTagged {
            request: RunRequest::Edit { edits, .. },
            ..
        }),
    ] = &effects[..]
    else {
        panic!("one edit: {effects:?}");
    };
    let [PlanEdit::AmendTask { brief, .. }] = &edits[..] else {
        panic!("one amend: {edits:?}");
    };
    assert_eq!(brief.as_deref(), Some("Line one\nLine two\nx\ny\nz"));
}

#[test]
fn edit_choices_have_ascii_twins() {
    let mut app = gate();
    app.settings.badges =
        crate::ui::badge::BadgeSet::from_config(&app.settings.badges_config, true);
    open_edit(&mut app, "t1");
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = draw(&app, w, h);
        let rows = audit::rows(&buffer).join("\n");
        assert_eq!(audit::first_non_ascii(&buffer), None, "{rows}");
        assert!(rows.contains("> runtime    < claude >"), "{rows}");
        assert!(rows.contains("  effort     < medium >"), "{rows}");
        assert!(rows.contains("enter save - tab next"), "{rows}");
    }
    let ascii = Palette {
        ascii: true,
        ..Palette::PLAIN
    };
    let form = edit_form(&app);
    assert_eq!(
        form.value_parts_in(EditField::Size, ascii),
        ("< M >".to_string(), None)
    );
    assert_eq!(
        form.value_parts_in(EditField::Size, Palette::PLAIN),
        ("‹ M ›".to_string(), None)
    );
}

/// The one tagged request of `effects`.
fn one_tagged(effects: &[Effect]) -> (u64, RunRequest) {
    match effects {
        [Effect::Send(proto::ClientMsg::RunTagged { id, request })] => (*id, request.clone()),
        other => panic!("one tagged request: {other:?}"),
    }
}

/// An empty session (no window, no run) started in `dir`.
fn empty_session(dir: &str) -> App {
    let mut app = App::new(vec![], dir.into(), UiSettings::default());
    let _ = app.set_terminal_size(80, 24);
    let snap = crate::tree::run_fixtures::snapshot(3_460, vec![]);
    app.on_daemon(DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    app
}

/// Decision 37 (the user's 9.0.6 try-out): with no selection and no focused window,
/// `C-b g` and `C-b P` open on the TUI's start directory. The client probes nothing: the
/// form's submit carries the directory and the daemon's own refusal (`not a git
/// repository: …`) is the form's error row, as the Profile screen's error row shows its
/// refused status. A selected project or a focused window still wins; an empty start
/// directory still toasts `NO_PROJECT`.
#[test]
fn an_empty_session_uses_the_start_directory() {
    let repo = std::path::PathBuf::from("/tmp/repo");
    let mut app = empty_session("/tmp/repo");
    assert!(app.windows.is_empty() && app.runs.runs.is_empty());
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Char('g')).is_empty());
    let Some(Modal::StartGoal(form)) = &app.modal else {
        panic!("no goal form: {:?} {:?}", app.modal, app.toast_text());
    };
    assert_eq!(form.project, repo);
    assert_eq!(app.toast_text(), None);
    for c in "add a readme".chars() {
        tap(&mut app, KeyCode::Char(c));
    }
    let (id, request) = one_tagged(&tap(&mut app, KeyCode::Enter));
    let RunRequest::StartGoal { dir, goal, .. } = &request else {
        panic!("{request:?}");
    };
    assert_eq!((dir, goal.as_str()), (&repo, "add a readme"));
    let refusal = "not a git repository: /tmp/repo";
    app.on_daemon(DaemonMsg::Run(proto::RunReply::Refused {
        request: proto::run_wire::request::START_GOAL.into(),
        message: refusal.into(),
        request_id: Some(id),
    }));
    let Some(Modal::StartGoal(form)) = &app.modal else {
        panic!("the form closed: {:?}", app.modal);
    };
    assert_eq!(form.error.as_deref(), Some(refusal));
    assert!(!form.submitting);
    let buffer = draw(&app, 80, 24);
    assert!(
        !audit::find(&buffer, refusal).is_empty(),
        "{}",
        audit::rows(&buffer).join("\n")
    );
    assert_eq!(app.toast_text(), None, "the form's row, not a toast");
    tap(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, None);

    // `C-b P`: the Profile screen on it, with its three requests.
    prefix(&mut app);
    let effects = tap(&mut app, KeyCode::Char('P'));
    let asked: Vec<RunRequest> = (effects.iter())
        .filter_map(|e| match e {
            Effect::Send(proto::ClientMsg::RunTagged { request, .. }) => Some(request.clone()),
            _ => None,
        })
        .collect();
    let profile = |request| RunRequest::Profile(request);
    assert_eq!(
        asked,
        vec![
            profile(proto::ProfileRequest::Status { dir: repo.clone() }),
            profile(proto::ProfileRequest::Show {
                dir: repo.clone(),
                proposed: false,
            }),
            profile(proto::ProfileRequest::Show {
                dir: repo.clone(),
                proposed: true,
            }),
        ]
    );
    let status_id = tagged_ids(&effects)[0];
    app.on_daemon(DaemonMsg::Run(proto::RunReply::Profile {
        reply: Box::new(proto::ProfileReply::Refused {
            message: refusal.into(),
        }),
        request_id: Some(status_id),
    }));
    let buffer = draw(&app, 80, 24);
    assert!(
        !audit::find(&buffer, refusal).is_empty(),
        "{}",
        audit::rows(&buffer).join("\n")
    );
    tap(&mut app, KeyCode::Esc);
    assert!(app.screen.is_none());

    // A focused window's project still wins.
    let mut app = App::new(
        vec![project_win(1, "/p/a")],
        "/tmp/repo".into(),
        UiSettings::default(),
    );
    let _ = app.set_terminal_size(80, 24);
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('g'));
    let Some(Modal::StartGoal(form)) = &app.modal else {
        panic!("no goal form");
    };
    assert_eq!(form.project, std::path::PathBuf::from("/p/a"));
    // A selected project too, over the focused window's.
    let mut app = App::new(project_windows(), "/tmp/repo".into(), UiSettings::default());
    let _ = app.set_terminal_size(80, 24);
    let focused = app
        .focused_window()
        .expect("a focused window")
        .project
        .clone();
    let other = if focused == std::path::Path::new("/p/a") {
        "/p/b"
    } else {
        "/p/a"
    };
    let rows = crate::tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree
        .select(&rows, crate::tree::NodeKey::Project(other.into()));
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('P'));
    match &app.screen {
        Some(crate::app::screens::Screen::Profile(s)) => {
            assert_eq!(s.dir, std::path::PathBuf::from(other));
        }
        other => panic!("{other:?}"),
    }

    // An empty start directory still says there is no project.
    let mut app = empty_session("");
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Char('g')).is_empty());
    assert_eq!(app.modal, None);
    assert_eq!(app.toast_text(), Some(crate::run_goal::NO_PROJECT));
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Char('P')).is_empty());
    assert!(app.screen.is_none());
    assert_eq!(
        app.toast_text(),
        Some(crate::app::profile_screen::NO_PROJECT)
    );
}

fn tagged_ids(effects: &[Effect]) -> Vec<u64> {
    (effects.iter())
        .filter_map(|e| match e {
            Effect::Send(proto::ClientMsg::RunTagged { id, .. }) => Some(*id),
            _ => None,
        })
        .collect()
}

/// Decision 37 brings a new source of drawn text, the start directory (any path the
/// user stood in): the goal form's title, the Profile screen's title and its Reject page
/// draw it sanitised. The hostile characters sit inside the drawn columns.
#[test]
fn a_hostile_start_directory_is_drawn_sanitised() {
    let hidden = |buffer: &ratatui::buffer::Buffer| {
        audit::rows(buffer)
            .iter()
            .any(|r| r.contains(['\u{200D}', '\u{202E}']))
    };
    let mut app = empty_session("/tmp/x\u{200D}y\u{202E}z");
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('g'));
    let buffer = draw(&app, 80, 24);
    let rows = audit::rows(&buffer).join("\n");
    assert!(rows.contains("start a goal in /tmp/xyz"), "{rows}");
    assert!(!hidden(&buffer), "{rows}");
    tap(&mut app, KeyCode::Esc);
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('P'));
    let buffer = draw(&app, 80, 24);
    let rows = audit::rows(&buffer).join("\n");
    assert!(rows.contains("profile · xyz"), "{rows}");
    assert!(!hidden(&buffer), "{rows}");
    tap(&mut app, KeyCode::Char('x'));
    let buffer = draw(&app, 80, 24);
    let rows = audit::rows(&buffer).join("\n");
    assert!(rows.contains("the proposal for /tmp/xyz is"), "{rows}");
    assert!(!hidden(&buffer), "{rows}");
}

/// Decision 35 (fix round 1, m5): a brief round-trips byte for byte through an edit:
/// CJK, trailing spaces and blank lines kept. Hidden format characters (a joiner, a
/// right-to-left mark) are dropped on edit, as M8c meant; a tab becomes a space.
#[test]
fn an_edited_brief_round_trips_byte_for_byte() {
    use crate::run_edit::TaskEditForm;
    use crate::run_edit::tests::edit_fixture_task;
    let brief = "改善する  \n\n  末尾の空白  \n  last line ";
    let mut t = edit_fixture_task();
    t.brief = brief.into();
    let mut form = TaskEditForm::new(crate::tree::run_fixtures::RUN_ID, &t);
    assert_eq!(form.brief.text(), brief, "opens whole");
    assert_eq!(form.edits(), Ok(vec![]), "an untouched brief is not sent");
    while form.focus != EditField::Brief {
        form.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    form.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    form.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    let [PlanEdit::AmendTask { brief: sent, .. }] = &form.edits().unwrap()[..] else {
        panic!("one amend");
    };
    assert_eq!(sent.as_deref(), Some(format!("{brief}\nx").as_str()));

    t.brief = "a\u{200D}b\u{200F}c\td".into();
    let mut form = TaskEditForm::new(crate::tree::run_fixtures::RUN_ID, &t);
    assert_eq!(form.brief.text(), "abc d");
    assert_eq!(
        form.edits(),
        Ok(vec![]),
        "untouched, the plan's own is kept"
    );
    form.on_paste("!");
    assert_eq!(
        form.edits(),
        Ok(vec![]),
        "a paste elsewhere changes nothing"
    );
    while form.focus != EditField::Brief {
        form.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    form.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    let [PlanEdit::AmendTask { brief: sent, .. }] = &form.edits().unwrap()[..] else {
        panic!("one amend");
    };
    assert_eq!(sent.as_deref(), Some("abc dx"));
}

/// Decision 35 (fix round 1, R2): a brief opens whole up to `BRIEF_MAX_CHARS`, past
/// the old 16,384-character field bound, and is sent whole.
#[test]
fn a_long_brief_opens_and_is_sent_whole() {
    use crate::run_edit::{TEXT_MAX_CHARS, TaskEditForm};
    let mut t = crate::run_edit::tests::edit_fixture_task();
    let brief = "word \n".repeat(TEXT_MAX_CHARS);
    t.brief = brief.clone();
    let mut form = TaskEditForm::new(crate::tree::run_fixtures::RUN_ID, &t);
    assert_eq!(form.brief.text(), brief);
    assert_eq!(form.error, None);
    while form.focus != EditField::Brief {
        form.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    form.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    let [PlanEdit::AmendTask { brief: sent, .. }] = &form.edits().unwrap()[..] else {
        panic!("one amend");
    };
    assert_eq!(sent.as_deref(), Some(format!("{brief}x").as_str()));
}

/// Decision 35 (fix round 1, R2): a brief past `BRIEF_MAX_CHARS` opens cut with an
/// error row, its other fields still edit, and an edit of it is refused: a cut brief
/// is never sent back.
#[test]
fn a_brief_past_the_cap_is_never_sent_cut() {
    use crate::run_edit::{BRIEF_MAX_CHARS, BRIEF_TOO_LONG, EditOutcome, TaskEditForm};
    let mut t = crate::run_edit::tests::edit_fixture_task();
    t.brief = "z".repeat(BRIEF_MAX_CHARS + 10);
    let mut form = TaskEditForm::new(crate::tree::run_fixtures::RUN_ID, &t);
    assert_eq!(form.error.as_deref(), Some(BRIEF_TOO_LONG));
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    while form.focus != EditField::Size {
        form.on_key(key(KeyCode::Tab));
    }
    form.on_key(key(KeyCode::Right));
    let [PlanEdit::AmendTask { brief, size, .. }] = &form.edits().unwrap()[..] else {
        panic!("one amend");
    };
    assert_eq!(
        (brief, size),
        (&None, &Some(proto::Size::S)),
        "the size alone"
    );
    while form.focus != EditField::Brief {
        form.on_key(key(KeyCode::Tab));
    }
    form.on_key(key(KeyCode::Backspace));
    assert_eq!(form.on_key(key(KeyCode::Enter)), EditOutcome::Stay);
    assert_eq!(form.error.as_deref(), Some(BRIEF_TOO_LONG));
    assert!(!form.submitting, "nothing sent");
}
