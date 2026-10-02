//! The design kit's widgets (milestone 9.0.6 decisions 4, 5, 21): every new dialog,
//! form and screen draws hints, labelled rows, choices, text areas and frames through
//! these, so the grammar stays one grammar. Each widget takes already-built strings and
//! sanitises its text inputs with `safe_text::one_line` itself, so a screen cannot
//! forget. Pure: no I/O, no clock (`AGENTS.md` hard rule 5).

mod dialog;
mod hints;
mod pane;
mod rows;
mod screen;
mod text;

pub use dialog::{choice, choice_in, dialog_area, dialog_frame};
pub use hints::{Hint, destructive, hints, hints_joined};
pub use pane::{pane_frame, selection_bar};
pub use rows::{labelled_rows, run_name, run_name_in, scroll_marks};
pub(crate) use screen::{from_top, from_top_until, last_top, window_span};
pub use screen::{screen_frame, window};
pub use text::{cursor_block, text_area, text_area_focus};
pub(crate) use text::{cut, starts_with_mark, wrap_words};

/// Dialog text wraps at this many columns (decision 5).
pub const WRAP: u16 = 60;
/// A dialog is at most this wide, and exactly the area's width when narrower.
pub const DIALOG_MAX: u16 = 64;

#[cfg(test)]
mod tests;
