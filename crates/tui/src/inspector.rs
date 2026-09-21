//! The node inspector: everything anthrex knows about the selected node, in a
//! bordered panel below the graph overview's canvas (spec §3).
//!
//! Two halves, tested separately (decision 13). This is the first: `inspect`
//! projects a `Row` and the `App` behind it into an `Inspection` — labels and
//! values, and nothing that knows about a terminal — asserted by exact field
//! lists. The renderer that lays an `Inspection` out follows.

use crate::app::App;
use crate::theme;
use crate::tree::{self, NodeKey, Row, RowKind, RuntimeCounts};
use crate::ui::statusbar::git_spans;
use crate::ui::terminal::shorten_home;
use crate::ui::tree_view::counts_text;
use proto::{GitState, Head, Status, SubagentInfo, SubagentState, WindowInfo};
use ratatui::style::Style;
use ratatui::text::Span;
use std::collections::BTreeSet;
use std::path::Path;

/// The panel's own height: a border, the title row, five field rows, a border
/// (decision 2).
pub const INSPECTOR_HEIGHT: u16 = 8;

/// Below this many rows of overview interior the panel gives way to milestone
/// 4.6's single line, so a short terminal loses the inspector and never the
/// canvas (decision 6).
pub const MIN_INTERIOR_FOR_PANEL: u16 = INSPECTOR_HEIGHT + 6;

/// One labelled value. `wrap` marks the one field a column may not elide: the
/// sub-agent's task, which the panel exists to show whole (decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub wrap: bool,
}

/// One node, projected: its status glyph in its status colour, the name it is
/// known by, and its fields in the order decisions 8 to 10 give them.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub glyph: Span<'static>,
    pub name: String,
    pub fields: Vec<Field>,
}

fn field(label: &'static str, value: impl Into<String>) -> Field {
    Field {
        label,
        value: value.into(),
        wrap: false,
    }
}

/// Everything anthrex knows about one node. Pure: it reads `app`, and computes
/// strings (decision 13).
pub fn inspect(row: &Row<'_>, app: &App) -> Inspection {
    match &row.kind {
        RowKind::Project {
            root,
            name,
            status,
            counts,
            ..
        } => Inspection {
            glyph: status_span(*status, app),
            name: name.clone(),
            fields: project_fields(root, *status, *counts, app),
        },
        RowKind::Window { info, position, .. } => Inspection {
            glyph: status_span(info.status, app),
            name: format!("{position} {}", info.name),
            fields: window_fields(info, app),
        },
        RowKind::Subagent { info } => Inspection {
            glyph: Span::styled(
                theme::subagent_glyph(info, app.spinner_frame),
                Style::default().fg(theme::subagent_color(info)),
            ),
            name: tree::subagent_label(info),
            fields: subagent_fields(row, info, app),
        },
    }
}

fn status_span(status: Status, app: &App) -> Span<'static> {
    Span::styled(
        theme::status_glyph(status, app.spinner_frame),
        Style::default().fg(theme::status_color(status)),
    )
}

/// A project's path, status and runtime counts, and its git state only when
/// every window in it stands in one worktree (decision 8).
fn project_fields(root: &Path, status: Status, counts: RuntimeCounts, app: &App) -> Vec<Field> {
    let mut fields = vec![
        field("path", display_path(root)),
        field("status", status.label()),
        field("agents", counts_text(counts)),
    ];
    // Ordered and deduplicated: two windows in the same worktree are one
    // worktree, and the count below has to say so.
    let worktrees: BTreeSet<&Path> = app
        .windows
        .iter()
        .filter(|window| window.project == root)
        .filter_map(|window| window.worktree.as_deref())
        .collect();
    match worktrees.len() {
        0 => {}
        // Picking one of several arbitrarily would be a lie, so the field
        // degrades to a count and `changes` is left out entirely.
        1 => {
            if let Some(state) = worktrees.iter().next().and_then(|root| app.git.get(*root)) {
                fields.push(field("branch", head_text(&state.head)));
                fields.push(field("changes", changes_text(state)));
            }
        }
        count => fields.push(field("branch", format!("{count} worktrees"))),
    }
    fields
}

/// A window's runtime, model, status and timings, where it stands, and what it
/// has running under it (decision 9).
fn window_fields(info: &WindowInfo, app: &App) -> Vec<Field> {
    let mut fields = vec![
        field("runtime", info.runtime.label()),
        field("model", info.model.as_deref().unwrap_or("-")),
        field(
            "status",
            with_tool(info.status.label(), info.tool.as_deref()),
        ),
        field("for", tree::format_elapsed(app.elapsed_secs(info))),
        field("dir", display_path(&info.cwd)),
    ];
    if let Some(worktree) = info.worktree.as_deref()
        && worktree != info.cwd
    {
        fields.push(field("worktree", display_path(worktree)));
    }
    // Keyed by the worktree root, and absent state omits the field rather than
    // showing it empty (decision 12).
    if let Some(state) = info.worktree.as_deref().and_then(|root| app.git.get(root)) {
        fields.push(field("branch", git_text(state)));
    }
    if let Some(session) = info.session_id.as_deref() {
        fields.push(field("session", session));
    }
    if !info.subagents.is_empty() {
        let running = info
            .subagents
            .iter()
            .filter(|subagent| subagent.state == SubagentState::Running)
            .count();
        fields.push(field(
            "sub-agents",
            format!("{}, {running} running", info.subagents.len()),
        ));
    }
    fields
}

/// A sub-agent's kind, task, model, state, timing, parentage and depth
/// (decision 10). The task is the one field marked for wrapping.
fn subagent_fields(row: &Row<'_>, info: &SubagentInfo, app: &App) -> Vec<Field> {
    let (state, duration) = match info.state {
        SubagentState::Running => ("running", app.age_secs(info.started_secs)),
        SubagentState::Done => ("done", finished_secs(info)),
        SubagentState::Failed => ("failed", finished_secs(info)),
    };
    let mut fields = vec![field("kind", info.kind.clone())];
    if let Some(label) = info.label.as_deref() {
        fields.push(Field {
            label: "task",
            value: label.to_owned(),
            wrap: true,
        });
    }
    fields.push(field("model", info.model.as_deref().unwrap_or("-")));
    fields.push(field("state", with_tool(state, info.tool.as_deref())));
    fields.push(field("for", tree::format_elapsed(duration)));
    fields.push(field("spawned by", spawned_by(row, info, app)));
    // A window's own sub-agents sit at row depth 2, and one level below the
    // window is what "depth" means here.
    fields.push(field("depth", row.depth.saturating_sub(1).to_string()));
    fields
}

/// The parent sub-agent's label when `parent_id` names one still in the
/// window's list, and the owning window otherwise — including when `parent_id`
/// names a sub-agent that has since gone (decision 11).
///
/// This is the field the milestone exists for: a sub-agent three levels down
/// looks exactly like one directly under its window.
fn spawned_by(row: &Row<'_>, info: &SubagentInfo, app: &App) -> String {
    let NodeKey::Subagent { window_id, .. } = &row.key else {
        return "-".to_owned();
    };
    let Some(window) = app.windows.iter().find(|window| window.id == *window_id) else {
        return "-".to_owned();
    };
    if let Some(parent) = info.parent_id.as_deref().and_then(|parent_id| {
        window
            .subagents
            .iter()
            .find(|subagent| subagent.id == parent_id)
    }) {
        return tree::subagent_label(parent);
    }
    // The same number the window's own box and the sidebar show, so the two
    // can be matched up by eye.
    match tree::agent_order(&app.rows())
        .iter()
        .position(|id| *id == window.id)
    {
        Some(index) => format!("{} {}", index + 1, window.name),
        None => window.name.clone(),
    }
}

/// How long a sub-agent that has stopped ran for. Both fields are ages, so the
/// run is the difference between them, and a missing end reads as zero.
fn finished_secs(info: &SubagentInfo) -> u64 {
    info.ended_secs
        .map_or(0, |ended| info.started_secs.saturating_sub(ended))
}

fn with_tool(label: &str, tool: Option<&str>) -> String {
    match tool {
        Some(tool) => format!("{label} · {tool}"),
        None => label.to_owned(),
    }
}

fn display_path(path: &Path) -> String {
    let text = shorten_home(path);
    if text == "~/" { "~".to_owned() } else { text }
}

/// Where a worktree's `HEAD` points, in the idiom the status bar already uses.
fn head_text(head: &Head) -> String {
    match head {
        Head::Branch(name) | Head::Unborn(name) => name.clone(),
        Head::Detached(oid) => format!("@{oid}"),
    }
}

/// What is uncommitted in a worktree, for the project's `changes` field.
fn changes_text(state: &GitState) -> String {
    let mut parts = Vec::new();
    if state.conflicts > 0 {
        parts.push(format!("⚠{}", state.conflicts));
    }
    if state.dirty > 0 {
        parts.push(format!("●{}", state.dirty));
    }
    if state.untracked > 0 {
        parts.push(format!("?{}", state.untracked));
    }
    if parts.is_empty() {
        "clean".to_owned()
    } else {
        parts.join(" ")
    }
}

/// A window's branch with its dirty and ahead/behind counts, built by the one
/// function that already decides what a worktree's git state reads as.
fn git_text(state: &GitState) -> String {
    git_spans(state, usize::MAX)
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[cfg(test)]
mod tests;
