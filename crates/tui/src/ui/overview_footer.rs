//! The overview's single line under the canvas (milestone 4.6), moved out of
//! `ui/overview.rs` to keep it within its bound (final fix wave).

use super::tree_view;
use crate::app::App;
use crate::inspector;
use crate::theme;
use crate::tree::{self, Row, RowKind};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// The selected node in full: its label untruncated, then its model, its state
/// and how long it has been in it (decision 18).
///
/// Only the line's own width cuts anything here, which is why the box above can
/// afford to elide: whatever a box hides, this line shows. The panel replaces
/// it (decision 1); it is what `i` turns back on and what a short terminal
/// keeps (decisions 6 and 7).
pub(super) fn footer_line(row: &Row<'_>, app: &App) -> Line<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let (glyph, label, fields) = footer_parts(row, app);
    let p = app.palette();
    Line::from(vec![
        glyph,
        Span::raw(" "),
        Span::styled(theme::fold(&label, p.ascii), bold),
        Span::styled(
            theme::fold(&fields, p.ascii),
            theme::role(theme::Role::Muted, p),
        ),
    ])
}

/// One node's footer: its status glyph in its status colour, the text it is
/// known by, and the fields that follow it.
pub(in crate::ui) fn footer_parts(row: &Row<'_>, app: &App) -> (Span<'static>, String, String) {
    let (frame, ascii) = (app.spinner_frame, app.palette().ascii);
    let look = |look| crate::inspector::look(look, app);
    match &row.kind {
        RowKind::Project {
            root,
            name,
            status,
            counts,
            ..
        } => {
            let root = crate::ui::terminal::shorten_home(root);
            let root = if root == "~/" { "~" } else { &root };
            (
                look(theme::status_look(*status, frame, ascii)),
                name.clone(),
                format!(
                    "  {root}  {}  {}",
                    status.label(),
                    tree_view::counts_text(*counts)
                ),
            )
        }
        RowKind::Window { info, position, .. } => (
            look(theme::status_look(info.status, frame, ascii)),
            format!("{position} {}", info.name),
            format!(
                "  {}  {}  {}  {}{}",
                info.runtime.label(),
                info.model.as_deref().unwrap_or("-"),
                info.status.label(),
                tree::format_elapsed(app.elapsed_secs(info)),
                info.tool
                    .as_deref()
                    .map(|tool| format!("  {tool}"))
                    .unwrap_or_default()
            ),
        ),
        RowKind::Subagent { info } => {
            let (state, duration) = match info.state {
                proto::SubagentState::Running => ("running", app.age_secs(info.started_secs)),
                proto::SubagentState::Done => ("done", finished_secs(info)),
                proto::SubagentState::Failed => ("failed", finished_secs(info)),
            };
            (
                look(theme::subagent_look(info, frame, ascii)),
                tree::subagent_label(info),
                format!(
                    "  {}  {state}  {}{}",
                    info.model.as_deref().unwrap_or("-"),
                    tree::format_elapsed(duration),
                    info.tool
                        .as_deref()
                        .map(|tool| format!("  {tool}"))
                        .unwrap_or_default()
                ),
            )
        }
        // Every run kind's single line is its inspection's title: the glyph, the
        // name, then two spaces and the right-hand text (milestone 8c, Interfaces
        // "The single line"). The glyph is the canvas's own, by construction.
        RowKind::Run { .. }
        | RowKind::Planner { .. }
        | RowKind::Scout { .. }
        | RowKind::Task { .. }
        | RowKind::Stage { .. }
        | RowKind::Round { .. }
        | RowKind::IdleOrchestrator { .. }
        | RowKind::AgentRound { .. } => {
            let inspection = inspector::inspect(row, app);
            let right = inspection
                .right
                .map(|right| format!("  {right}"))
                .unwrap_or_default();
            (inspection.glyph, inspection.name, right)
        }
    }
}

/// How long a sub-agent that has stopped ran for. Both fields are ages, so the
/// run is the difference between them, and a missing end reads as zero.
fn finished_secs(info: &proto::SubagentInfo) -> u64 {
    info.ended_secs
        .map_or(0, |ended| info.started_secs.saturating_sub(ended))
}
