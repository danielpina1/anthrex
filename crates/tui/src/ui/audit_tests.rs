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

/// Final fix wave I2: every accented Unicode box glyph is one frame's, so a frame left
/// accented under a centred dialog (which `accented_frames` cannot see: the dialog
/// crosses its left side) fails here.
#[test]
fn every_accented_box_glyph_is_one_frames() {
    let mut strays = Vec::new();
    for (name, app) in fixtures() {
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = audit::draw(&app, w, h);
            if let Some(stray) = audit::stray_accent(&buffer, app.palette()) {
                strays.push(format!("{name} at {w}x{h}: {stray:?}"));
            }
        }
    }
    assert!(strays.is_empty(), "{}", strays.join("\n"));
}

/// The accented `▌` cells in `app`'s frame at `w`x`h`.
fn accented_bars(app: &App, w: u16, h: u16) -> usize {
    let accent = role(Role::Accent, app.palette()).fg;
    let buffer = audit::draw(app, w, h);
    buffer
        .content()
        .iter()
        .filter(|c| c.symbol() == "▌" && Some(c.fg) == accent)
        .count()
}

/// Follow-up to the final fix wave's M3 (decisions 1 and 20): a selection bar or a
/// focus mark wears the accent only where the keys are, so a frame holds at most one
/// accented `▌`, and none with the help over any fixture (the help has no bar of its
/// own). (Unicode only: ASCII's `>` is also the accented prompt separator.)
#[test]
fn at_most_one_accented_bar_in_every_frame() {
    let mut extra = Vec::new();
    for (name, mut app) in fixtures() {
        for (w, h) in [(80, 24), (120, 40)] {
            let bars = accented_bars(&app, w, h);
            if bars > 1 {
                extra.push(format!("{name} at {w}x{h}: {bars}"));
            }
        }
        if app.modal.is_none() {
            help_over(&mut app);
            for (w, h) in [(80, 24), (120, 40)] {
                let bars = accented_bars(&app, w, h);
                if bars > 0 {
                    extra.push(format!("help over {name} at {w}x{h}: {bars}"));
                }
            }
        }
    }
    assert!(extra.is_empty(), "{}", extra.join("\n"));
}

/// Pinning the helper: one frame with a joined rule is clean; a second frame crossed by
/// a dialog, or an accented rule not joined to the frame, is a stray.
#[test]
fn stray_accent_sees_a_frame_under_a_dialog() {
    let p = palette(false);
    let pane = |t: &str, keys: bool| kit::pane_frame(Line::from(t.to_owned()), keys, p);
    let draw = |under: bool| {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 12));
        pane("main", under).render(Rect::new(10, 0, 30, 12), &mut buffer);
        Clear.render(Rect::new(4, 3, 24, 6), &mut buffer);
        kit::dialog_frame("ask", true, p).render(Rect::new(4, 3, 24, 6), &mut buffer);
        buffer
    };
    assert_eq!(audit::stray_accent(&draw(false), p), None);
    let buffer = draw(true);
    // The old count is blind to it: the dialog cuts the main frame's left side.
    assert_eq!(audit::accented_frames(&buffer, p), 1);
    assert!(audit::stray_accent(&buffer, p).is_some());
    let mut ruled = Buffer::empty(Rect::new(0, 0, 12, 8));
    pane("plan", true).render(Rect::new(0, 0, 12, 8), &mut ruled);
    let accent = role(Role::Accent, p);
    Paragraph::new(Line::styled(format!("├{}┤", "─".repeat(10)), accent))
        .render(Rect::new(0, 3, 12, 1), &mut ruled);
    assert_eq!(audit::stray_accent(&ruled, p), None);
    Paragraph::new(Line::styled("──", accent)).render(Rect::new(3, 5, 2, 1), &mut ruled);
    assert_eq!(audit::stray_accent(&ruled, p), Some((3, 5, "─".into())));
    // ASCII `-` (an accented `C-b`) is never a box glyph.
    let mut keys = Buffer::empty(Rect::new(0, 0, 12, 8));
    pane("x", true).render(Rect::new(0, 0, 12, 8), &mut keys);
    Paragraph::new(Line::styled("C-b", accent)).render(Rect::new(2, 3, 3, 1), &mut keys);
    assert_eq!(audit::stray_accent(&keys, p), None);
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
        // Milestone 9.0.7 decision 22: at 80x24 the run view is the compact list, which
        // has no boxes; the graph's are checked at 120x40.
        if crate::ui::overview::view(&app, main).list {
            continue;
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
    audit::accented_frames(&buffer, p)
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
    // The help at 80x24 sits on row 0, over an accented pane, and has a bottom title;
    // in colour and (final fix wave, task 2's deferred minor) in ASCII. In ASCII the
    // modal's top-right `+` on the pane's top border reads as a shared column (the
    // `shared` case above), so two accents may count three there. The exact count is
    // an artifact; what is pinned is that two accented frames never count one, which
    // is all the audit's `== 1` needs to catch them.
    for q in [p, ascii] {
        let modal = pane("keys", true, q).title_bottom(Line::from(" any key  close "));
        let under = |keys| {
            let base = (Rect::new(0, 0, 40, 12), pane("agents", keys, q));
            [base, (Rect::new(10, 0, 20, 12), modal.clone())]
        };
        let two = count(q, 40, 12, &under(true));
        assert!(two >= 2, "two accents count {two}, ascii {}", q.ascii);
        assert_eq!(
            count(q, 40, 12, &under(false)),
            1,
            "the modal's, ascii {}",
            q.ascii
        );
    }
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

/// Milestone 9.0.7 task 11: a rule joined to the frame's sides (`├─…─┤`, the plan
/// review's) continues the frame's left side, so the frame still counts once; a muted
/// junction breaks the side, so a frame whose rules are not its border's colour does
/// not count.
#[test]
fn accented_frames_counts_a_frame_with_joined_rules_once() {
    let p = palette(false);
    let accent = role(Role::Accent, p);
    let draw = |style: ratatui::style::Style| {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 8));
        kit::pane_frame(Line::from("plan"), true, p).render(Rect::new(0, 0, 12, 8), &mut buffer);
        for y in [2, 4] {
            Paragraph::new(Line::styled(format!("├{}┤", "─".repeat(10)), style))
                .render(Rect::new(0, y, 12, 1), &mut buffer);
        }
        audit::accented_frames(&buffer, p)
    };
    assert_eq!(draw(accent), 1);
    assert_eq!(draw(role(Role::Muted, p)), 0);
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
