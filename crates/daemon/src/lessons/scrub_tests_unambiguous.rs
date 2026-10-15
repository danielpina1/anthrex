//! Task 13's review, I1 (the controller's ruling after M9.15.14): the unambiguous
//! prefixes (`ghp_` and the other `gh?_`, `github_pat_`, `glpat-`, `sk-ant-`,
//! `sk-proj-`, `sk-svcacct-`, `sk-admin-`, `xox?-`, `AKIA`) are unambiguous by
//! definition, so they match anywhere, with no left boundary, and a body of at least
//! four characters of the prefix's own alphabet (a placeholder counts as body) that
//! holds an uppercase letter, a digit or a placeholder (the ruling after that fix) is a
//! token. A token clipped at a check tail's 300 characters, or split by a placeholder,
//! right after a written-out colour code is therefore still redacted from its prefix
//! on. Every fake secret is built at run time.

use super::scrub_tests_spellings::SPELLINGS;
use super::{TOKEN, redact, scrub};

/// The spellings task 13's review measured besides [`SPELLINGS`].
const MORE: [&str; 6] = [
    r"\x1b[31m",
    r"\u001b[31m",
    r"\u{1b}[31m",
    r"\e[31m",
    "[31m",
    "\x1b[31m",
];

const PREFIXES: [&str; 12] = [
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "sk-ant-",
    "sk-proj-",
    "sk-svcacct-",
    "sk-admin-",
    "xoxb-",
    "xoxp-",
    "AKIA",
];

/// Bodies: full, clipped to 4 to 10 characters, and split by a placeholder. Every
/// clipped body holds an uppercase letter or a digit, as a real base62 body does (the
/// controller's ruling after M9.15.14's fix); [`an_all_lowercase_clipped_body_is_kept`]
/// pins the residual.
fn bodies() -> Vec<String> {
    let full = ["FAKE0", "FAKE1", "FAKE2", "FAKE3"].concat();
    let mut all = vec![full.clone()];
    all.extend((4..=10).map(|k| full[..k].to_string()));
    all.push(format!("FA<sha>{}", &full[..8]));
    all.push(format!("{}<id>", &full[..4]));
    all
}

fn spellings() -> impl Iterator<Item = &'static str> {
    SPELLINGS.iter().chain(MORE.iter()).copied()
}

#[test]
fn an_unambiguous_token_behind_any_spelling_is_replaced() {
    let mut inputs = 0;
    for spelling in spellings() {
        for prefix in PREFIXES {
            for body in bodies() {
                let token = format!("{prefix}{body}");
                // The prefix and the body's first two characters: a recognisable start.
                let start = format!("{prefix}{}", &body[..2]);
                for input in [
                    format!("error: {spelling}{token}"),
                    format!("error: {spelling}{token}\nnext"),
                    format!("{{\"msg\":\"auth {spelling}{token} rest\"}}"),
                ] {
                    inputs += 1;
                    for (how, out) in [("redact", redact(&input)), ("scrub", scrub(&input))] {
                        assert!(out.contains(TOKEN), "{how} {input:?} -> {out:?}");
                        assert!(!out.contains(&start), "{how} {input:?} -> {out:?}");
                    }
                }
            }
        }
    }
    assert_eq!(inputs, 36 * 12 * 10 * 3);
}

#[test]
fn unambiguous_prefixes_keep_prose() {
    let sha = "3f2a9c01".repeat(5);
    let uuid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let keep = [
        "AKIA is the prefix of an access key id".to_string(),
        "AKIA".to_string(),
        "AKIAno and AKIA.".to_string(),
        "ghp_".to_string(),
        "ghp_abc".to_string(),
        "ghp_ab rest".to_string(),
        "glpat-x and xoxb-12".to_string(),
        "ask-for-help".to_string(),
        "[ask-for-help](url)".to_string(),
        "Bearer authentication-required".to_string(),
        "task-sk-ipped".to_string(),
        "npm_install_script_name".to_string(),
        format!("commit {sha}"),
        sha.clone(),
        format!("request {uuid} failed"),
        "sk-1.2.3-rc-2026-10-13".to_string(),
    ];
    for text in &keep {
        assert_eq!(&redact(text), text, "{text:?}");
        assert_eq!(&scrub(text), text, "{text:?}");
        for spelling in spellings() {
            let input = format!("{spelling}{text}");
            assert_eq!(redact(&input), input, "{input:?}");
        }
    }
}

#[test]
fn the_ruling_costs_these_words() {
    // The controller's ruling after M9.15.14's redaction fix: a body must also hold an
    // uppercase letter or a digit (real token bodies are random base62), so an
    // identifier that holds an unambiguous prefix and four more lowercase characters
    // is a word again. These were redacted under the first ruling.
    for text in [
        "the laughs_count field",
        "sighs_total",
        "task-proj-name",
        "desk-admin-panel",
        "desk-admin-panel-component-renderer",
        "ghs_lowercase_only_identifier_name",
    ] {
        assert_eq!(redact(text), text, "{text:?}");
        assert_eq!(scrub(text), text, "{text:?}");
    }
}

#[test]
fn an_all_lowercase_clipped_body_is_kept() {
    // The accepted residual of the same ruling: a token clipped to an all-lowercase
    // body of four is not caught, behind any spelling. (`AKIA`'s alphabet has no
    // lowercase, so it never had a body here.)
    for spelling in spellings() {
        for prefix in PREFIXES {
            let kept = format!("{prefix}fake");
            let input = format!("error: {spelling}{kept}");
            for (how, out) in [("redact", redact(&input)), ("scrub", scrub(&input))] {
                assert!(out.ends_with(&kept), "{how} {input:?} -> {out:?}");
                assert!(!out.contains(TOKEN), "{how} {input:?} -> {out:?}");
            }
        }
    }
}

#[test]
fn the_unambiguous_body_check_is_linear() {
    let n = 2_000;
    for unit in [
        "ghp_a ".to_string(),
        "ghp_<sha> ".to_string(),
        "AKIA AKIAB1 ".to_string(),
        "ghp_FAKE0 ".to_string(),
        "xoxb-ab xoxp-\u{1b}[0m\u{200b} ".to_string(),
    ] {
        // Units the whole-token pass leaves to the body check (under 16 characters).
        let text = unit.repeat(n);
        super::BODY_READ.with(|c| c.set(0));
        let _ = redact(&text);
        let read = super::BODY_READ.with(|c| c.get());
        // Each match's body is read once, and matches do not overlap.
        assert!(read <= text.len() as u64, "{read} for {}", text.len());
        assert!(read > 0, "the check ran for {unit:?}");
    }
}

#[test]
fn a_token_split_by_an_escape_or_a_hidden_character_goes_whole() {
    // A short body is a token now, so the body reads through what a later step
    // removes (`scrub` strips the escape, `multi_line` drops the hidden character);
    // otherwise the rest of the token would survive once they are gone.
    let rest = ["FAKE0", "FAKE1", "FAKE2"].concat();
    for split in [
        "\u{1b}[0m",
        "\u{1b}]8;;x\u{7}",
        "\u{9b}1m",
        "\u{200b}",
        "\u{2066}",
    ] {
        for prefix in ["ghp_", "AKIA", "xoxb-"] {
            let input = format!("error: {prefix}FAKE{split}{rest} next");
            for (how, out) in [("redact", redact(&input)), ("scrub", scrub(&input))] {
                assert_eq!(out, format!("error: {TOKEN} next"), "{how} {input:?}");
            }
        }
    }
}
