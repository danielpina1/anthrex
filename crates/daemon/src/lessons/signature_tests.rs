//! Task M9.15.2: decision 5's signatures.

use sha2::{Digest, Sha256};

use super::{HEADLINE_MAX, SIG_LINES, of};

/// Decision 5 step 4's fingerprint of `lines`, as the test expects it.
fn fp_of(lines: &[&str]) -> String {
    let digest = Sha256::digest(lines.join("\n").as_bytes());
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..16]
        .to_string()
}

fn failure(path: &str, at: &str, took: &str, commit: &str, message: &str) -> String {
    format!(
        "   Compiling app v0.1.0 ({path})\n\
         error[E0308]: {message} at {path}{at}\n  --> {path}{at}\n\
         assertion failed at commit {commit}\n\
         test x::y ... FAILED in {took}\n"
    )
}

#[test]
fn signature_strips_worktree_ids_linecol_durations() {
    let a = failure(
        "/tmp/a/runs/fix-login-3f2a/t3/src/x.rs",
        ":12:7",
        "0.42s",
        &"3f2a9c01".repeat(5),
        "mismatched types",
    );
    let b = failure(
        "/Users/b/.anthrex/worktrees/app/runs/add-cache-9c01/t12/src/x.rs",
        ":40:2",
        "13ms",
        &"e1d0b7a4".repeat(5),
        "mismatched types",
    );
    let c = failure(
        "/tmp/a/runs/fix-login-3f2a/t3/src/x.rs",
        ":12:7",
        "0.42s",
        &"3f2a9c01".repeat(5),
        "cannot find value `y` in this scope",
    );
    let (a, b, c) = (of(&a).unwrap(), of(&b).unwrap(), of(&c).unwrap());
    assert_eq!(a.fp, b.fp, "{a:?} {b:?}");
    assert_ne!(a.fp, c.fp);
    assert_eq!(a.fp.len(), 16);
    assert!(a.fp.chars().all(|ch| ch.is_ascii_hexdigit()), "{}", a.fp);
    assert_eq!(
        a.headline,
        "error[E0308]: mismatched types at <worktree>/src/x.rs:<l>:<c>"
    );
    // Only the commit, the duration and the checkout differ between them.
    assert_eq!(
        a.fp,
        fp_of(&[
            "error[E0308]: mismatched types at <worktree>/src/x.rs:<l>:<c>",
            "assertion failed at commit <sha>",
            "test x::y ... FAILED in <dur>",
        ])
    );
}

/// Ruling C10 (b): a 9.14 pool directory varies per lease.
#[test]
fn signature_strips_target_pool_directories() {
    let a = "error: could not write /d/repos/app-1/target-pool/a3/debug/deps/x.rmeta";
    let b = "error: could not write /e/repos/app-2/target-pool/c9/debug/deps/x.rmeta";
    let (a, b) = (of(a).unwrap(), of(b).unwrap());
    assert_eq!(a, b);
    assert_eq!(
        a.headline,
        "error: could not write <target>/debug/deps/x.rmeta"
    );
}

#[test]
fn signature_keeps_three_decisive_lines() {
    let long = format!("error: {}", "word ".repeat(60));
    let mut lines = vec![
        "   Compiling app v0.1.0 (/src/app)".to_string(),
        "    Finished `test` profile [unoptimized] target(s)".into(),
        "     Running unittests src/lib.rs (target/debug/deps/app)".into(),
        "running 4 tests".into(),
        "test a::one ... ok".into(),
        "test a::two ... ok".into(),
        long.clone(),
        "test a::three ... FAILED".into(),
        "thread 'a::four' panicked at src/lib.rs".into(),
        "error: test failed, to rerun pass `--lib`".into(),
        "caused an error in the end".into(),
        "test result: FAILED. 2 passed; 2 failed".into(),
    ];
    while lines.len() < 30 {
        lines.push(format!("stdout line {}", lines.len()));
    }
    let output = lines.join("\n");
    let sig = of(&output).unwrap();
    let want = [
        long.trim_end(),
        "test a::three ... FAILED",
        "thread 'a::four' panicked at src/lib.rs",
    ];
    assert_eq!(want.len(), SIG_LINES);
    assert_eq!(sig.fp, fp_of(&want));
    assert_eq!(sig.headline.chars().count(), HEADLINE_MAX);
    assert!(long.starts_with(&sig.headline), "{}", sig.headline);
}

#[test]
fn signature_falls_back_to_last_three_lines() {
    let output = "   Compiling app v0.1.0\nalpha\nbeta\ngamma\ndelta\n    Finished dev\n";
    let sig = of(output).unwrap();
    assert_eq!(sig.fp, fp_of(&["beta", "gamma", "delta"]));
    assert_eq!(sig.headline, "beta");
}

#[test]
fn signature_none_for_empty_or_boilerplate_output() {
    for output in [
        "",
        "  \n\t\n   ",
        "   Compiling app v0.1.0 (/src/app)\n    Finished `dev` profile in 3.2s\n",
    ] {
        assert_eq!(of(output), None, "{output:?}");
    }
    // Review M1: the same boilerplate with one line of its own is a signature.
    let sig = of("   Compiling app v0.1.0 (/src/app)\nboom\n    Finished `dev` profile in 3.2s\n")
        .expect("a line survives the skips");
    assert_eq!(sig.headline, "boom");
    assert_eq!(sig.fp, fp_of(&["boom"]));
}

/// Review I1 (task M9.15.2): Rust 1.99's panic line carries the thread id, often under
/// the hex rule's 7 characters.
#[test]
fn signature_ignores_panic_thread_ids() {
    let panic = |tid: &str| {
        format!("thread 'a::b' ({tid}) panicked at src/lib.rs:12:5:\nassertion failed: x\n")
    };
    let (a, b) = (of(&panic("201395")).unwrap(), of(&panic("4242")).unwrap());
    assert_eq!(a, b);
    assert_eq!(
        a.headline,
        "thread 'a::b' (<tid>) panicked at src/lib.rs:<l>:<c>:"
    );
    assert_eq!(a, of(&panic("12724030")).unwrap());
}

/// Review I2 (task M9.15.2): ISO-8601 timestamps become `<time>` before the
/// line:column rule sees their `:mm:ss`.
#[test]
fn signature_ignores_iso_timestamps() {
    let logged = |at: &str| format!("[{at} ERROR app] connection failed\n");
    let stamps = [
        "2026-10-13T12:34:56Z",
        "2027-01-02T03:04:05Z",
        "2026-10-13T12:34:56.123456Z",
        "2026-10-13T12:34:56+02:00",
        "2026-10-13 23:59:59.5-07:00",
        "2026-10-13T12:34:56",
    ];
    let first = of(&logged(stamps[0])).unwrap();
    assert_eq!(first.headline, "[<time> ERROR app] connection failed");
    for at in stamps {
        assert_eq!(of(&logged(at)).unwrap(), first, "{at}");
    }
    // A location still becomes `<l>:<c>`.
    let located = of("error at src/file.rs:12:5 on 2026-10-13T12:34:56Z\n").unwrap();
    assert_eq!(located.headline, "error at src/file.rs:<l>:<c> on <time>");
}

#[test]
fn signature_drops_escapes_and_bidi() {
    let plain = "error: the build failed\nsee the log";
    let dressed =
        "\x1b[1m\x1b[31merror\x1b[0m: the \u{202E}build failed\x1b]8;;http://x\x07\nsee the log";
    let (plain, dressed) = (of(plain).unwrap(), of(dressed).unwrap());
    assert_eq!(plain, dressed);
    for bad in ['\x1b', '\u{202E}', '['] {
        assert!(!dressed.headline.contains(bad), "{:?}", dressed.headline);
    }
}

fn ghp(fill: &str) -> String {
    format!("ghp_{}", fill.repeat(9))
}

/// The controller's ruling (task M9.15.9; task 8's review, I3): a headline is redacted
/// before its clip, so it never carries a token, nor a recognisable piece of one, and
/// the fingerprint is taken over the redacted lines: the same failure keeps its `fp`,
/// whatever token it printed.
#[test]
fn a_headline_never_carries_a_token() {
    for token in [ghp("FAKE"), ghp("FALS")] {
        // The token straddles the 160-character clip; a colour code and a JSON escape
        // stand right before the others.
        // "error: auth failed: " is 20 characters: the token starts at 149.
        let pad = "p".repeat(HEADLINE_MAX - 32);
        let outputs = [
            format!("error: auth failed: {pad} {token}\n"),
            format!("error: auth failed for \x1b[1;31m{token}\x1b[0m\n"),
            format!("error: {{\"msg\":\"denied\\n{token}\"}}\n"),
            // Task 9's review, I1: nF escapes and a lone ESC before the token.
            format!("error: token: \x1b(B\x1b[m{token}\n"),
            format!("error: \x1b#8{token}\n"),
            format!("error: \x1b{token}\n"),
            // Task 9's review, I2: text-escaped colour codes and other escapes that end
            // in an alphanumeric character.
            format!("error: {{\"msg\":\"\\u001b[31m{token}\"}}\n"),
            format!("error: left: \"\\x1b[31m{token}\"\n"),
            format!("error: + \\033[31m{token}\n"),
            format!("error: redirect_uri=x%253D{token}\n"),
            format!("error: \\U00000041{token}\n"),
        ];
        for output in outputs {
            let sig = of(&output).expect("a signature");
            for fragment in ["ghp_", "FAKEFAKE", "FALSFALS"] {
                assert!(!sig.headline.contains(fragment), "{output:?} -> {sig:?}");
            }
            assert!(sig.headline.chars().count() <= HEADLINE_MAX);
        }
    }
    // The fingerprint is stable: the same failure twice, and with another token.
    let at = |token: &str| of(&format!("error: auth failed for {token}\n")).unwrap();
    let (a, b) = (at(&ghp("FAKE")), at(&ghp("FALS")));
    assert_eq!(a, at(&ghp("FAKE")));
    assert_eq!(a.fp, b.fp);
    assert_eq!(a.headline, "error: auth failed for [redacted token]");
    assert_eq!(a.fp, fp_of(&["error: auth failed for [redacted token]"]));
}
