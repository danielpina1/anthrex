use super::*;
use config::Rgb;
use unicode_width::UnicodeWidthStr;

#[test]
fn prefers_ascii_follows_the_setting_then_the_locale() {
    assert!(prefers_ascii(true, Some("en_US.UTF-8")));
    assert!(!prefers_ascii(false, Some("en_US.UTF-8")));
    assert!(!prefers_ascii(false, Some("en_US.utf8")));
    assert!(prefers_ascii(false, Some("C")));
    assert!(prefers_ascii(false, Some("POSIX")));
    assert!(prefers_ascii(false, Some("")));
    assert!(!prefers_ascii(false, None));
}

/// Pairwise-distinct glyphs, ASCII forms and colours across the three runtimes, so a
/// `from_config` that swapped two runtimes (e.g. Claude and Codex) would fail this test
/// instead of passing by coincidence.
fn distinct_badges() -> config::Badges {
    config::Badges {
        force_ascii: false,
        claude: config::Badge {
            glyph: "\u{25c6}".to_string(),
            ascii: "[C]".to_string(),
            color: Rgb(0x11, 0x22, 0x33),
        },
        codex: config::Badge {
            glyph: "\u{25c7}".to_string(),
            ascii: "[X]".to_string(),
            color: Rgb(0x44, 0x55, 0x66),
        },
        shell: config::Badge {
            glyph: "$".to_string(),
            ascii: "[$]".to_string(),
            color: Rgb(0x77, 0x88, 0x99),
        },
    }
}

#[test]
fn badges_come_from_the_config_in_both_modes() {
    let cfg = distinct_badges();

    let unicode = BadgeSet::from_config(&cfg, false);
    assert_eq!(unicode.claude.text, cfg.claude.glyph);
    assert_eq!(unicode.codex.text, cfg.codex.glyph);
    assert_eq!(unicode.shell.text, cfg.shell.glyph);

    let ascii = BadgeSet::from_config(&cfg, true);
    assert_eq!(ascii.claude.text, cfg.claude.ascii);
    assert_eq!(ascii.codex.text, cfg.codex.ascii);
    assert_eq!(ascii.shell.text, cfg.shell.ascii);

    let claude_color = Color::Rgb(cfg.claude.color.0, cfg.claude.color.1, cfg.claude.color.2);
    let codex_color = Color::Rgb(cfg.codex.color.0, cfg.codex.color.1, cfg.codex.color.2);
    let shell_color = Color::Rgb(cfg.shell.color.0, cfg.shell.color.1, cfg.shell.color.2);
    assert_eq!(unicode.claude.color, claude_color);
    assert_eq!(unicode.codex.color, codex_color);
    assert_eq!(unicode.shell.color, shell_color);
    assert_eq!(ascii.claude.color, claude_color);
    assert_eq!(ascii.codex.color, codex_color);
    assert_eq!(ascii.shell.color, shell_color);

    // The three colours (and glyphs, and ASCII forms) are pairwise distinct in the
    // fixture, so this also proves no two runtimes were swapped.
    assert_ne!(claude_color, codex_color);
    assert_ne!(codex_color, shell_color);
    assert_ne!(claude_color, shell_color);
}

#[test]
fn for_runtime_maps_each_runtime() {
    let set = BadgeSet::from_config(&distinct_badges(), false);
    let claude = set.for_runtime(proto::Runtime::Claude);
    let codex = set.for_runtime(proto::Runtime::Codex);
    let shell = set.for_runtime(proto::Runtime::Shell);
    assert_eq!(claude.text, "\u{25c6}");
    assert_eq!(codex.text, "\u{25c7}");
    assert_eq!(shell.text, "$");
    assert_ne!(claude.text, codex.text);
    assert_ne!(codex.text, shell.text);
    assert_ne!(claude.text, shell.text);
}

#[test]
fn a_badge_span_is_always_badge_width_columns() {
    let defaults = config::Badges::default();

    let unicode = BadgeSet::from_config(&defaults, false);
    let ascii = BadgeSet::from_config(&defaults, true);

    assert_eq!(
        UnicodeWidthStr::width(span(&unicode.claude).content.as_ref()),
        BADGE_WIDTH as usize
    );
    assert_eq!(
        UnicodeWidthStr::width(span(&ascii.claude).content.as_ref()),
        BADGE_WIDTH as usize
    );
}

#[test]
fn colour_is_never_the_only_signal() {
    for ascii in [false, true] {
        let set = BadgeSet::from_config(&config::Badges::default(), ascii);
        for badge in [&set.claude, &set.codex, &set.shell] {
            let rendered = span(badge);
            assert!(
                !rendered.content.trim().is_empty(),
                "badge text must not be blank (ascii={ascii})"
            );
        }
    }
}
