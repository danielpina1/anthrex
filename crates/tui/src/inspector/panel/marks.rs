//! Milestone 9.0.7 decision 13: the marks a task section's value carries, coloured
//! only where the client wrote them (`Marks`): `✓` in `Done`, `✗` in `Failed`, `◌` in
//! `Muted`, and the pipeline's current step in `Working` bold. What follows a `Lead`
//! mark is agent text, never coloured, so a criterion cannot forge a tick's colour.
//!
//! Pure like the rest of the panel.

use crate::inspector::Marks;
use crate::theme::{self, Glyph, Palette, Role};
use ratatui::style::Modifier;
use ratatui::text::Span;

/// A mark glyph's role: `✓` `Done`, `✗` `Failed`, `◌` `Muted` (or their twins).
fn mark_role(word: &str, ascii: bool) -> Option<Role> {
    [
        (Glyph::Passed, Role::Done),
        (Glyph::Failed, Role::Failed),
        (Glyph::NotStarted, Role::Muted),
    ]
    .into_iter()
    .find(|(g, _)| theme::glyph(*g, ascii) == word)
    .map(|(_, role)| role)
}

const STEPS: [&str; 5] = ["done", "proof", "check", "review", "merge"];

/// The pipeline's current step: the step word the whole, unwrapped value gives no mark
/// (each step appears once). Judged before wrapping, so a step word that ends a wrapped
/// line, its mark on the next, is not taken for it.
pub(super) fn current_step(value: &str, marks: Marks, ascii: bool) -> Option<String> {
    if marks != Marks::Pipeline {
        return None;
    }
    let words: Vec<&str> = value.split_whitespace().collect();
    let unmarked = |n: usize| {
        words
            .get(n + 1)
            .is_none_or(|w| mark_role(w, ascii).is_none())
    };
    (0..words.len())
        .find(|&n| STEPS.contains(&words[n]) && unmarked(n))
        .map(|n| words[n].to_owned())
}

/// One drawn value line as spans, its marks in their roles (decision 13): a `Lead`
/// mark only among the first two words of a line that begins one of the value's own
/// (`starts`; the caller clears it past the first line for `First`), every pipeline
/// mark, and the `current` step's word.
pub(super) fn marked(
    text: String,
    marks: Marks,
    starts: bool,
    current: Option<&str>,
    p: Palette,
) -> Vec<Span<'static>> {
    let words: Vec<&str> = text.split(' ').collect();
    let role = |word: &str| mark_role(word, p.ascii);
    let lead = (matches!(marks, Marks::Lead | Marks::First) && starts)
        .then(|| words.iter().take(2).position(|w| role(w).is_some()))
        .flatten();
    let style = |n: usize, word: &str| match marks {
        Marks::Lead | Marks::First if lead == Some(n) => role(word).map(|r| theme::role(r, p)),
        Marks::Pipeline => role(word).map(|r| theme::role(r, p)).or_else(|| {
            (current == Some(word))
                .then(|| theme::role(Role::Working, p).add_modifier(Modifier::BOLD))
        }),
        _ => None,
    };
    let (mut out, mut plain) = (Vec::new(), String::new());
    for (n, word) in words.iter().enumerate() {
        if n > 0 {
            plain.push(' ');
        }
        match style(n, word) {
            Some(style) => {
                out.push(Span::raw(std::mem::take(&mut plain)));
                out.push(Span::styled((*word).to_owned(), style));
            }
            None => plain.push_str(word),
        }
    }
    out.push(Span::raw(plain));
    out.retain(|span| !span.content.is_empty());
    out
}
