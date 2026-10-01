//! M9.0.7.2: the render audit over every fixture (decision 36), and the audit's own
//! helpers pinned on hand-built buffers.

use super::audit::{self, fixtures};
use super::kit;
use crate::app::App;
use crate::theme::{Palette, Role, role};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

fn palette(ascii: bool) -> Palette {
    Palette {
        accent: crate::theme::DEFAULT_ACCENT,
        truecolor: false,
        ascii,
    }
}

fn fixture(name: &str) -> App {
    fixtures()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
        .1
}

fn help_over(app: &mut App) {
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    assert!(app.modal.is_some(), "`C-b ?` opened the help");
}

/// Review focus 1: whatever has the keys, exactly one frame wears the accent.
#[test]
fn exactly_one_accented_frame_in_every_region() {
    for (name, app) in fixtures() {
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

/// Review focus 2.
#[test]
fn ascii_mode_emits_only_ascii_everywhere() {
    for (name, mut app) in fixtures() {
        app.settings.badges.ascii = true;
        // The badge set is built for one form: rebuild it, as `UiSettings` does.
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
}

/// §6.9: the state word and the `esc` hint visible where the mode has one, and the
/// fixture's actionable keys never only muted.
#[test]
fn every_fixture_shows_its_state_word_esc_and_keys() {
    for (name, app) in fixtures() {
        let shows = audit::shows(name);
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = audit::draw(&app, w, h);
            let shown = audit::rows(&buffer).join("\n");
            for text in std::iter::once(shows.state).chain(shows.esc) {
                assert!(
                    !audit::find(&buffer, text).is_empty(),
                    "{name} at {w}x{h} shows {text:?}:\n{shown}"
                );
            }
            for text in shows.actionable {
                audit::assert_actionable(&buffer, text, app.palette());
            }
        }
    }
}

#[test]
fn pane_titles_use_weight_not_the_accent() {
    let (_, app) = fixtures()
        .into_iter()
        .find(|(n, _)| *n == "sidebar tree")
        .unwrap();
    let buffer = audit::draw(&app, 120, 40);
    let accent = crate::theme::role(Role::Accent, app.palette()).fg;
    // ` agents · tree `: bold, default colour.
    let y = 0;
    let x = audit::rows(&buffer)[0].find("agents").unwrap() as u16;
    assert_ne!(buffer[(x, y)].fg, accent.unwrap());
    assert_ne!(buffer[(x, y)].fg, app.settings.accent);
    assert_eq!(buffer[(x, y)].fg, ratatui::style::Color::Reset);
    assert!(
        buffer[(x, y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
}

/// Decision 1 (pinning 9.0.6's final review minor 3): a full-body screen under the
/// help mutes its own frame; the help's is the one accented frame.
#[test]
fn the_profile_screen_mutes_under_a_modal() {
    let mut cases = vec![("profile", fixture("profile under the help"))];
    for name in ["settings", "stats"] {
        let mut app = fixture(name);
        help_over(&mut app);
        cases.push((name, app));
    }
    for (name, app) in cases {
        let accent = role(Role::Accent, app.palette()).fg;
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = audit::draw(&app, w, h);
            let shown = audit::rows(&buffer).join("\n");
            assert_eq!(
                audit::accented_frames(&buffer, app.palette()),
                1,
                "{name} under the help at {w}x{h}:\n{shown}"
            );
            assert_ne!(
                Some(buffer[(0, 0)].fg),
                accent,
                "{name}'s own corner at {w}x{h}"
            );
        }
    }
}

/// Decision 1: graph node boxes never wear the accent; a lit dependency's border is
/// bold on the muted border instead.
#[test]
fn graph_boxes_are_never_accented() {
    let mut app = fixture("run view running");
    app.tree.selected = Some(crate::tree::NodeKey::Task {
        run: crate::tree::run_fixtures::RUN_ID.into(),
        id: "t1".into(),
    });
    let p = app.palette();
    let accent = role(Role::Accent, p).fg;
    let muted = role(Role::Muted, p).fg;
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = audit::draw(&app, w, h);
        let shown = audit::rows(&buffer).join("\n");
        assert_eq!(audit::accented_frames(&buffer, p), 1, "{w}x{h}:\n{shown}");
        // Inside the run view's own frame, no box glyph is in the accent.
        let main = crate::ui::layout_for(&app, Rect::new(0, 0, w, h)).main;
        for y in main.y + 1..main.bottom() - 1 {
            for x in main.x + 1..main.right() - 1 {
                let cell = &buffer[(x, y)];
                let boxy = "╭╮╰╯│─┌┐└┘+-|".contains(cell.symbol());
                assert!(
                    !(boxy && Some(cell.fg) == accent),
                    "({x}, {y}) {:?} is accented at {w}x{h}:\n{shown}",
                    cell.symbol()
                );
            }
        }
        // `t0` is `t1`'s dependency, so it is lit: its box's corner is bold.
        let (at, row) = audit::find(&buffer, "t0 proto")
            .into_iter()
            .find(|&(x, y)| x > main.x && y > main.y)
            .unwrap_or_else(|| panic!("t0's box at {w}x{h}:\n{shown}"));
        let corner = (main.x + 1..at)
            .rev()
            .find(|&x| buffer[(x, row - 1)].symbol() == "╭")
            .unwrap_or_else(|| panic!("t0's corner at {w}x{h}:\n{shown}"));
        let cell = &buffer[(corner, row - 1)];
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "t0's border is bold"
        );
        assert_eq!(Some(cell.fg), muted, "t0's border is muted");
    }
}

fn frame_into(buffer: &mut Buffer, area: Rect, block: ratatui::widgets::Block<'static>) {
    block.render(area, buffer);
}

/// Pinning the helper: rounded, plain and ASCII accent frames count, titled or not; a
/// muted frame and a `+` in the text do not.
#[test]
fn accented_frames_counts_dialog_and_pane_corners() {
    let p = palette(false);
    let ascii = palette(true);
    let accent = role(Role::Accent, p);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 12));
    frame_into(
        &mut buffer,
        Rect::new(0, 0, 14, 4),
        kit::pane_frame(Line::from("rounded"), true, p),
    );
    frame_into(
        &mut buffer,
        Rect::new(15, 0, 14, 4),
        kit::dialog_frame("plain", false, p),
    );
    frame_into(
        &mut buffer,
        Rect::new(30, 0, 14, 4),
        kit::pane_frame(Line::from("ascii"), true, ascii),
    );
    frame_into(
        &mut buffer,
        Rect::new(45, 0, 14, 4),
        kit::pane_frame(Line::default(), true, p),
    );
    assert_eq!(audit::accented_frames(&buffer, p), 4);
    // A muted frame is not counted.
    frame_into(
        &mut buffer,
        Rect::new(0, 5, 14, 4),
        kit::pane_frame(Line::from("muted"), false, p),
    );
    // Nor is a `+` inside text in the accent, even over a `|` and before a `-`.
    Paragraph::new(vec![
        Line::styled("a+b-c", accent),
        Line::styled(" |", accent),
    ])
    .render(Rect::new(20, 5, 10, 2), &mut buffer);
    assert_eq!(
        audit::accented_frames(&buffer, p),
        4,
        "{}",
        audit::rows(&buffer).join("\n")
    );
}

fn count(p: Palette, width: u16, height: u16, frames: &[(Rect, Block<'static>)]) -> usize {
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    for (area, block) in frames {
        Clear.render(*area, &mut buffer);
        block.clone().render(*area, &mut buffer);
    }
    let n = audit::accented_frames(&buffer, p);
    eprintln!("{n}:\n{}", audit::rows(&buffer).join("\n"));
    n
}

/// Fix round 1: the frames a top-row reading miscounted. Abutting frames, frames
/// sharing a column (rounded and ASCII), a top row filled by its title, left and right
/// titles filling it, a modal whose top row lies on a pane's top row.
#[test]
fn accented_frames_counts_shared_and_filled_edges() {
    let (p, ascii) = (palette(false), palette(true));
    let pane = |t: &str, keys: bool, p: Palette| kit::pane_frame(Line::from(t.to_owned()), keys, p);
    let a = Rect::new(0, 0, 8, 4);
    for q in [p, ascii] {
        let abut = [
            (a, pane("a", true, q)),
            (Rect::new(8, 0, 8, 4), pane("b", true, q)),
        ];
        assert_eq!(count(q, 20, 6, &abut), 2, "abutting, ascii {}", q.ascii);
        let shared = [
            (Rect::new(0, 0, 9, 4), pane("a", true, q)),
            (Rect::new(8, 0, 8, 4), pane("b", true, q)),
        ];
        assert_eq!(
            count(q, 20, 6, &shared),
            2,
            "a shared column, ascii {}",
            q.ascii
        );
        let filled = [
            (a, pane("a title too long", true, q)),
            (Rect::new(8, 0, 8, 4), pane("b", true, q)),
        ];
        assert_eq!(
            count(q, 20, 6, &filled),
            2,
            "a filled title, ascii {}",
            q.ascii
        );
        let both = pane("left", true, q).title(Line::from("right side").right_aligned());
        assert_eq!(count(q, 20, 6, &[(Rect::new(0, 0, 12, 4), both)]), 1);
    }
    // The help at 80x24 sits on row 0, over an accented pane, and has a bottom title.
    let modal = pane("keys", true, p).title_bottom(Line::from(" any key  close "));
    let under = |keys| {
        let base = (Rect::new(0, 0, 40, 12), pane("agents", keys, p));
        [base, (Rect::new(10, 0, 20, 12), modal.clone())]
    };
    assert_eq!(count(p, 40, 12, &under(true)), 2, "two accents");
    assert_eq!(count(p, 40, 12, &under(false)), 1, "the modal's only");
}

/// Fix round 1: a frame clipped at the screen's bottom edge counts once, whether
/// ratatui drew its bottom border on the last row or it has none; an ASCII one's
/// top-right `+` is not a second frame. A frame whose bottom row is overwritten above
/// the edge is broken, and does not count.
#[test]
fn accented_frames_counts_a_frame_clipped_at_the_bottom() {
    let open = Borders::TOP | Borders::LEFT | Borders::RIGHT;
    for q in [palette(false), palette(true)] {
        let pane = |t: &str| kit::pane_frame(Line::from(t.to_owned()), true, q);
        let drawn = [(Rect::new(0, 5, 10, 10), pane("cut"))];
        assert_eq!(
            count(q, 12, 10, &drawn),
            1,
            "ratatui's clip, ascii {}",
            q.ascii
        );
        let bare = [(Rect::new(0, 5, 10, 5), pane("open").borders(open))];
        assert_eq!(
            count(q, 12, 10, &bare),
            1,
            "no bottom border, ascii {}",
            q.ascii
        );
    }
    let p = palette(false);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 10));
    kit::pane_frame(Line::from("x"), true, p).render(Rect::new(0, 2, 10, 5), &mut buffer);
    Paragraph::new("overwritten").render(Rect::new(0, 6, 12, 1), &mut buffer);
    assert_eq!(audit::accented_frames(&buffer, p), 0);
}

/// Decision 2: the pane frame is `+ - |` in ASCII, its title wrapped in one space
/// each side, bold in the default colour.
#[test]
fn pane_frame_draws_ascii_corners() {
    let p = palette(true);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 7, 3));
    frame_into(
        &mut buffer,
        Rect::new(0, 0, 7, 3),
        kit::pane_frame(Line::from("x"), true, p),
    );
    assert_eq!(audit::rows(&buffer), ["+ x --+", "|     |", "+-----+"]);
    assert_eq!(Some(buffer[(0, 0)].fg), role(Role::Accent, p).fg);
    assert_eq!(buffer[(2, 0)].fg, ratatui::style::Color::Reset);
    assert!(buffer[(2, 0)].modifier.contains(Modifier::BOLD));
    let mut unicode = Buffer::empty(Rect::new(0, 0, 7, 3));
    frame_into(
        &mut unicode,
        Rect::new(0, 0, 7, 3),
        kit::pane_frame(Line::from("x"), false, palette(false)),
    );
    assert_eq!(audit::rows(&unicode), ["╭ x ──╮", "│     │", "╰─────╯"]);
    assert_eq!(
        Some(unicode[(0, 0)].fg),
        role(Role::Muted, palette(false)).fg
    );
}

/// The audit's other two checks, pinned on hand-built buffers.
#[test]
fn first_non_ascii_and_actionable_find_their_cells() {
    let p = palette(false);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 2));
    Paragraph::new(vec![
        Line::from(vec![
            ratatui::text::Span::styled("a", role(Role::Accent, p)),
            ratatui::text::Span::styled(" approve", role(Role::Muted, p)),
        ]),
        Line::styled("ok ✓", role(Role::Muted, p)),
    ])
    .render(Rect::new(0, 0, 20, 2), &mut buffer);
    assert_eq!(audit::first_non_ascii(&buffer), Some((3, 1, "✓".into())));
    audit::assert_actionable(&buffer, "a approve", p);
    let muted = std::panic::catch_unwind(|| audit::assert_actionable(&buffer, "ok", p));
    assert!(muted.is_err(), "`ok` is drawn only in muted");
    let absent = std::panic::catch_unwind(|| audit::assert_actionable(&buffer, "reject", p));
    assert!(absent.is_err(), "`reject` is not on screen");
}
