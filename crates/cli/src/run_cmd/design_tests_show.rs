//! The final fix wave's `run show` items: ruling T16-3 (FW-48, the stored bytes when
//! stdout is not a terminal) and WB-C M-4 (FW-71, a review draft's header). Pure.

use proto::{DocKind, DocView};

use super::show::show_text;
use super::tests::{ID, design_run, spec_v2};

/// Ruling T16-3: piped or redirected, `run show` writes the stored text exactly (a TAB,
/// a `\r\n` and a ZWJ included), so `show > f; edit-doc --file f` round-trips byte for
/// byte; on a terminal the text is made printable. The header on stderr is printable
/// either way.
#[test]
fn show_sanitises_only_on_a_terminal() {
    let info = design_run();
    let text = "# Reset\r\n\tR1 reset\u{200d}by mail\x1b[2J\n";
    let doc = DocView {
        text: text.into(),
        ..spec_v2()
    };
    let (piped, err) = show_text(&info, &doc, (false, false), false);
    assert_eq!(piped, text);
    assert!(!err.contains('\x1b'));
    let (shown, _) = show_text(&info, &doc, (false, false), true);
    assert_eq!(shown, "# Reset\n R1 resetby mail [2J\n");
    // The sections asked for after it follow the same rule.
    let doc = DocView {
        diff: Some("+\tR1\r\n".into()),
        ..doc
    };
    let (piped, _) = show_text(&info, &doc, (true, false), false);
    assert_eq!(piped, format!("{text}=== diff against v1 ===\n+\tR1\r\n"));
}

/// WB-C M-4: a review draft (version 0, ruling T5-1) is headed `draft r<k>`, never
/// "v0 of 0", and its diff is against the draft before it, with no `v0` arithmetic.
#[test]
fn show_heads_a_review_draft_by_its_review() {
    let mut info = design_run();
    info.doc_gate = None;
    info.docs.clear();
    let doc = DocView {
        version: 0,
        draft_review: Some(2),
        diff: Some("+R2 The link expires.\n".into()),
        ..spec_v2()
    };
    let (out, err) = show_text(&info, &doc, (true, false), true);
    assert_eq!(err, format!("Spec · run {ID} · draft r2\n"));
    assert_eq!(
        out,
        "# Reset\n=== diff against the previous review draft ===\n+R2 The link expires.\n"
    );
    let first = DocView {
        diff: None,
        draft_review: Some(1),
        ..doc
    };
    let (out, err) = show_text(&info, &first, (true, false), true);
    assert_eq!(err, format!("Spec · run {ID} · draft r1\n"));
    assert!(
        out.ends_with("=== no earlier version to diff against ===\n"),
        "{out}"
    );
    assert_eq!(DocKind::Spec, first.kind);
}
