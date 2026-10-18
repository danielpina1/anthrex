//! Milestone 9.15 decisions 45, 46 and 50 (task M9.15.15): the lessons graph page,
//! drawn over the body in the screens' frame (`Lessons · <project>`). Inside it: the
//! canvas (Braille edges, nodes printed on top), the inspect panel when open (right,
//! [`PANEL_WIDTH`] columns, when the area is at least [`PANEL_SIDE_MIN`] wide, else the
//! lower half), the footer of decision 43 (or the page's error or last outcome), and
//! the hint row (the keys, the query being typed, or a question in the destructive
//! style). Every stored string passes `safe_text::one_line` and is folded in ASCII.
//! Pure: rendering takes `&App`.

use super::kit::{self, Hint, cut};
use crate::app::App;
use crate::app::screens::lessons_graph::LessonsGraphScreen;
use crate::lessons_graph::layout::Point;
use crate::lessons_graph::nav::View;
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, dot, ellipsis, fold, role};
use proto::{GraphNode, GraphNodeKind, LessonStatus};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as Segment};
use ratatui::widgets::{Clear, Paragraph};

/// The side panel's width (Interfaces).
pub const PANEL_WIDTH: u16 = 44;
/// The narrowest area that takes the panel on the side (Interfaces).
pub const PANEL_SIDE_MIN: u16 = 100;
/// The longest label drawn (Interfaces).
pub const LABEL_MAX: usize = 24;
/// The zoom from which every visible lesson is labelled (decision 45).
pub const LABEL_ZOOM: f64 = 2.0;

/// The page's areas inside its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Areas {
    pub canvas: Rect,
    pub panel: Option<Rect>,
    pub footer: Option<Rect>,
    pub hints: Rect,
}

/// Decision 43's footer: the omitted count, or that only modules are drawn.
pub fn footer_text(s: &LessonsGraphScreen) -> Option<String> {
    let graph = s.graph.as_ref()?;
    if graph.omitted > 0 {
        return Some(format!(
            "{} more lessons not drawn; anthrex lessons list shows them all",
            graph.omitted
        ));
    }
    let lessons = graph.nodes.iter().any(|n| n.kind == GraphNodeKind::Lesson);
    (!lessons).then(|| "no lessons yet; modules only".to_string())
}

/// What the footer row shows: the page's error, else decision 43's footer, else the
/// last outcome.
fn footer_line(s: &LessonsGraphScreen) -> Option<(String, Role)> {
    if let Some(error) = &s.error {
        return Some((error.clone(), Role::Failed));
    }
    if let Some(text) = footer_text(s) {
        return Some((text, Role::Muted));
    }
    s.message.clone().map(|m| (m, Role::Muted))
}

/// The page's areas in `area` (the body): the frame's interior less the hint row, the
/// footer row when there is one, and the panel when it is open.
pub fn areas(s: &LessonsGraphScreen, area: Rect) -> Areas {
    let inner = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let row = |y: u16| Rect {
        y,
        height: 1,
        ..inner
    };
    let hints = row(inner.y + inner.height.saturating_sub(1));
    let mut rest = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let footer = footer_line(s).is_some().then(|| {
        rest.height = rest.height.saturating_sub(1);
        row(rest.y + rest.height)
    });
    let (canvas, panel) = match s.panel {
        None => (rest, None),
        Some(_) if area.width >= PANEL_SIDE_MIN => {
            let w = PANEL_WIDTH.min(rest.width);
            let canvas = Rect {
                width: rest.width - w,
                ..rest
            };
            let panel = Rect {
                x: rest.x + canvas.width,
                width: w,
                ..rest
            };
            (canvas, Some(panel))
        }
        Some(_) => {
            let top = rest.height / 2;
            let canvas = Rect {
                height: top,
                ..rest
            };
            let panel = Rect {
                y: rest.y + top,
                height: rest.height - top,
                ..rest
            };
            (canvas, Some(panel))
        }
    };
    Areas {
        canvas,
        panel,
        footer,
        hints,
    }
}

/// The canvas's projection over `canvas`.
pub fn view(s: &LessonsGraphScreen, canvas: Rect) -> View {
    View {
        x: canvas.x,
        y: canvas.y,
        width: canvas.width,
        height: canvas.height,
        camera: s.camera,
    }
}

/// Decisions 45 and 50: a node's glyph and its role (`None`: the normal text colour).
pub fn look(node: &GraphNode, ascii: bool) -> (&'static str, Option<Role>) {
    let pick = |u: &'static str, a: &'static str| if ascii { a } else { u };
    match (node.kind, node.status) {
        (GraphNodeKind::Module | GraphNodeKind::Root, _) => (pick("◉", "O"), None),
        (_, Some(LessonStatus::Proposed)) => (pick("○", "o"), Some(Role::Attention)),
        (_, Some(LessonStatus::Known)) => (pick("•", "x"), Some(Role::Failed)),
        (_, Some(LessonStatus::OnTrial)) => (pick("•", "*"), Some(Role::Muted)),
        (_, Some(LessonStatus::Approved | LessonStatus::Kept)) => (pick("•", "*"), None),
        _ => (pick("•", "*"), Some(Role::Muted)),
    }
}

/// A label, on one line, folded in ASCII, cut to [`LABEL_MAX`].
pub fn label(text: &str, p: Palette) -> String {
    cut(&fold(&one_line(text), p.ascii), LABEL_MAX, ellipsis(p))
}

fn colour(r: Role, p: Palette) -> ratatui::style::Color {
    role(r, p).fg.unwrap_or_default()
}

/// The canvas: edges (the selected node's in the accent, drawn last), then the nodes,
/// the selected one last so it stays on top.
fn draw_canvas(frame: &mut Frame, s: &LessonsGraphScreen, canvas: Rect, p: Palette) {
    let Some(graph) = &s.graph else {
        let text = format!("loading{}", ellipsis(p));
        frame.render_widget(
            Paragraph::new(Line::styled(text, role(Role::Muted, p))),
            canvas,
        );
        return;
    };
    let v = view(s, canvas);
    let Some((xb, yb)) = v.bounds() else {
        return;
    };
    let selected = s.selected.as_deref();
    let lit = s.emphasis();
    let at = |id: &str| s.index_of(id).and_then(|i| s.positions.get(i).copied());
    let mut edges: Vec<(bool, Point, Point)> = graph
        .edges
        .iter()
        .filter_map(|e| {
            let hot = selected.is_some_and(|id| e.from == id || e.to == id);
            Some((hot, at(&e.from)?, at(&e.to)?))
        })
        .collect();
    edges.sort_by_key(|(hot, _, _)| *hot);
    let mut order: Vec<usize> = (0..graph.nodes.len()).collect();
    order.sort_by_key(|i| Some(graph.nodes[*i].id.as_str()) == selected);
    let zoomed = s.camera.zoom() >= LABEL_ZOOM;
    // The normal text colour, set: a label printed over a Braille edge would otherwise
    // keep the edge's colour.
    let normal = Style::default().fg(ratatui::style::Color::Reset);
    let nodes: Vec<(f64, f64, Line<'static>)> = order
        .into_iter()
        .filter_map(|i| {
            let node = &graph.nodes[i];
            let (x, y) = *s.positions.get(i)?;
            let is_selected = Some(node.id.as_str()) == selected;
            let dim = !lit.get(i).copied().unwrap_or(true);
            let (g, r) = look(node, p.ascii);
            let glyph_style = match (dim, r) {
                (true, _) => role(Role::Muted, p),
                (false, Some(r)) => role(r, p),
                (false, None) => normal,
            };
            let mut spans = vec![Span::styled(g.to_string(), glyph_style)];
            let hub = node.kind != GraphNodeKind::Lesson;
            if hub || is_selected || zoomed {
                let style = if is_selected {
                    role(Role::Accent, p).add_modifier(Modifier::BOLD)
                } else if dim {
                    role(Role::Muted, p)
                } else if hub {
                    normal.add_modifier(Modifier::BOLD)
                } else {
                    normal
                };
                spans.push(Span::styled(format!(" {}", label(&node.label, p)), style));
            }
            Some((x, -y, Line::from(spans)))
        })
        .collect();
    let (muted, accent) = (colour(Role::Muted, p), colour(Role::Accent, p));
    let widget = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds(xb)
        .y_bounds(yb)
        .paint(move |ctx| {
            for (hot, a, b) in &edges {
                ctx.draw(&Segment {
                    x1: a.0,
                    y1: -a.1,
                    x2: b.0,
                    y2: -b.1,
                    color: if *hot { accent } else { muted },
                });
            }
            for (x, y, line) in &nodes {
                ctx.print(*x, *y, line.clone());
            }
        });
    frame.render_widget(widget, canvas);
}

/// The hint row: a question, the query being typed, or the keys.
pub fn hint_line(s: &LessonsGraphScreen, width: u16, p: Palette) -> Line<'static> {
    if let Some(ask) = &s.question {
        let text = fold(&one_line(&ask.text()), p.ascii);
        let head = text.strip_suffix("y/n").unwrap_or(&text).to_string();
        let line = Line::from(vec![
            Span::raw(head),
            Span::styled("y/n", role(Role::Accent, p)),
        ]);
        let mut line = kit::destructive(line, "y/n", ask.verb(), p);
        if let Some(first) = line.spans.first_mut() {
            first.style = role(Role::Failed, p);
        }
        return line;
    }
    if s.search.typing {
        let query = fold(&one_line(&s.search.query), p.ascii);
        return Line::from(vec![
            Span::styled("/", role(Role::Accent, p)),
            Span::raw(cut(
                &query,
                usize::from(width).saturating_sub(2),
                ellipsis(p),
            )),
            kit::cursor_block(Style::default(), p),
        ]);
    }
    let mut list: Vec<(&str, String, u8)> = vec![];
    if s.search.active() {
        let q = cut(&fold(&one_line(&s.search.query), p.ascii), 16, ellipsis(p));
        list.push(("n/N", format!("next /{q}"), 7));
    }
    let panel = if s.panel.is_some() {
        "panel"
    } else {
        "inspect"
    };
    list.extend([
        ("arrows", "select".to_string(), 8),
        ("enter", panel.to_string(), 6),
        ("/", "search".to_string(), 5),
        ("d", "remove".to_string(), 4),
        ("a/r", "approve/reject".to_string(), 3),
        ("+/-", "zoom".to_string(), 2),
        ("hjkl", "pan".to_string(), 1),
        ("0", "centre".to_string(), 0),
        ("esc", "back".to_string(), 9),
    ]);
    if s.panel.is_some() {
        list.insert(2, ("tab", "focus".to_string(), 5));
    }
    let hints: Vec<Hint> = list
        .into_iter()
        .map(|(k, w, priority)| Hint {
            key: k.to_string(),
            word: w,
            priority,
        })
        .collect();
    kit::hints_joined(width, &hints, crate::theme::dot_sep(p), p)
}

/// The project's directory name, as the title names it.
fn project(s: &LessonsGraphScreen) -> String {
    s.dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| s.dir.display().to_string())
}

/// Draws the page over `area` (the body).
pub fn render(frame: &mut Frame, app: &App, s: &LessonsGraphScreen, area: Rect) {
    let p = app.palette();
    let keys = app.modal.is_none();
    let title = format!("Lessons {} {}", dot(p), project(s));
    frame.render_widget(Clear, area);
    frame.render_widget(kit::screen_frame(&title, keys, p), area);
    if area.width < 3 || area.height < 3 {
        return;
    }
    let a = areas(s, area);
    draw_canvas(frame, s, a.canvas, p);
    if let (Some(rect), Some(_)) = (a.panel, s.panel) {
        super::lessons_graph_panel::render(frame, s, rect, (keys, app.run_now(), app.zone()), p);
    }
    if let (Some(rect), Some((text, r))) = (a.footer, footer_line(s)) {
        let text = cut(
            &fold(&one_line(&text), p.ascii),
            usize::from(rect.width),
            ellipsis(p),
        );
        frame.render_widget(Paragraph::new(Line::styled(text, role(r, p))), rect);
    }
    frame.render_widget(Paragraph::new(hint_line(s, a.hints.width, p)), a.hints);
}

#[cfg(test)]
#[path = "lessons_graph_tests.rs"]
mod tests;
