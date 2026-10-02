use crate::theme::{Glyph, Palette, Role, glyph, role, truecolor};
use ratatui::style::Color;

const ROLES: [Role; 7] = [
    Role::Accent,
    Role::Attention,
    Role::Working,
    Role::Done,
    Role::Failed,
    Role::Muted,
    Role::Paused,
];

/// §6.9's theme test, built here because the roles are: with truecolor off, every
/// role resolves to one of the 16 ANSI colours, whatever the configured accent.
#[test]
fn every_role_is_ansi_when_truecolor_is_off() {
    let p = Palette {
        accent: Color::Rgb(1, 2, 3),
        truecolor: false,
        ascii: false,
    };
    for r in ROLES {
        let fg = role(r, p).fg.expect("a role has a foreground");
        assert!(
            !matches!(fg, Color::Rgb(..) | Color::Indexed(_)),
            "{r:?} is {fg:?}"
        );
    }
    assert_ne!(role(Role::Working, p).fg, role(Role::Attention, p).fg);
}

#[test]
fn truecolor_needs_colorterm_unless_forced() {
    use config::Truecolor::*;
    assert!(truecolor(Auto, Some("truecolor")) && truecolor(Auto, Some("24BIT")));
    assert!(!truecolor(Auto, Some("256color")) && !truecolor(Auto, None));
    assert!(truecolor(On, None) && !truecolor(Off, Some("truecolor")));
}

#[test]
fn every_glyph_has_an_ascii_twin() {
    for g in [
        Glyph::Passed,
        Glyph::Failed,
        Glyph::NotStarted,
        Glyph::Idle,
        Glyph::NeedsYou,
        Glyph::Collapsed,
        Glyph::Selection,
        Glyph::Separator,
        Glyph::Warning,
        Glyph::Expanded,
        Glyph::Live,
        Glyph::Checking,
        Glyph::Review,
        Glyph::Queued,
        Glyph::Merging,
        Glyph::Blocked,
        Glyph::Ended,
        Glyph::Paused,
        Glyph::Run,
        Glyph::Hub,
        Glyph::Focus,
    ] {
        assert!(glyph(g, true).is_ascii(), "{g:?}");
        assert!(!glyph(g, false).is_ascii(), "{g:?}");
    }
    assert_eq!(glyph(Glyph::Failed, false), "✗");
    // Milestone 9.0.7 decision 6: the twelve marks appended to the table.
    let grown = [
        (Glyph::Expanded, "▾", "v"),
        (Glyph::Live, "●", "*"),
        (Glyph::Checking, "◇", "~"),
        (Glyph::Review, "◐", "%"),
        (Glyph::Queued, "▫", ":"),
        (Glyph::Merging, "»", "="),
        (Glyph::Blocked, "⊘", "#"),
        (Glyph::Ended, "–", "_"),
        (Glyph::Paused, "‖", "\""),
        (Glyph::Run, "◉", "@"),
        (Glyph::Hub, "◆", "H"),
        (Glyph::Focus, "▎", "|"),
    ];
    for (g, uni, ascii) in grown {
        assert_eq!((glyph(g, false), glyph(g, true)), (uni, ascii), "{g:?}");
    }
    assert_eq!(
        (0..4)
            .map(|f| crate::theme::spinner(f, true))
            .collect::<String>(),
        "-\\|/"
    );
}

/// With truecolor on, the roles are the decision-1 colours; the accent is the
/// configured one; `Working` is never `Attention`.
#[test]
fn truecolor_roles_use_the_configured_accent_and_the_decision_colours() {
    let accent = Color::Rgb(9, 8, 7);
    let p = Palette {
        accent,
        truecolor: true,
        ascii: false,
    };
    assert_eq!(role(Role::Accent, p).fg, Some(accent));
    assert_eq!(
        role(Role::Attention, p).fg,
        Some(Color::Rgb(0xfa, 0xb3, 0x87))
    );
    assert_eq!(
        role(Role::Working, p).fg,
        Some(Color::Rgb(0xf9, 0xe2, 0xaf))
    );
    assert_eq!(role(Role::Done, p).fg, Some(Color::Rgb(0xa6, 0xe3, 0xa1)));
    assert_eq!(role(Role::Failed, p).fg, Some(Color::Rgb(0xf3, 0x8b, 0xa8)));
    assert_eq!(role(Role::Muted, p).fg, Some(Color::Rgb(0x6c, 0x70, 0x86)));
    assert_eq!(role(Role::Paused, p).fg, Some(Color::Rgb(0xcb, 0xa6, 0xf7)));
    assert_ne!(role(Role::Working, p).fg, role(Role::Attention, p).fg);
}

#[test]
fn attention_is_bold_and_the_ansi_table_is_decision_1s() {
    use ratatui::style::Modifier;
    for truecolor in [true, false] {
        let p = Palette {
            accent: Color::Rgb(1, 2, 3),
            truecolor,
            ascii: false,
        };
        assert!(
            role(Role::Attention, p)
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(!role(Role::Working, p).add_modifier.contains(Modifier::BOLD));
    }
    let p = Palette {
        accent: Color::Rgb(1, 2, 3),
        truecolor: false,
        ascii: false,
    };
    let fgs: Vec<_> = ROLES.iter().map(|r| role(*r, p).fg).collect();
    assert_eq!(
        fgs,
        [
            Color::LightBlue,
            Color::LightMagenta,
            Color::Yellow,
            Color::Green,
            Color::Red,
            Color::DarkGray,
            Color::Cyan
        ]
        .map(Some)
    );
}

#[test]
fn glyph_table_is_the_spec_table() {
    let table = [
        (Glyph::Passed, "✓", "+"),
        (Glyph::Failed, "✗", "x"),
        (Glyph::NotStarted, "◌", "."),
        (Glyph::Idle, "○", "o"),
        (Glyph::NeedsYou, "⚑", "!"),
        (Glyph::Collapsed, "▸", ">"),
        (Glyph::Selection, "▌", ">"),
        (Glyph::Separator, "›", ">"),
        (Glyph::Warning, "⚠", "!"),
    ];
    for (g, uni, ascii) in table {
        assert_eq!(glyph(g, false), uni, "{g:?}");
        assert_eq!(glyph(g, true), ascii, "{g:?}");
    }
    assert_eq!(crate::theme::spinner(0, false), crate::theme::SPINNER[0]);
    assert_eq!(crate::theme::spinner(13, false), crate::theme::SPINNER[3]);
    assert_eq!(crate::theme::spinner(5, true), "\\");
}

#[test]
fn runtime_tags() {
    use proto::Runtime;
    assert_eq!(crate::theme::runtime_tag(Runtime::Claude), "cl");
    assert_eq!(crate::theme::runtime_tag(Runtime::Codex), "cx");
    assert_eq!(crate::theme::runtime_tag(Runtime::Shell), "sh");
}
