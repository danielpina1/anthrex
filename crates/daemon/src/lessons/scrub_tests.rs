//! Task 10's review, I1 and I2 (the controller's ruling at M9.15.11): [`scrub`] is the
//! one safe entry point. It decodes text-escaped control characters, then redacts,
//! strips escapes and redacts again, so no escape form, real or written out, hides a
//! secret. Every fake secret is built at run time.

use super::{BEARER, PRIVATE_KEY, TOKEN, scrub};

/// Every escape form task 9's and task 10's reviews used, written out or real.
const FORMS: [&str; 22] = [
    r"\u001b[31m",
    r"\u001B[1;31m",
    r"\u{1b}[31m",
    r"\x1b[31m",
    r"\x1B[0m",
    r"\033[31m",
    r"\033[1m",
    r"\0033[1m",
    r"\e[31m",
    r"\x1b",
    r"\b",
    r"\f",
    r"\U00000041",
    "%253D",
    "%25253D",
    "%1B[31m",
    "%zz",
    "%2",
    "q=",
    "\u{1b}[31m",
    "\u{1b}(B\u{1b}[m",
    "\u{1b}#8",
];

/// The secrets the forms must not hide: (the secret, a fragment that must not survive).
fn secrets() -> Vec<(String, String)> {
    let jwt = {
        let segment = format!("{}{}", "eyJmYWtl", "IjoiZmFrZSJ9");
        [segment.as_str(); 3].join(".")
    };
    let mut out: Vec<(String, String)> = [
        format!("sk-proj-{}", "FAKE0fake1".repeat(4)),
        format!("sk-svcacct-{}", "FAKE0fake1".repeat(4)),
        format!("sk-admin-{}", "FAKE0fake1".repeat(4)),
        format!("sk-{}", "FAKE0fake1".repeat(4)),
        format!("npm_{}", "FAKE0fake1".repeat(4)),
        format!("ghp_{}", "FAKE".repeat(9)),
        format!("AKIA{}", "FAKEFAKEFAKEFAKE"),
        jwt,
    ]
    .into_iter()
    .map(|s| {
        let tail = s.chars().skip(4).collect();
        (s, tail)
    })
    .collect();
    // Task 12's review, I2: an ambiguous credential needs a digit.
    let credential = "zFAKE0cred".repeat(4);
    out.push((format!("Bearer {credential}"), credential));
    out
}

#[test]
fn every_escape_form_before_every_secret_is_scrubbed() {
    for (secret, fragment) in secrets() {
        for form in FORMS {
            for input in [
                format!("{form}{secret}"),
                format!("key{form}{secret}"),
                format!("{{\"msg\":\"{form}{secret}\\u001b[0m\"}}"),
                format!("assert_eq!(left: \"auth {form}{secret}\")"),
            ] {
                let out = scrub(&input);
                assert!(
                    out.contains(TOKEN) || out.contains(BEARER),
                    "{input:?} -> {out:?}"
                );
                assert!(!out.contains(&fragment), "{input:?} -> {out:?}");
                assert!(!out.contains('\u{1b}'), "{input:?} -> {out:?}");
                assert_eq!(scrub(&out), out, "{input:?}: idempotent");
            }
        }
    }
}

/// Pinning: what is not a secret and not an escape passes unchanged.
#[test]
fn ordinary_text_is_left_alone() {
    let sha = "3f2a9c01".repeat(5);
    for text in [
        format!("commit {sha} on main; abbrev {}", &sha[..7]),
        "task-sk-ipaaaaaaaaaaaaaaaaaaaaaa xnpm_config_cache_directory_x".to_string(),
        // Task 13's review, I1: `laughs_count` and `sighs_total` (`ghs_` and four more)
        // are now tokens; see `the_ruling_costs_these_words`.
        "the laughs_at and sighs fields; AKIAno; eyJ alone; glpat".to_string(),
        "risk-assessment-of-the-new-pipeline-module desk-reservation-x".to_string(),
        "https://example.com/a%20b/c?x=1&y=%2Fz https://host:8080/p".to_string(),
        "/tmp/my%20dir/file%20name.txt and 100% done, 50%off".to_string(),
        r"C:\temp\new\token\sky and C:\exports\x".to_string(),
        r"a JSON \n and \t and \u00e9 and \uD83D\uDE00 and \u0041".to_string(),
        "error[E0308]: mismatched types\n  --> src/lib.rs:3:5".to_string(),
    ] {
        assert_eq!(scrub(&text), text, "{text:?}");
    }
}

/// Decoding never turns a placeholder into something else, and a scrubbed text
/// scrubs to itself.
#[test]
fn placeholders_survive_a_second_scrub() {
    let text =
        format!("a {TOKEN} b {BEARER} c {PRIVATE_KEY} https://***@host/x \u{1b}{TOKEN} %25{TOKEN}");
    let once = scrub(&text);
    assert!(
        once.contains(&format!("a {TOKEN} b {BEARER} c {PRIVATE_KEY}")),
        "{once}"
    );
    assert!(once.contains("https://***@host/x"), "{once}");
    assert_eq!(once.matches(TOKEN).count(), 3, "{once}");
    assert_eq!(scrub(&once), once);
}

/// Decoding is bounded: deep percent nesting stops after a few rounds and costs one
/// pass each.
#[test]
fn decoding_is_bounded() {
    let deep = format!("{}3D", "%25".repeat(50));
    let out = scrub(&deep);
    assert!(out.len() < deep.len(), "{out}");
    let long = r"\u001b[31m".repeat(20_000);
    assert_eq!(scrub(&long), "");
}

/// Ruling I2: the escape stripper is private to `redact.rs`; only it names it.
#[test]
fn only_redact_names_the_escape_stripper() {
    let names = [["ESC", "APE"].concat(), ["strip", "_escapes"].concat()];
    let src = testexec::checkout_dir(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src is readable") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            seen += 1;
            let rel = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "lessons/redact.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            for name in &names {
                let named = text.match_indices(name.as_str()).any(|(i, _)| {
                    let before = text[..i].chars().next_back();
                    let after = text[i + name.len()..].chars().next();
                    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                    !word(before) && !word(after)
                });
                assert!(!named, "{rel} names {name}");
            }
        }
    }
    assert!(seen > 100, "the walk found the sources ({seen})");
}
