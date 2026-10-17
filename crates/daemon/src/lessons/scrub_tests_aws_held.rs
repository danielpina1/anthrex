//! Fix round 2 for task 9's review, I1: a text escape whose `\` or `%` is a byte of a
//! real escape sequence (an ST's `\`, an nF intermediate `%`) is kept as text even when
//! it decodes to an ESC or U+009B. Otherwise a secret that starts with `033`, `233`,
//! `x1b`, `u001b`, `1B` or `9b` right after such an escape becomes an ESC or a CSI that
//! eats the secret's next characters, and 34 to 38 of them are left. Every key-shaped
//! input is built at run time (ruling C9).

use super::scrub;
use super::scrub_tests_aws_property::{CONTEXTS, headed, longest_leak, place};
use super::scrub_tests_residuals::aws_secret;

/// An escape whose last byte is a `\` or a `%`, and the secret's heads that would make
/// a text escape with it.
fn shapes() -> Vec<(&'static str, [&'static str; 8])> {
    let after_st = ["033", "0033", "233", "x1b", "x1B", "x9b", "u001b", "u009b"];
    let after_percent = ["1B", "1b", "9B", "9b", "1B3", "1b/", "9b+", "9BQ"];
    vec![
        // A lone ST, an OSC ended by ST, a DCS and an APC ended by ST.
        ("\u{1b}\\", after_st),
        ("\u{1b}]0;title\u{1b}\\", after_st),
        ("\u{1b}P1$r\u{1b}\\", after_st),
        ("\u{1b}_x\u{1b}\\", after_st),
        // nF intermediates with no final: the secret's first character is the final.
        ("\u{1b}%", after_percent),
        ("\u{1b})%", after_percent),
        ("\u{1b} %", after_percent),
    ]
}

#[test]
fn a_held_text_escape_that_spells_an_esc_or_a_csi_stays_text() {
    let secret = aws_secret();
    for (escape, heads) in shapes() {
        for head in heads {
            let s = headed(&secret, head);
            for context in 0..CONTEXTS {
                let input = place(context, &format!("{escape}{s}"));
                let out = scrub(&input);
                let leak = longest_leak(&out, &s);
                assert!(leak < 8, "{input:?} -> {out:?} leaks {leak}");
                assert_eq!(scrub(&out), out, "idempotent on {input:?}");
            }
        }
    }
}
