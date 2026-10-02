//! M9.0.7.13: the old dialogs on the kit's grammar (decision 35): `kit::dialog_frame`
//! and `kit::dialog_area`, lower-case labels, `‹ value ›` choices, `⏎ action · tab next
//! · esc cancel`, destructive titles and verbs in `Failed`, ASCII twins, one accented
//! frame, sanitised text.

use super::*;
use crate::app::prompt::RenamePrompt;
use crate::app::{App, Modal};
use crate::dialog::{FormDefaults, NewAgentForm, RemoveConfirm, TextInput};
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::settings::UiSettings;
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::{Terminal, backend::TestBackend};

/// Visible hostile characters at the front of a text, inside the drawn columns: a
/// zero-width joiner and a right-to-left override, which ratatui draws as given.
const PLANT: &str = "x\u{200D}y\u{202E}z";

/// The audit's dialog fixtures (`ui/audit.rs`), each with the hint its row keeps.
const DIALOGS: &[(&str, Option<&str>)] = &[
    ("new agent over the pane", Some("esc cancel")),
    ("remove over the pane", Some("esc cancel")),
    ("force remove over the pane", None),
    ("rename over the pane", Some("esc cancel")),
    ("config notice over the pane", Some("esc close")),
    ("edit form over the run view", Some("esc cancel")),
];

fn ascii() -> Palette {
    Palette {
        ascii: true,
        ..Palette::PLAIN
    }
}

fn paint(width: u16, height: u16, f: impl FnOnce(&mut Frame)) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(f).unwrap();
    terminal.backend().buffer().clone()
}

fn fixture(name: &str) -> App {
    audit::fixtures()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no audit fixture {name}"))
        .1
}

fn row(buffer: &Buffer, y: u16, from: u16, to: u16) -> String {
    (from..to).map(|x| buffer[(x, y)].symbol()).collect()
}

/// The dialog titled `title`: its top border row, and its interior rows inside the
/// border and the one column of padding, trailing blanks trimmed.
fn dialog(buffer: &Buffer, title: &str, p: Palette) -> (String, Vec<String>) {
    let (corner, right, bottom) = if p.ascii {
        ("+", "+", "+")
    } else {
        ("┌", "┐", "└")
    };
    let found = audit::find(buffer, &format!("{corner} {title} "));
    let &(x, y) = found
        .first()
        .unwrap_or_else(|| panic!("no {title:?} dialog:\n{}", audit::rows(buffer).join("\n")));
    let end = (x + 1..buffer.area.right())
        .find(|&c| buffer[(c, y)].symbol() == right)
        .expect("the top-right corner");
    let last = (y + 1..buffer.area.bottom())
        .find(|&r| buffer[(x, r)].symbol() == bottom)
        .expect("the bottom-left corner");
    let top = row(buffer, y, x, end + 1);
    let rows = (y + 1..last)
        .map(|r| row(buffer, r, x + 2, end - 1).trim_end().to_string())
        .collect();
    (top, rows)
}

fn fg(role_: Role) -> Color {
    role(role_, Palette::PLAIN).fg.expect("a colour")
}

/// The foreground of the cell where `text` starts.
fn colour_at(buffer: &Buffer, text: &str) -> Color {
    let &(x, y) = audit::find(buffer, text)
        .first()
        .unwrap_or_else(|| panic!("{text:?} not drawn:\n{}", audit::rows(buffer).join("\n")));
    buffer[(x, y)].fg
}

fn form() -> NewAgentForm {
    NewAgentForm::new(&FormDefaults {
        runtime: proto::Runtime::Claude,
        dir: "/work".into(),
        model: String::new(),
    })
}

fn new_agent(form: &NewAgentForm, w: u16, h: u16, p: Palette) -> Buffer {
    paint(w, h, |f| render_new_agent(f, form, f.area(), p))
}

fn remove(confirm: &RemoveConfirm, w: u16, h: u16, p: Palette) -> Buffer {
    paint(w, h, |f| render_remove_confirm(f, confirm, f.area(), p))
}

fn modal_app(modal: Modal) -> App {
    let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
    app.modal = Some(modal);
    app
}

fn modal(app: &App, w: u16, h: u16) -> Buffer {
    paint(w, h, |f| crate::ui::modal::render(f, app, f.area()))
}

#[test]
fn the_new_agent_form_speaks_the_grammar() {
    let mut form = form();
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = new_agent(&form, w, h, Palette::PLAIN);
        let (top, rows) = dialog(&buffer, "new agent", Palette::PLAIN);
        assert_eq!(top.chars().count(), 64, "at most 64 wide, {w}x{h}: {top}");
        assert!(
            top.starts_with("┌ new agent ─") && top.ends_with('┐'),
            "{top}"
        );
        assert_eq!(
            rows,
            vec![
                "▌ runtime    ‹ claude ›",
                "  name       automatic (claude-N)",
                "  directory  /work",
                "  worktree   ‹ off › create a git worktree",
                "  model",
                "  prompt",
                "",
                "⏎ create · tab next · esc cancel",
            ],
            "{w}x{h}"
        );
        assert_eq!(colour_at(&buffer, "⏎ create"), fg(Role::Accent));
        assert_eq!(colour_at(&buffer, "esc cancel"), fg(Role::Accent));
        assert_eq!(colour_at(&buffer, "automatic"), fg(Role::Muted));
    }

    form.worktree = true;
    form.branch = TextInput::new("feat/x");
    form.error = Some("name taken".into());
    let buffer = new_agent(&form, 80, 24, Palette::PLAIN);
    let (_, rows) = dialog(&buffer, "new agent", Palette::PLAIN);
    assert_eq!(rows[3], "  worktree   ‹ on › create a git worktree");
    assert_eq!(rows[4], "  branch     feat/x");
    assert_eq!(rows[7], "✗ name taken");
    assert_eq!(colour_at(&buffer, "✗ name taken"), fg(Role::Failed));
    assert_eq!(rows[8], "");
    assert_eq!(rows[9], "⏎ create · tab next · esc cancel");

    // A long error wraps at 60 columns, at most three lines.
    form.error = Some("word ".repeat(60));
    let (_, rows) = dialog(
        &new_agent(&form, 120, 40, Palette::PLAIN),
        "new agent",
        Palette::PLAIN,
    );
    let errors: Vec<_> = rows.iter().filter(|r| r.contains("word")).collect();
    assert_eq!(errors.len(), 3, "{rows:#?}");
    assert!(
        errors.iter().all(|r| r.chars().count() <= 60),
        "{errors:#?}"
    );

    form.error = None;
    form.submitting = true;
    let (_, rows) = dialog(
        &new_agent(&form, 80, 24, Palette::PLAIN),
        "new agent",
        Palette::PLAIN,
    );
    assert_eq!(rows.last().unwrap(), "creating the worktree… · esc close");
    form.worktree = false;
    let (_, rows) = dialog(
        &new_agent(&form, 80, 24, Palette::PLAIN),
        "new agent",
        Palette::PLAIN,
    );
    assert_eq!(rows.last().unwrap(), "creating… · esc close");
}

#[test]
fn the_new_agent_cursor_sits_in_the_focused_field() {
    let mut form = form();
    form.focus = crate::dialog::FormField::Name;
    form.name = TextInput::new("ab");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| render_new_agent(f, &form, f.area(), Palette::PLAIN))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let &(x, y) = audit::find(&buffer, "name       ab")
        .first()
        .expect("the name row");
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!((cursor.x, cursor.y), (x + 13, y), "after `ab`");
}

#[test]
fn the_remove_confirm_is_destructive() {
    let confirm = RemoveConfirm {
        window_id: 1,
        name: "api-worker".into(),
        branch: Some("anthrex/wt".into()),
        remove_worktree: false,
    };
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = remove(&confirm, w, h, Palette::PLAIN);
        let (top, rows) = dialog(&buffer, "remove", Palette::PLAIN);
        assert_eq!(top.chars().count(), 64);
        assert_eq!(
            rows,
            vec![
                "Remove 'api-worker'?",
                "",
                "also remove worktree anthrex/wt  ‹ no ›",
                "the branch is kept; ignored files go too",
                "",
                "space toggle · y remove · esc cancel",
            ],
            "{w}x{h}"
        );
        assert_eq!(
            colour_at(&buffer, "remove ─"),
            fg(Role::Failed),
            "the title"
        );
        assert_eq!(colour_at(&buffer, "y remove"), fg(Role::Failed));
        let &(x, y) = audit::find(&buffer, "y remove").first().unwrap();
        assert_eq!(buffer[(x + 2, y)].fg, fg(Role::Failed), "the verb");
        assert_eq!(colour_at(&buffer, "space toggle"), fg(Role::Accent));
        assert_eq!(colour_at(&buffer, "the branch is kept"), fg(Role::Muted));
    }

    let plain = RemoveConfirm {
        window_id: 2,
        name: "shell-1".into(),
        branch: None,
        remove_worktree: false,
    };
    let (_, rows) = dialog(
        &remove(&plain, 80, 24, Palette::PLAIN),
        "remove",
        Palette::PLAIN,
    );
    assert_eq!(rows, vec!["Remove 'shell-1'?", "", "y remove · esc cancel"]);

    // Space flips the choice; Enter does not confirm (9.0.6 ruling: `y` only).
    let mut app = fixture("remove over the pane");
    let space = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
    assert!(app.on_key(space).is_empty());
    let buffer = audit::draw(&app, 80, 24);
    assert!(!audit::find(&buffer, "also remove worktree anthrex/wt  ‹ yes ›").is_empty());
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.on_key(enter).is_empty());
    assert!(
        matches!(app.modal, Some(Modal::Remove(_))),
        "Enter keeps it open"
    );
    let buffer = audit::draw(&app, 80, 24);
    assert!(!audit::find(&buffer, "press y to remove").is_empty());
}

#[test]
fn the_force_prompt_names_its_keys() {
    let message = "worktree /tmp/shop/feat-api has uncommitted or untracked changes";
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = paint(w, h, |f| {
            render_force_remove(f, "api", message, f.area(), Palette::PLAIN)
        });
        let (top, rows) = dialog(&buffer, "worktree holds work", Palette::PLAIN);
        assert_eq!(top.chars().count(), 64);
        assert_eq!(
            rows,
            vec![
                "agent 'api'",
                "worktree /tmp/shop/feat-api has uncommitted or",
                "untracked changes",
                "",
                "f  force: delete the worktree and everything in it",
                "k  keep the worktree, remove the window",
                "n  cancel",
            ],
            "{w}x{h}: the rows are the hints"
        );
        assert_eq!(colour_at(&buffer, "worktree holds work"), fg(Role::Failed));
        assert_eq!(colour_at(&buffer, "f  force"), fg(Role::Failed));
        assert_eq!(colour_at(&buffer, "k  keep"), fg(Role::Accent));
        assert_eq!(colour_at(&buffer, "n  cancel"), fg(Role::Accent));
    }
}

#[test]
fn rename_and_notice_on_the_kit() {
    let mut app = modal_app(Modal::Rename(RenamePrompt::new(1, "kept")));
    for (w, h) in [(80, 24), (120, 40)] {
        let (top, rows) = dialog(&modal(&app, w, h), "rename", Palette::PLAIN);
        assert_eq!(top.chars().count(), 64);
        assert_eq!(rows, vec!["name  kept█", "", "⏎ rename · esc cancel"]);
    }
    if let Some(Modal::Rename(prompt)) = &mut app.modal {
        prompt.error = Some("a window named 'api' exists".into());
    }
    let buffer = modal(&app, 80, 24);
    let (_, rows) = dialog(&buffer, "rename", Palette::PLAIN);
    assert_eq!(rows[1], "a window named 'api' exists");
    assert_eq!(colour_at(&buffer, "a window named"), fg(Role::Failed));
    assert_eq!(
        colour_at(&buffer, "name  kept"),
        fg(Role::Muted),
        "the label"
    );

    let mut notice = modal_app(Modal::Notice {
        title: " config ".into(),
        lines: vec!["config.toml: unknown key `x`".into()],
    });
    for (w, h) in [(80, 24), (120, 40)] {
        let (top, rows) = dialog(&modal(&notice, w, h), "config", Palette::PLAIN);
        assert_eq!(top.chars().count(), 64);
        assert_eq!(rows, vec!["config.toml: unknown key `x`", "", "esc close"]);
    }
    // Each problem wraps at 60.
    notice.modal = Some(Modal::Notice {
        title: " config ".into(),
        lines: vec!["word ".repeat(20)],
    });
    let (_, rows) = dialog(&modal(&notice, 120, 40), "config", Palette::PLAIN);
    assert_eq!(rows.len(), 4, "{rows:#?}");
    assert!(rows.iter().all(|r| r.chars().count() <= 60), "{rows:#?}");
}

#[test]
fn every_old_dialog_is_one_accented_frame() {
    for (name, _) in DIALOGS {
        let app = fixture(name);
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = audit::draw(&app, w, h);
            assert_eq!(
                audit::accented_frames(&buffer, app.palette()),
                1,
                "{name} at {w}x{h}:\n{}",
                audit::rows(&buffer).join("\n")
            );
        }
    }
}

#[test]
fn old_dialogs_in_ascii() {
    for (name, _) in DIALOGS {
        let mut app = fixture(name);
        app.settings.badges =
            crate::ui::badge::BadgeSet::from_config(&app.settings.badges_config, true);
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = audit::draw(&app, w, h);
            assert_eq!(
                audit::first_non_ascii(&buffer),
                None,
                "{name} at {w}x{h}:\n{}",
                audit::rows(&buffer).join("\n")
            );
        }
    }
    let buffer = new_agent(&form(), 80, 24, ascii());
    let (top, rows) = dialog(&buffer, "new agent", ascii());
    assert!(top.starts_with("+ new agent -"), "{top}");
    assert_eq!(rows[0], "> runtime    < claude >");
    assert_eq!(rows[3], "  worktree   < off > create a git worktree");
    assert_eq!(rows.last().unwrap(), "enter create - tab next - esc cancel");
    let confirm = RemoveConfirm {
        window_id: 1,
        name: "api".into(),
        branch: Some("anthrex/wt".into()),
        remove_worktree: true,
    };
    let (_, rows) = dialog(&remove(&confirm, 80, 24, ascii()), "remove", ascii());
    assert_eq!(rows[2], "also remove worktree anthrex/wt  < yes >");
    assert_eq!(rows[5], "space toggle - y remove - esc cancel");
    let mut rename_ascii = modal_app(Modal::Rename(RenamePrompt::new(1, "kept")));
    rename_ascii.settings.badges =
        crate::ui::badge::BadgeSet::from_config(&rename_ascii.settings.badges_config, true);
    let buffer = modal(&rename_ascii, 80, 24);
    assert_eq!(audit::first_non_ascii(&buffer), None);
    let (_, rows) = dialog(&buffer, "rename", ascii());
    assert_eq!(rows[0], "name  kept", "the cursor a reversed cell");
}

#[test]
fn every_dialog_keeps_esc_at_40_columns() {
    for (name, esc) in DIALOGS {
        let Some(esc) = esc else { continue };
        for ascii in [false, true] {
            let mut app = fixture(name);
            app.settings.badges =
                crate::ui::badge::BadgeSet::from_config(&app.settings.badges_config, ascii);
            for (w, h) in [(40, 24), (40, 40)] {
                let buffer = audit::draw(&app, w, h);
                assert!(
                    !audit::find(&buffer, esc).is_empty(),
                    "{name} at {w}x{h}, ascii {ascii}, keeps {esc:?}:\n{}",
                    audit::rows(&buffer).join("\n")
                );
            }
        }
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    for (name, _) in DIALOGS {
        let app = fixture(name);
        for (w, h) in [(20, 5), (1, 1), (5, 40), (40, 3), (3, 2)] {
            audit::draw(&app, w, h);
        }
    }
    let mut form = form();
    form.error = Some("e ".repeat(100));
    for (w, h) in [(20, 5), (1, 1), (5, 40), (0, 0)] {
        new_agent(&form, w, h, Palette::PLAIN);
    }
}

/// Window names, branches, the daemon's messages, typed fields and config problems
/// pass `safe_text`: hostile characters planted at the front of each, inside the
/// drawn columns, never reach a cell.
#[test]
fn dialog_text_is_sanitised() {
    let bad = format!("{PLANT}{}", hostile_text());
    let clean = |buffer: &Buffer| {
        let text = audit::rows(buffer).join(" ");
        assert_eq!(first_hostile(&text), None, "{text}");
        text
    };

    let confirm = RemoveConfirm {
        window_id: 1,
        name: bad.clone(),
        branch: Some(bad.clone()),
        remove_worktree: false,
    };
    let buffer = remove(&confirm, 120, 40, Palette::PLAIN);
    clean(&buffer);
    let (_, rows) = dialog(&buffer, "remove", Palette::PLAIN);
    assert!(rows[0].starts_with("Remove 'xyza b a b"), "{rows:#?}");
    assert!(
        rows.iter()
            .any(|r| r.starts_with("also remove worktree xyza b")),
        "{rows:#?}"
    );

    let buffer = paint(120, 40, |f| {
        render_force_remove(f, &bad, &bad, f.area(), Palette::PLAIN)
    });
    clean(&buffer);
    let (_, rows) = dialog(&buffer, "worktree holds work", Palette::PLAIN);
    assert!(rows[0].starts_with("agent 'xyza b"), "{rows:#?}");
    assert!(rows[1].starts_with("xyza b"), "{rows:#?}");

    let rename = modal_app(Modal::Rename(RenamePrompt::new(1, PLANT)));
    let buffer = modal(&rename, 80, 24);
    clean(&buffer);
    assert_eq!(dialog(&buffer, "rename", Palette::PLAIN).1[0], "name  xyz█");

    let notice = modal_app(Modal::Notice {
        title: " config ".into(),
        lines: vec![bad.clone()],
    });
    let buffer = modal(&notice, 80, 24);
    clean(&buffer);
    assert!(dialog(&buffer, "config", Palette::PLAIN).1[0].starts_with("xyza b"));

    let mut form = form();
    form.dir = TextInput::new(PLANT);
    form.error = Some(bad.clone());
    let buffer = new_agent(&form, 80, 24, Palette::PLAIN);
    clean(&buffer);
    let (_, rows) = dialog(&buffer, "new agent", Palette::PLAIN);
    assert_eq!(rows[2], "  directory  xyz");
    assert!(rows[6].starts_with("✗ xyza b"), "{rows:#?}");
}
