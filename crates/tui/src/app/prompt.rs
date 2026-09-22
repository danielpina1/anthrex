//! The rename prompt (task M6.10): pure state for `Modal::Rename`, and decision 22's
//! name check run client-side before a `Rename` is sent, so a bad name shows inline in
//! the box instead of round-tripping to the daemon for a toast. No I/O (`AGENTS.md`
//! rule 5, the same discipline `dialog.rs` follows for the new-agent form) — opening,
//! submitting and closing the modal is wired in `app/mod.rs` and `app/modal_keys.rs`,
//! the same split `dialog.rs` and `app/modal_keys.rs` already give that form.
//!
//! `crates/tui` does not depend on `crates/daemon` (no I/O crate does, by design), so
//! this cannot call `daemon::manager::validate_name` directly. The rule is instead kept
//! here in full, character for character: if it ever changes, both copies have to be
//! changed together, the same trade the daemon crate itself makes between
//! `validate_name` (reject) and `sanitize_name` (repair) sharing `is_disallowed_name_char`
//! rather than each inlining the predicate.

use crate::dialog::{TextInput, apply_text_key};
use crossterm::event::{KeyCode, KeyEvent};
use unicode_segmentation::UnicodeSegmentation;

/// Design decision 22's limit, counted in grapheme clusters — see `validate`'s doc
/// comment for why that unit and not `char`s or bytes.
pub const NAME_MAX_CHARS: usize = 64;

/// `Modal::Rename`'s pure state: which window is being renamed, the box's own text
/// field, and the inline error decision 22's client-side check sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePrompt {
    pub window_id: u32,
    pub input: TextInput,
    pub error: Option<String>,
}

impl RenamePrompt {
    /// Opens prefilled with the window's current name, cursor at the end — the same
    /// starting point `TextInput::new` gives every other field.
    pub fn new(window_id: u32, name: &str) -> Self {
        Self {
            window_id,
            input: TextInput::new(name),
            error: None,
        }
    }
}

/// What a key press does to an open rename prompt. `Submit` hands back the raw
/// (untrimmed, unvalidated) text: `prompt.rs` has no view of the other windows'
/// names, so `app/modal_keys.rs` is the one that calls `validate` and decides whether
/// to send the `Rename` or put the modal back with an error.
pub enum RenameOutcome {
    Cancel,
    Stay,
    Submit(String),
}

/// `Esc` cancels, `Enter` submits whatever text is in the box, and every other key is
/// the text field's own editing (`dialog::apply_text_key`, shared with the new-agent
/// form's fields).
pub fn on_key(prompt: &mut RenamePrompt, key: KeyEvent) -> RenameOutcome {
    match key.code {
        KeyCode::Esc => RenameOutcome::Cancel,
        KeyCode::Enter => RenameOutcome::Submit(prompt.input.text().to_string()),
        _ => {
            apply_text_key(&mut prompt.input, key);
            RenameOutcome::Stay
        }
    }
}

/// Decision 22's bidi text-direction override characters, refused with the same
/// message as a control character (fix wave 6's amendment: `char::is_control()` alone,
/// Unicode category `Cc`, lets U+202E RIGHT-TO-LEFT OVERRIDE through, because it is
/// category `Cf` ("format") — the "Trojan Source" spoofing class this milestone's
/// daemon-side `validate_name` was amended to close, mirrored here so the client
/// refuses the same input rather than sending it and letting the daemon's `Error`
/// arrive as a toast instead of an inline error).
fn is_bidi_override(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The full set of characters decision 22 refuses in a name, mirroring
/// `daemon::manager::is_disallowed_name_char`: the Unicode `Cc` control category plus
/// the bidi overrides above. `Cf` also holds U+200D ZERO WIDTH JOINER, which a
/// multi-codepoint emoji sequence needs to fuse into the one grapheme cluster `validate`
/// counts against the 64-character limit, so the whole `Cf` category is not rejected —
/// only the bidi overrides are.
fn is_disallowed_name_char(c: char) -> bool {
    c.is_control() || is_bidi_override(c)
}

/// Decision 22's client-side check. `window_id` is the window being renamed, excluded
/// from its own duplicate-name check so renaming a window to the name it already has
/// succeeds; `windows` is every other window's `(id, name)` this client currently knows
/// about.
///
/// The 64-character limit is counted in grapheme clusters
/// (`UnicodeSegmentation::graphemes`), the same unit `dialog.rs`, `tree.rs` and
/// `tree_input.rs` already count user-facing text in, so a multi-codepoint glyph (an
/// accented letter, a flag, a family emoji fused by ZWJ) counts as the one character a
/// person typing it perceives, not as however many Unicode scalar values encode it.
///
/// The duplicate-name message is decision 23's own wording (`ui/modal.rs`'s rename box
/// mock), not the daemon's `"a window named '<name>' already exists"` — a different,
/// shorter phrasing for the same rule shown in a different place, and binding as written
/// in the task M6.10 brief.
pub fn validate<'a>(
    name: &str,
    window_id: u32,
    windows: impl Iterator<Item = (u32, &'a str)>,
) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if trimmed.graphemes(true).count() > NAME_MAX_CHARS {
        return Err(format!("name must be at most {NAME_MAX_CHARS} characters"));
    }
    if trimmed.chars().any(is_disallowed_name_char) {
        return Err("name must not contain control characters".to_string());
    }
    if windows
        .into_iter()
        .any(|(id, existing)| id != window_id && existing == trimmed)
    {
        return Err(format!("a window named '{trimmed}' exists"));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows<'a>(pairs: &'a [(u32, &'a str)]) -> impl Iterator<Item = (u32, &'a str)> {
        pairs.iter().copied()
    }

    #[test]
    fn trims_and_accepts_a_plain_name() {
        assert_eq!(
            validate("  api-worker  ", 1, windows(&[])),
            Ok("api-worker".to_string())
        );
    }

    #[test]
    fn renaming_a_window_to_its_own_current_name_succeeds() {
        assert_eq!(
            validate("api", 1, windows(&[(1, "api"), (2, "billing")])),
            Ok("api".to_string())
        );
    }

    #[test]
    fn empty_and_all_spaces_are_refused() {
        assert_eq!(
            validate("", 1, windows(&[])),
            Err("name must not be empty".to_string())
        );
        assert_eq!(
            validate("   ", 1, windows(&[])),
            Err("name must not be empty".to_string())
        );
    }

    #[test]
    fn sixty_five_characters_is_refused_but_sixty_four_is_not() {
        let sixty_four = "a".repeat(64);
        assert_eq!(validate(&sixty_four, 1, windows(&[])), Ok(sixty_four));
        let sixty_five = "a".repeat(65);
        assert_eq!(
            validate(&sixty_five, 1, windows(&[])),
            Err("name must be at most 64 characters".to_string())
        );
    }

    /// The 64-character limit is grapheme clusters, not `char`s: a family emoji fused
    /// by ZWJ is one glyph but several `char`s, and must still fit at exactly 64 of
    /// them repeated (mirrors the daemon-side test this amendment was written for).
    #[test]
    fn the_limit_counts_grapheme_clusters_not_chars() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"; // man+ZWJ+woman+ZWJ+girl+ZWJ+boy
        let sixty_four_emoji = family.repeat(64);
        assert_eq!(
            validate(&sixty_four_emoji, 1, windows(&[])),
            Ok(sixty_four_emoji)
        );
    }

    #[test]
    fn a_duplicate_name_is_refused_with_decision_23s_wording() {
        assert_eq!(
            validate("api", 1, windows(&[(2, "api")])),
            Err("a window named 'api' exists".to_string())
        );
    }

    #[test]
    fn control_characters_and_bidi_overrides_are_refused() {
        assert_eq!(
            validate("bad\x1bname", 1, windows(&[])),
            Err("name must not contain control characters".to_string())
        );
        assert_eq!(
            validate("bad\u{202e}name", 1, windows(&[])),
            Err("name must not contain control characters".to_string())
        );
    }
}
