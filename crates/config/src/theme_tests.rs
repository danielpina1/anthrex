//! Milestone 9.0.6 task 2: the `[theme]` table.

use super::super::*;

#[test]
fn theme_truecolor_reads_each_value() {
    assert_eq!(parse("").0.theme.truecolor, Truecolor::Auto);
    for (text, want) in [
        ("auto", Truecolor::Auto),
        ("on", Truecolor::On),
        ("off", Truecolor::Off),
    ] {
        let (config, problems) = parse(&format!("[theme]\ntruecolor = \"{text}\"\n"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.theme.truecolor, want, "{text}");
    }
}

#[test]
fn a_bad_truecolor_is_a_problem() {
    let (config, problems) = parse("[theme]\ntruecolor = 3\n");
    assert_eq!(config.theme.truecolor, Truecolor::Auto);
    let shown: Vec<String> = problems.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        vec![r#"theme.truecolor: must be "auto", "on" or "off" (using auto)"#.to_string()]
    );
}

#[test]
fn an_unknown_theme_key_is_reported() {
    let (_, problems) = parse("[theme]\ncolour = \"on\"\n");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].key, "theme.colour");
    assert_eq!(problems[0].message, "unknown key, ignored");
}
