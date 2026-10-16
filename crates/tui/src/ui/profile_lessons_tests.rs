//! Milestone 9.15 decisions 35-37 (task M9.15.12): the Lessons page drawn at 80×24, one
//! row per lesson with decision 37's glyph in its role, in Unicode and in ASCII; stored
//! text sanitised onto one line; the Profile hint line's `l lessons · <n> to review`.

use crate::app::App;
use crate::app::profile_screen::lessons::{Question, Verb};
use crate::app::profile_screen::{LessonsPage, ProfilePage};
use crate::theme::{Role, role};
use crate::ui::audit;
use crate::ui::profile::tests::{screen_mut, sp_app};
use proto::{LessonInfo, LessonLayer, LessonOrigin, LessonStatus};
use ratatui::style::Color;

fn lesson(id: &str, status: LessonStatus, title: &str) -> LessonInfo {
    LessonInfo {
        id: id.into(),
        layer: LessonLayer::Episodic,
        status,
        origin: LessonOrigin::Distilled,
        title: title.into(),
        body: format!("body of {title}"),
        files: vec!["crates/a/**".into(), "crates/b/x.rs".into()],
        created_at: 0,
        last_seen: 0,
        runs: vec![],
        green: 2,
        outcomes: 3,
        reconfirmed: 0,
        headlines: vec![],
        retired_reason: None,
    }
}

/// Every status, one lesson each, in decision 37's order.
fn every_status() -> Vec<LessonInfo> {
    let mut list = vec![
        lesson("kf-000001", LessonStatus::Known, "known failure"),
        lesson("l-000002", LessonStatus::OnTrial, "on trial"),
        lesson("l-000003", LessonStatus::Proposed, "proposed one"),
        lesson("l-000004", LessonStatus::Approved, "approved one"),
        lesson("l-000005", LessonStatus::Kept, "kept one"),
        lesson("l-000006", LessonStatus::Rejected, "rejected one"),
        lesson("l-000007", LessonStatus::Retired, "retired one"),
    ];
    list[1].outcomes = 0;
    list[1].green = 0;
    list
}

fn with_page(ascii: bool, lessons: Vec<LessonInfo>) -> App {
    let mut app = sp_app(ascii);
    let page = LessonsPage {
        lessons: Some(lessons),
        ..LessonsPage::default()
    };
    screen_mut(&mut app).page = Some(ProfilePage::Lessons(Box::new(page)));
    app
}

fn page_mut(app: &mut App) -> &mut LessonsPage {
    match &mut screen_mut(app).page {
        Some(ProfilePage::Lessons(page)) => page,
        _ => panic!("no lessons page"),
    }
}

/// The foreground of the first cell of `text` on screen.
fn fg_of(buffer: &ratatui::buffer::Buffer, text: &str) -> Option<Color> {
    let (x, y) = *audit::find(buffer, text).first()?;
    Some(buffer[(x, y)].fg)
}

#[test]
fn rows_at_80x24_with_status_glyphs() {
    for ascii in [false, true] {
        let app = with_page(ascii, every_status());
        let p = app.palette();
        let buffer = audit::draw(&app, 80, 24);
        let rows = audit::rows(&buffer);
        let all = rows.join("\n");
        let normal = Color::Reset;
        let cases: [(&str, &str, &str, Option<Role>); 7] = [
            ("✖", "x", "known failure", Some(Role::Failed)),
            ("◌", ".", "on trial", Some(Role::Muted)),
            ("○", "o", "proposed one", Some(Role::Attention)),
            ("●", "*", "approved one", None),
            ("●", "*", "kept one", None),
            ("–", "-", "rejected one", Some(Role::Muted)),
            ("–", "-", "retired one", Some(Role::Muted)),
        ];
        for (uni, asc, title, r) in cases {
            let glyph = if ascii { asc } else { uni };
            let text = format!("{glyph} {title}");
            let row = rows
                .iter()
                .find(|row| row.contains(&text))
                .unwrap_or_else(|| panic!("no row `{text}` (ascii {ascii})\n{all}"));
            // The layer and the usefulness at the row's right.
            let dot = if ascii { "-" } else { "·" };
            let right = if title == "on trial" {
                format!("episodic {dot} {}", if ascii { "-" } else { "–" })
            } else {
                format!("episodic {dot} 2/3")
            };
            let inside = row.trim_end_matches(['│', '|']).trim_end();
            assert!(inside.ends_with(&right), "{row:?} ends with {right:?}");
            let want = r.map_or(normal, |r| role(r, p).fg.unwrap_or(Color::Reset));
            assert_eq!(fg_of(&buffer, &text), Some(want), "{title} (ascii {ascii})");
        }
        // The selected (first) lesson's body and files under the list.
        assert!(all.contains("body of known failure"), "{all}");
        assert!(all.contains("crates/a/**, crates/b/x.rs"), "{all}");
        if ascii {
            assert_eq!(audit::first_non_ascii(&buffer), None, "{all}");
        }
    }
}

/// `r` and `d`'s question on the page's bottom line, its verb and `y/n` in `Failed`.
#[test]
fn the_question_is_on_the_bottom_line() {
    let mut app = with_page(false, every_status());
    page_mut(&mut app).question = Some(Question {
        verb: Verb::Forget,
        id: "kf-000001".into(),
    });
    let buffer = audit::draw(&app, 80, 24);
    let rows = audit::rows(&buffer);
    let at = audit::find(&buffer, "forget kf-000001? y/n");
    assert_eq!(at.len(), 1, "{}", rows.join("\n"));
    let failed = role(Role::Failed, app.palette()).fg;
    assert_eq!(fg_of(&buffer, "forget kf-000001"), failed);
    assert_eq!(fg_of(&buffer, "y/n"), failed);
    // It is the frame's last row: the border's corner is under its left.
    let (x, y) = at[0];
    assert_eq!(buffer[(x - 1, y + 1)].symbol(), "└", "{}", rows.join("\n"));
}

/// Stored text is the store's, already scrubbed; the page still draws every title, body
/// and file through `safe_text` on one line, so no stored character reaches the terminal.
#[test]
fn hostile_titles_on_one_line() {
    let hostile = crate::safe_text::tests::hostile_text();
    let mut bad = lesson("l-0000ff", LessonStatus::Proposed, "");
    bad.title = format!("one\ntwo\x1b[31mred {hostile}");
    bad.body = hostile.clone();
    bad.files = vec![hostile.clone()];
    bad.id = format!("l-\x1b]0;x\x07{hostile}");
    for ascii in [false, true] {
        let mut app = with_page(ascii, vec![bad.clone()]);
        let buffer = audit::draw(&app, 80, 24);
        let rows = audit::rows(&buffer);
        let all = rows.join(" ");
        assert!(
            rows.iter().any(|r| r.contains("one two [31mred")),
            "the title on one line\n{all}"
        );
        assert_eq!(crate::safe_text::tests::first_hostile(&all), None, "{all}");
        page_mut(&mut app).question = Some(Question {
            verb: Verb::Reject,
            id: bad.id.clone(),
        });
        page_mut(&mut app).error = Some(hostile.clone());
        let all = audit::rows(&audit::draw(&app, 80, 24)).join(" ");
        assert_eq!(crate::safe_text::tests::first_hostile(&all), None, "{all}");
        let crate::app::screens::Screen::Profile(s) = app.screen.as_ref().unwrap() else {
            panic!("no profile screen");
        };
        let page = s.page.as_ref().unwrap();
        for line in crate::ui::profile::page_lines(&app, s, page, 78) {
            for span in line.spans {
                assert_eq!(
                    crate::safe_text::tests::first_hostile(&span.content),
                    None,
                    "{span:?}"
                );
            }
        }
    }
}

/// Decision 36: `l lessons`, then ` · <n> to review` while `n` lessons are proposed.
#[test]
fn profile_hint_counts_proposals() {
    let mut app = sp_app(false);
    let hint = |app: &App| {
        let crate::app::screens::Screen::Profile(s) = app.screen.as_ref().unwrap() else {
            panic!("no profile screen");
        };
        crate::ui::profile::hints(s)
            .into_iter()
            .find(|h| h.key == "l")
            .map(|h| h.word)
    };
    assert_eq!(hint(&app).as_deref(), Some("lessons"));
    screen_mut(&mut app).to_review = 2;
    assert_eq!(hint(&app).as_deref(), Some("lessons · 2 to review"));
    let bar = audit::rows(&audit::draw(&app, 80, 24)).join("\n");
    assert!(bar.contains("l lessons · 2 to review"), "{bar}");
    screen_mut(&mut app).to_review = 0;
    assert_eq!(hint(&app).as_deref(), Some("lessons"));
}
