//! The tree's row labels and their cuts, split out of `tree.rs` per the 600-line rule.

use proto::{Runtime, SubagentInfo};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn short_model(runtime: Runtime, model: &str) -> String {
    let model = match runtime {
        Runtime::Claude => {
            let stripped = model.strip_prefix("claude-").unwrap_or(model);
            let numeric_suffix = stripped.char_indices().find_map(|(index, character)| {
                (character == '-'
                    && stripped[index + character.len_utf8()..]
                        .chars()
                        .next()
                        .is_some_and(|next| next.is_ascii_digit()))
                .then_some(index)
            });
            &stripped[..numeric_suffix.unwrap_or(stripped.len())]
        }
        Runtime::Codex | Runtime::Shell => model,
    };
    cut_to_width(model, 8)
}

fn cut_to_width(text: &str, max_width: usize) -> String {
    let mut end = 0;
    for (index, grapheme) in text.grapheme_indices(true) {
        let candidate_end = index + grapheme.len();
        if UnicodeWidthStr::width(&text[..candidate_end]) > max_width {
            break;
        }
        end = candidate_end;
    }
    text[..end].to_owned()
}

/// The text a sub-agent row shows in its name column: `kind: label` when a
/// label was set, `kind` alone otherwise.
///
/// Shared by the sidebar and the graph overview's layout and painter, so the
/// string a tier is sized to and the string drawn inside it can never drift
/// apart into two definitions.
pub fn subagent_label(info: &SubagentInfo) -> String {
    match info.label.as_deref() {
        Some(label) => format!("{}: {label}", info.kind),
        None => info.kind.clone(),
    }
}

pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}
