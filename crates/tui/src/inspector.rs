//! The node inspector: everything anthrex knows about the selected node, in a
//! bordered panel below the graph overview's canvas (spec §3).
//!
//! Two halves, tested separately (decision 13). This file is the projection:
//! `inspect` turns a `Row` and the `App` behind it into an `Inspection` —
//! labels and values, and nothing here knows that a terminal exists. `panel`
//! lays an `Inspection` out. The projection is asserted by exact field lists,
//! the renderer by exact rendered strings, the way the graph's painter is.

use crate::app::App;
use crate::theme;
use crate::tree::{self, NodeKey, Row, RowKind, RuntimeCounts};
use crate::ui::statusbar::{change_parts, git_spans_in, head_text};
use crate::ui::terminal::shorten_home;
use crate::ui::tree_view::counts_text;
use proto::{GitState, Status, SubagentInfo, SubagentState, WindowInfo};
use ratatui::text::Span;
use std::collections::BTreeSet;
use std::path::Path;

/// The panel's own height: a border, the title row, five field rows, a border
/// (decision 2).
pub const INSPECTOR_HEIGHT: u16 = 8;

/// Milestone 9.0.7 decision 17: the canvas rows the panel always leaves above it
/// (milestone 4.7 decision 6's six), the run view's content-sized one included.
pub const RUN_CANVAS_MIN: u16 = 6;
/// Below this many rows of overview interior the panel gives way to milestone
/// 4.6's single line, so a short terminal loses the inspector and never the
/// canvas (decision 6). Derived, so the run view's clamp always has room.
pub const MIN_INTERIOR_FOR_PANEL: u16 = INSPECTOR_HEIGHT + RUN_CANVAS_MIN;
/// Decision 29: a run inspection's label column.
pub const RUN_LABEL_WIDTH: usize = 10;
/// Decision 30: the run's progress bar, and a planner's and a task budget's.
pub const RUN_PROGRESS_WIDTH: usize = 18;
pub const PROGRESS_WIDTH: usize = 10;

/// How the panel lays fields out: milestone 4.7's columns, or one per row (decision 29).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldLayout {
    #[default]
    Columns,
    Rows,
    /// Milestone 9.0.7 decision 12: a task's OUTCOME, EVIDENCE, INTENT and DETAIL, each
    /// a bold title row and its fields, every value wrapping under itself, scrolled by
    /// `Inspection.scroll` between the title and the pinned `Inspection.footer`.
    Sections,
}

/// One titled part of a `Sections` inspection (decision 22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub fields: Vec<SectionField>,
}

/// One labelled value of a section. `value` may hold several lines and any text an
/// agent wrote: the panel sanitises and wraps it. `note` follows the label, muted
/// (the summary's source); `collapse` cuts the value to its first `BRIEF_LINES`
/// wrapped lines and `… (b: more)`; `marks` says which of its glyphs the panel colours.
/// An empty label puts the value at the left edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionField {
    pub label: &'static str,
    pub note: Option<&'static str>,
    pub value: String,
    pub collapse: bool,
    pub marks: Marks,
}

/// Milestone 9.0.7 decision 13: the marks a section value's glyphs carry (`✓` in
/// `Done`, `✗` in `Failed`, `◌` in `Muted`), only where the client wrote them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Marks {
    #[default]
    None,
    /// The first mark among the first two words of each line (`✓ passed`, `◌ <criterion>`,
    /// `r2 ✓ approve`): what follows is agent text and never coloured.
    Lead,
    /// `Lead` on the value's first line only (the review row; its summary line follows).
    First,
    /// The pipeline: every step's mark, and the current step's word in `Working` bold.
    Pipeline,
}

/// Milestone 9.0.7 decision 12: a collapsed brief's wrapped lines (INTENT's one line).
pub const BRIEF_LINES: usize = 1;

/// One labelled value. `wrap` marks the one field a column may not elide: the
/// sub-agent's task, which the panel exists to show whole (decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub wrap: bool,
}

/// One node, projected: its status glyph in its status colour, the name it is
/// known by, and its fields in the order decisions 8 to 10 give them. A run-view node
/// also has a right-aligned `right` text and lays its fields out in rows (M8c).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inspection {
    pub glyph: Span<'static>,
    pub name: String,
    pub right: Option<String>,
    pub fields: Vec<Field>,
    pub layout: FieldLayout,
    /// `Sections` only: the sections, and the first body row shown (decision 25).
    pub sections: Vec<Section>,
    pub scroll: u16,
    /// `Sections` only: decision 12's footer, pinned to the last interior row.
    pub footer: Option<String>,
    /// `Sections` only: the node lists an action, so the border says ` . actions `.
    pub actions: bool,
}

fn field(label: &'static str, value: impl Into<String>) -> Field {
    Field {
        label,
        value: value.into(),
        wrap: false,
    }
}

/// Everything anthrex knows about one node. Pure: it reads `app`, and computes
/// strings (decision 13). In ASCII mode every string is folded here (milestone 9.0.7
/// decision 5), so the panel lays out, and `task_panel_rows` counts, what is drawn.
pub fn inspect(row: &Row<'_>, app: &App) -> Inspection {
    let inspection = inspect_raw(row, app);
    if app.palette().ascii {
        fold_inspection(inspection)
    } else {
        inspection
    }
}

/// `inspection` with every string `theme::fold`ed to ASCII, its glyphs their twins.
fn fold_inspection(mut inspection: Inspection) -> Inspection {
    let fold = |text: &str| theme::ascii_twins(&theme::fold(text, true));
    inspection.name = fold(&inspection.name);
    inspection.right = inspection.right.as_deref().map(fold);
    inspection.footer = inspection.footer.as_deref().map(fold);
    for field in &mut inspection.fields {
        field.value = fold(&field.value);
    }
    for field in inspection.sections.iter_mut().flat_map(|s| &mut s.fields) {
        field.value = fold(&field.value);
    }
    inspection
}

fn inspect_raw(row: &Row<'_>, app: &App) -> Inspection {
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
            ..Default::default()
        },
        RowKind::Window { info, position, .. } => Inspection {
            glyph: status_span(info.status, app),
            name: format!("{position} {}", info.name),
            fields: window_fields(info, app),
            ..Default::default()
        },
        RowKind::Subagent { info } => Inspection {
            glyph: look(
                theme::subagent_look(info, app.spinner_frame, app.palette().ascii),
                app,
            ),
            name: subagent_name(info),
            fields: subagent_fields(row, info, app),
            ..Default::default()
        },
        RowKind::Run {
            run, orchestrator, ..
        } => run::run_inspection(run, *orchestrator, app),
        RowKind::Planner { run, planner } => run::planner_inspection(run, planner, app),
        RowKind::Scout { run, scout, window } => run::scout_inspection(run, scout, *window, app),
        RowKind::Task { run, task } => run_task::task_inspection(run, task, app),
        RowKind::Stage { run, stage } => run_stage::stage_inspection(run, stage, app),
        RowKind::AgentRound { run, task, round } => {
            run_round::round_inspection(run, task, round, app)
        }
    }
}

fn status_span(status: Status, app: &App) -> Span<'static> {
    look(
        theme::status_look(status, app.spinner_frame, app.palette().ascii),
        app,
    )
}

/// A look's glyph styled in its role (milestone 9.0.7 decision 3).
pub(crate) fn look((glyph, role): (&'static str, theme::Role), app: &App) -> Span<'static> {
    Span::styled(glyph, theme::role(role, app.palette()))
}

/// A project's path, status and runtime counts, and its git state only when
/// every window in it stands in one worktree (decision 8).
fn project_fields(root: &Path, status: Status, counts: RuntimeCounts, app: &App) -> Vec<Field> {
    let mut fields = vec![
        field("path", display_path(root)),
        field("status", status.label()),
        field("agents", counts_text(counts)),
    ];
    let members = || app.windows.iter().filter(|window| window.project == root);
    // Ordered and deduplicated: two windows in the same worktree are one
    // worktree, and the count below has to say so.
    let worktrees: BTreeSet<&Path> = members()
        .filter_map(|window| window.worktree.as_deref())
        .collect();
    // A project is only *on* a worktree when every window in it is. One agent
    // in a worktree beside a plain shell is a mixed project, and showing that
    // worktree's branch would attribute it to the whole thing — the same lie
    // decision 8 refuses to tell when there are several.
    let all_on_a_worktree = members().all(|window| window.worktree.is_some());
    match worktrees.len() {
        1 if all_on_a_worktree => {
            if let Some(state) = worktrees.iter().next().and_then(|root| app.git.get(*root)) {
                fields.push(field("branch", head_text(&state.head)));
                fields.push(field("changes", changes_text(state, app.palette())));
            }
        }
        // A count of the worktrees a project spans is true whether or not every
        // window is on one, so it survives the mixed case that omits the rest.
        count if count > 1 => fields.push(field("branch", format!("{count} worktrees"))),
        _ => {}
    }
    fields
}

/// A window's status and timings, model, git state, and what it has running
/// under it, then where it stands (decision 9).
///
/// The order is what a narrow panel keeps: fields are dropped from the end, so
/// what a window is doing comes before where it is doing it, and the session id
/// — the longest value and the least read — comes last.
fn window_fields(info: &WindowInfo, app: &App) -> Vec<Field> {
    let mut fields = vec![
        field(
            "status",
            with_tool(info.status.label(), info.tool.as_deref()),
        ),
        field("for", tree::format_elapsed(app.elapsed_secs(info))),
        field("model", info.model.as_deref().unwrap_or("-")),
    ];
    // Keyed by the worktree root, and absent state omits the field rather than
    // showing it empty (decision 12).
    if let Some(state) = info.worktree.as_deref().and_then(|root| app.git.get(root)) {
        fields.push(field("branch", git_text(state, app.palette())));
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
    fields.push(field("runtime", info.runtime.label()));
    fields.push(field("dir", display_path(&info.cwd)));
    if let Some(worktree) = info.worktree.as_deref()
        && worktree != info.cwd
    {
        fields.push(field("worktree", display_path(worktree)));
    }
    if let Some(session) = info.session_id.as_deref() {
        fields.push(field("session", session));
    }
    fields
}

/// A sub-agent's parentage, state, timing, model, kind and depth (decision 10),
/// with its task as the one field marked for wrapping.
///
/// `spawned by` leads because it is the one thing the tree cannot tell you
/// (spec §3): a sub-agent three levels down looks exactly like one directly
/// under its window. A narrow panel drops from the end, and dropping the field
/// this milestone exists for would be the wrong way round.
fn subagent_fields(row: &Row<'_>, info: &SubagentInfo, app: &App) -> Vec<Field> {
    let (state, duration) = match info.state {
        SubagentState::Running => ("running", app.age_secs(info.started_secs)),
        SubagentState::Done => ("done", finished_secs(info)),
        SubagentState::Failed => ("failed", finished_secs(info)),
    };
    let mut fields = Vec::new();
    if let Some(label) = info.label.as_deref() {
        // Where it lands is the panel's business: it leaves the column flow for
        // the rows below, and the panel drops it when the title already showed
        // the label whole.
        fields.push(Field {
            label: "task",
            value: label.to_owned(),
            wrap: true,
        });
    }
    fields.push(field("spawned by", spawned_by(row, info, app)));
    fields.push(field("state", with_tool(state, info.tool.as_deref())));
    fields.push(field("for", tree::format_elapsed(duration)));
    fields.push(field("model", info.model.as_deref().unwrap_or("-")));
    fields.push(field("kind", info.kind.clone()));
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
        return subagent_name(parent);
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

/// What a sub-agent is called here: its label, which is the task it was given,
/// and its kind when it has none.
///
/// Not `tree::subagent_label`'s `kind: label`. The panel has a `kind` field of
/// its own, so repeating it in the title would say the same word twice, and a
/// parent's name in `spawned by` is shorter and easier to match by eye without
/// it.
fn subagent_name(info: &SubagentInfo) -> String {
    info.label.clone().unwrap_or_else(|| info.kind.clone())
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

/// What is uncommitted in a worktree, for the project's `changes` field. The
/// glyphs are the status bar's, joined rather than ranked — the panel has room
/// for all three (spec §3 names dirty and untracked; a conflict outweighs
/// either and is shown with them).
fn changes_text(state: &GitState, p: theme::Palette) -> String {
    let parts = change_parts(state, p);
    if parts.is_empty() {
        return "clean".to_owned();
    }
    parts
        .iter()
        .map(|part| part.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A window's branch with its dirty and ahead/behind counts, built by the one
/// function that already decides what a worktree's git state reads as.
fn git_text(state: &GitState, p: theme::Palette) -> String {
    git_spans_in(state, usize::MAX, p)
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

mod panel;
mod run;
pub(crate) mod run_format;
mod run_orch;
mod run_round;
mod run_stage;
mod run_stage_pr;
mod run_task;
mod run_task_outcome;
mod run_task_sections;

pub use panel::panel_rows;
pub(crate) use run_stage::tier_duration;
pub(crate) use run_task::state_word;
pub use run_task::{task_panel_room, task_panel_rows};

#[cfg(test)]
pub use panel::render;
pub use panel::render_in;
#[cfg(test)]
pub use run_format::progress_bar;
pub use run_format::{format_duration, format_tokens, local_hhmm};
pub(crate) use run_stage_pr::delivering;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod run_tests;

#[cfg(test)]
mod run_nodes_tests;

#[cfg(test)]
mod run_task_tests;

#[cfg(test)]
mod run_round_tests;

#[cfg(test)]
mod run_orch_tests;

#[cfg(test)]
mod run_stage_tests;

#[cfg(test)]
mod run_stage_pr_tests;

#[cfg(test)]
mod run_task_sections_tests;

#[cfg(test)]
mod run_task_outcome_tests;
