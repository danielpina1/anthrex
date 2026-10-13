//! Task M9.15.8: decision 11's redaction. Every fake secret is built at run time (the
//! shared test rules), so no secret-shaped literal sits in the repository.

use super::{BEARER, PRIVATE_KEY, TOKEN, redact};

fn ghp() -> String {
    format!("ghp_{}", "FAKE".repeat(9))
}

fn sk_ant() -> String {
    format!("sk-ant-{}", "fake0".repeat(8))
}

fn akia() -> String {
    format!("AKIA{}", "FAKEFAKEFAKEFAKE")
}

fn xoxb() -> String {
    format!("xoxb-{}", "0000-fake".repeat(3))
}

/// Three base64 segments of `{"fake":"fake"}`.
fn jwt() -> String {
    let segment = format!("{}{}", "eyJmYWtl", "IjoiZmFrZSJ9");
    [segment.as_str(); 3].join(".")
}

/// A PEM block of `kind` (`"RSA "`, `""`, …) whose body is `FAKE` repeated.
fn pem(kind: &str, newline: &str) -> String {
    let marker = |edge: &str| format!("-----{edge} {kind}{}-----", "PRIVATE KEY");
    let body = ["FAKE".repeat(16), "FAKE".repeat(16), "FAKE==".to_string()].join(newline);
    [marker("BEGIN"), body, marker("END")].join(newline)
}

/// Every prefixed shape the brief lists, each with 20 token characters; the ambiguous
/// `sk-` and `npm_` hold a digit (task 12's review, I2: the controller's ruling).
fn prefixed() -> Vec<String> {
    let mut tokens: Vec<String> = [
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "glpat-",
        "sk-",
        "sk-ant-",
        "xoxa-",
        "xoxb-",
        "xoxp-",
        "xoxr-",
        "xoxs-",
        "AKIA",
        "npm_",
    ]
    .iter()
    .map(|prefix| match *prefix {
        "sk-" | "npm_" => format!("{prefix}{}", "FAKE0".repeat(4)),
        _ => format!("{prefix}{}", "FAKE".repeat(5)),
    })
    .collect();
    tokens.extend([ghp(), sk_ant(), akia(), xoxb()]);
    tokens
}

/// The secret fragments that must never survive: the token itself, and the prefix with
/// its first eight characters (a recognisable start of it).
fn assert_gone(out: &str, secret: &str) {
    assert!(!out.contains(secret), "{secret:?} survived in {out:?}");
    let start: String = secret.chars().take(12).collect();
    assert!(!out.contains(&start), "{start:?} survived in {out:?}");
}

#[test]
fn pem_block_is_replaced_whole() {
    for kind in ["", "RSA ", "EC ", "OPENSSH ", "ENCRYPTED "] {
        for newline in ["\n", "\r\n", "\\n"] {
            let key = pem(kind, newline);
            let out = redact(&format!("before\n{key}\nafter"));
            assert_eq!(
                out,
                format!("before\n{PRIVATE_KEY}\nafter"),
                "{kind:?} {newline:?}"
            );
        }
    }
    // Inside a JSON string, its newlines escaped.
    let json = format!("{{\"key\":\"{}\",\"n\":1}}", pem("", "\\n"));
    assert_eq!(
        redact(&json),
        format!("{{\"key\":\"{PRIVATE_KEY}\",\"n\":1}}")
    );
    // Two blocks stay two blocks, and the text between them stays.
    let two = format!("{}\nmiddle\n{}", pem("", "\n"), pem("EC ", "\n"));
    assert_eq!(
        redact(&two),
        format!("{PRIVATE_KEY}\nmiddle\n{PRIVATE_KEY}")
    );
}

#[test]
fn a_cut_pem_block_is_still_replaced() {
    let key = pem("RSA ", "\n");
    // A head clip cut its end: everything from the BEGIN marker on goes.
    let end_at = key.find("-----END").unwrap();
    let head = format!("output\n{}", &key[..end_at - 10]);
    let out = redact(&head);
    assert_eq!(out, format!("output\n{PRIVATE_KEY}"));
    // A tail clip cut its start: the body lines before the END marker go.
    let body_at = key.find("FAKE").unwrap() + 8;
    let tail = format!("{}\nerror: done", &key[body_at..]);
    let out = redact(&tail);
    assert_eq!(out, format!("{PRIVATE_KEY}\nerror: done"));
    assert!(!out.contains("FAKE"), "{out:?}");
}

#[test]
fn prefixed_tokens_are_replaced() {
    for token in prefixed() {
        let cases = [
            (token.clone(), TOKEN.to_string()),
            (format!("token={token}&x=1"), format!("token={TOKEN}&x=1")),
            (format!("\"{token}\""), format!("\"{TOKEN}\"")),
            (format!("a {token} b"), format!("a {TOKEN} b")),
            (format!("x\n{token}\ny"), format!("x\n{TOKEN}\ny")),
            (format!("x\r\n{token}\r\ny"), format!("x\r\n{TOKEN}\r\ny")),
            (format!("({token}),{token}"), format!("({TOKEN}),{TOKEN}")),
        ];
        for (input, want) in cases {
            let out = redact(&input);
            assert_eq!(out, want, "{input:?}");
            assert_gone(&out, &token);
        }
    }
}

#[test]
fn fifteen_token_characters_are_not_a_token() {
    // Task 13's review, I1: an unambiguous prefix with four body characters is a token
    // (`ghp_` and `glpat-` moved to the second list); `AKIA`'s body is `[0-9A-Z]`, so
    // lowercase letters are no body.
    for prefix in ["sk-", "AKIA", "npm_"] {
        let short = format!("{prefix}{} tail", "a".repeat(15));
        assert_eq!(redact(&short), short);
    }
    for prefix in ["ghp_", "glpat-"] {
        let short = format!("{prefix}{} tail", "a".repeat(15));
        assert_eq!(redact(&short), format!("{TOKEN} tail"));
    }
}

#[test]
fn a_token_cut_at_a_line_end_is_replaced() {
    // A clip (a signature's headline, decision 5) can end a line inside a token. Task
    // 12's review, I2: not the ambiguous `sk-` and `npm_`, which are judged by their
    // body alone (a clipped one is the ruling's residual gap).
    let ambiguous =
        |t: &str| (t.starts_with("sk-") && !t.starts_with("sk-ant-")) || t.starts_with("npm_");
    // Task 13's review, I1: a cut token keeps at least four body characters
    // (`github_pat_F`, with one, is no longer a token).
    let prefix_len = |t: &str| {
        [
            "github_pat_",
            "sk-ant-",
            "glpat-",
            "xoxa-",
            "xoxb-",
            "xoxp-",
            "xoxr-",
            "xoxs-",
        ]
        .iter()
        .find(|p| t.starts_with(**p))
        .map_or(4, |p| p.len())
    };
    for token in prefixed().into_iter().filter(|t| !ambiguous(t)) {
        for body in [4, 6, 8] {
            let keep = prefix_len(&token) + body;
            let cut: String = token.chars().take(keep).collect();
            for input in [format!("error: {cut}"), format!("error: {cut}\nnext")] {
                let out = redact(&input);
                assert!(
                    out.starts_with(&format!("error: {TOKEN}")),
                    "{input:?} → {out:?}"
                );
            }
        }
    }
}

#[test]
fn a_token_split_by_a_signature_placeholder_is_replaced() {
    // Decision 5 rewrites hex runs to `<sha>` and more; the rest of the token stays.
    for split in [
        format!("ghp_ab<sha>{}", "FAKE".repeat(6)),
        "AKIA12<sha>FAKE".to_string(),
        format!("sk-ant-<id>{}", "fake0".repeat(3)),
    ] {
        let out = redact(&format!("error: {split} rest"));
        assert_eq!(out, format!("error: {TOKEN} rest"), "{split:?}");
    }
}

#[test]
fn jwt_is_replaced() {
    let token = jwt();
    for (input, want) in [
        (token.clone(), TOKEN.to_string()),
        (
            format!("{{\"id_token\":\"{token}\"}}"),
            format!("{{\"id_token\":\"{TOKEN}\"}}"),
        ),
        (
            format!("https://h/cb?jwt={token}&a=b"),
            format!("https://h/cb?jwt={TOKEN}&a=b"),
        ),
    ] {
        let out = redact(&input);
        assert_eq!(out, want, "{input:?}");
        assert_gone(&out, &token);
    }
    // A cut one, its signature gone, still goes.
    let cut = token.rsplit_once('.').unwrap().0.to_string();
    assert_eq!(redact(&format!("x {cut}")), format!("x {TOKEN}"));
}

#[test]
fn bearer_is_replaced() {
    // Task 12's review, I2: a credential needs a digit and 16 characters.
    let opaque = format!("{}{}", "abc.DEF_", "ghi-jkl~mno+pqr/stu0==");
    for (input, want) in [
        (
            format!("Authorization: Bearer {opaque}"),
            format!("Authorization: {BEARER}"),
        ),
        (
            format!("authorization: bearer {opaque}"),
            format!("authorization: {BEARER}"),
        ),
        (
            format!("AUTHORIZATION: BEARER {opaque}\nnext"),
            format!("AUTHORIZATION: {BEARER}\nnext"),
        ),
        (format!("-H 'bearer {opaque}'"), format!("-H '{BEARER}'")),
        // An all-letter credential is the token rule's, not the bearer rule's.
        (format!("Bearer {}", ghp()), format!("Bearer {TOKEN}")),
    ] {
        let out = redact(&input);
        assert_eq!(out, want, "{input:?}");
        assert!(!out.contains(&opaque[..8]), "{out:?}");
    }
    // A word, not a credential; and (task 12's review, I2) a credential with no digit,
    // or a short one, even at a line's end: the ruling's residual gap.
    for prose in [
        "the bearer of bad news",
        "Bearer authentication-required",
        "header: Bearer abc",
        "header: Bearer abc0",
    ] {
        assert_eq!(redact(prose), prose);
    }
}

#[test]
fn url_userinfo_is_replaced() {
    let tok = format!("{}{}", "to", "k");
    for (input, want) in [
        (
            format!("https://user:{tok}@host/x"),
            "https://***@host/x".to_string(),
        ),
        (
            format!(
                "git clone https://x-access-token:{}@github.com/o/r.git",
                ghp()
            ),
            "git clone https://***@github.com/o/r.git".to_string(),
        ),
        (
            format!("ssh://git:{tok}@host:22/r"),
            "ssh://***@host:22/r".to_string(),
        ),
        (
            format!("\"https://u:p@ss{tok}@host/x\""),
            "\"https://***@host/x\"".to_string(),
        ),
        (
            format!("https://token-only{tok}@host"),
            "https://***@host".to_string(),
        ),
        // A clip cut it before its `@`.
        (
            format!("fatal: https://user:{tok}{tok}"),
            "fatal: https://***".to_string(),
        ),
    ] {
        assert_eq!(redact(&input), want, "{input:?}");
    }
    for plain in [
        "https://host/x",
        "https://host:8080/x",
        "see https://host:8080",
        "git@github.com:o/r",
    ] {
        assert_eq!(redact(plain), plain);
    }
}

#[test]
fn ordinary_text_is_unchanged() {
    let sha = "3f2a9c01".repeat(5);
    for text in [
        "error[E0308]: mismatched types\n  --> crates/daemon/src/run/history.rs:95:12\n   |\n95 |     let x: u8 = \"a\";\n",
        &format!("HEAD is now at {sha} fix(daemon): the thing"),
        "/Users/someone/.anthrex/worktrees/app/runs/fix-login-3f2a/t3/src/lib.rs",
        "task-sk-ip and task-sk-ipaaaaaaaaaaaaaaaaaaaaaa",
        "test result: FAILED. 3 passed; 1 failed; 0 ignored",
        "## outcome\n-- END OF DOSSIER --",
        "akiapolitan npm_ ghp_ sk- skating",
        "",
    ] {
        assert_eq!(redact(text), text, "{text:?}");
    }
}

#[test]
fn redaction_is_idempotent() {
    let corpus = format!(
        "{}\n{} {} {}\nAuthorization: Bearer {}\nhttps://u:{}@h/x\n{}\n{} https://***@h",
        pem("RSA ", "\n"),
        ghp(),
        jwt(),
        akia(),
        "z".repeat(30),
        "p".repeat(9),
        xoxb(),
        sk_ant(),
    );
    let once = redact(&corpus);
    assert_eq!(redact(&once), once);
    for secret in [ghp(), jwt(), akia(), xoxb(), sk_ant(), "FAKE".repeat(16)] {
        assert_gone(&once, &secret);
    }
}

/// A JWT shaped as a real one: an `HS256` header, a claim, and a base64url signature
/// with `-` and `_`.
fn real_jwt() -> String {
    let header = format!("{}{}", "eyJhbGciOiJIUzI1NiIs", "InR5cCI6IkpXVCJ9");
    let claim = format!("{}{}", "eyJzdWIiOiJm", "YWtlIn0");
    format!("{header}.{claim}.{}", "FAKE_sig-".repeat(5))
}

/// Task 8's review, I2: an escape right before a token (JSON's `\n`, `\t`, `\r` and
/// `\uXXXX`, a quote, percent-encoding) ends in an alphanumeric character, and must not
/// hide the token.
#[test]
fn a_token_behind_an_escape_is_replaced() {
    let secrets = [ghp(), sk_ant(), akia(), real_jwt()];
    let escapes = [r"\n", r"\t", r"\r", r"\u0020", "\"", "%3D", "%20", r#"\""#];
    for secret in &secrets {
        for escape in escapes {
            for input in [
                format!("{escape}{secret}"),
                format!("left: \"export{escape}{secret}{escape}\""),
                format!("https://x/?q{escape}{secret}&b=1"),
            ] {
                let out = redact(&input);
                assert!(out.contains(TOKEN), "{input:?} -> {out:?}");
                assert_gone(&out, secret);
                assert!(
                    out.contains(escape),
                    "{input:?} -> {out:?}: the escape is kept"
                );
                assert_eq!(redact(&out), out, "{input:?}: idempotent");
            }
        }
    }
    // A bearer credential behind an escape goes too (with a digit: task 12's review, I2).
    let credential = "z0".repeat(15);
    for escape in escapes {
        let input = format!("headers{escape}Bearer {credential}{escape}x");
        let out = redact(&input);
        assert!(out.contains(BEARER), "{input:?} -> {out:?}");
        assert!(!out.contains(&credential), "{input:?} -> {out:?}");
    }
}

/// I2's boundary does not reach ordinary hex: SHAs, percent-encoded or escaped, stay.
#[test]
fn escapes_before_ordinary_hex_are_unchanged() {
    let sha = "3f2a9c01".repeat(5);
    for text in [
        format!(r"commit\n{sha}\t{sha}"),
        format!("%3D{sha}%20{sha}"),
        format!(r"\u0041{sha} \nAKIAno \tsk-short %3Dghp_"),
        r"C:\temp\new\token\sky".to_string(),
    ] {
        assert_eq!(redact(&text), text, "{text:?}");
    }
}

/// Every unambiguous token shape, each with at least 16 token characters.
fn unambiguous() -> Vec<String> {
    let mut tokens: Vec<String> = ["ghp_", "gho_", "github_pat_", "glpat-", "xoxp-"]
        .iter()
        .map(|prefix| format!("{prefix}{}", "FAKE".repeat(5)))
        .collect();
    tokens.extend([ghp(), sk_ant(), akia(), xoxb(), real_jwt()]);
    tokens
}

/// Task 9's review, I2: a text-escaped colour code, a lone text-escaped ESC, JSON's
/// `\b` and `\f`, Python's `\U`, a double percent encoding and a broken one each end in
/// an alphanumeric character right before the token, which must still go.
#[test]
fn a_token_behind_an_encoded_escape_is_replaced() {
    let forms = [
        r"\u001b[31m",
        r"\u{1b}[31m",
        r"\x1b[31m",
        r"\033[31m",
        r"\x1b[1m",
        r"\x1b",
        r"\b",
        r"\f",
        r"\U00000041",
        "%253D",
        "%zz",
        "%2",
        "q=",
        "token",
    ];
    for secret in unambiguous() {
        for form in forms {
            for input in [
                format!("{form}{secret}"),
                format!("{{\"msg\":\"auth {form}{secret}\\u001b[0m\"}}"),
                format!("assert_eq!(left: \"{form}{secret}\")"),
            ] {
                let out = redact(&input);
                assert!(out.contains(TOKEN), "{input:?} -> {out:?}");
                assert_gone(&out, &secret);
                assert!(out.contains(form), "{input:?} -> {out:?}: the form is kept");
                assert_eq!(redact(&out), out, "{input:?}: idempotent");
            }
        }
    }
}

/// The ambiguous prefixes keep their word boundary, and ordinary prose, identifiers
/// and SHAs, escaped or not, stay as they are.
#[test]
fn prose_and_shas_are_not_over_redacted() {
    let sha = "3f2a9c01".repeat(5);
    for text in [
        format!(r"commit\x1b[33m{sha}\x1b[0m %253D{sha} \U00000041{sha}"),
        "task-sk-ipaaaaaaaaaaaaaaaaaaaaaa xnpm_config_cache_directory_x".to_string(),
        // Task 13's review, I1: `laughs_count` and `sighs_total` (`ghs_` and four more)
        // are now tokens; see `the_ruling_costs_these_words`.
        "the laughs_at and sighs fields; AKIAno; eyJ alone; glpat".to_string(),
        "use rustfmt; it laughs_at nothing.\nnext".to_string(),
        // Task 10's review, I1: `\b` is now a boundary, so a short `npm_` at the text's
        // end reads as a clipped token; mid-line it is a word.
        r"\x1bsk-short and \bnpm_ok here".to_string(),
    ] {
        assert_eq!(redact(&text), text, "{text:?}");
    }
}
