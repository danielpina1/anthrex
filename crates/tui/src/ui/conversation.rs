//! The conversation view's rendering (task M6.5.13, spec §6). Takes `&App`, does no I/O.

pub mod diff;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::Rect;

pub fn render(_frame: &mut Frame, _app: &App, _area: Rect) {}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
