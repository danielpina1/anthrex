//! Milestone 9.15 decisions 45 to 49 (task M9.15.15): the lessons knowledge graph page.
//! `g` on the Profile screen's Lessons page opens `Screen::LessonsGraph`, which keeps
//! that Profile screen (`back`) and restores it on `Esc` (no query, no panel). It asks
//! the daemon for the graph (`LessonsRequest::Graph`) and lays it out once per reply
//! (decision 44; task 14's review, M5: never on a key or a render, counted in
//! `layouts`). The camera, arrows and click are `lessons_graph::nav`'s, the search
//! `lessons_graph::search`'s; `Enter` opens the inspect panel; `d`, `a` and `r` remove,
//! approve and reject, a question's answer acting on the id it asked about (task 12's
//! review, I1). After any reply the page asks for the graph again, keeping the camera
//! and the selection while the node exists. Mounted from `app/screens.rs` (decision
//! 39). Pure: every request leaves as an `Effect` (decision 49, AGENTS.md rule 5).

use crate::app::profile_screen::{ProfilePage, ProfileScreen};
use crate::app::replies::{PendingWhat, reply_timeout};
use crate::app::screens::Screen;
use crate::app::{App, Effect, ToastLevel};
use crate::lessons_graph::layout::{self, Point};
use crate::lessons_graph::nav::{self, Camera, Dir, View};
use crate::lessons_graph::search::{self, Search};
use crossterm::event::{KeyCode, KeyEvent};
use proto::{
    GraphEdgeKind, GraphNode, GraphNodeKind, LessonGraph, LessonInfo, LessonStatus, LessonsRequest,
    RunRequest,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Interfaces "Exact user-visible text": `d` on a module or the root hub.
pub const NOT_REMOVABLE: &str = "a module cannot be removed";
/// The refusal of `C-b a`, `C-b m` and `C-b t` over the page.
pub const LEAVE_GRAPH_FIRST: &str = "leave the lessons graph first (esc)";

/// A question on the hint row; its answer acts on the id it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphAsk {
    Forget { id: String, title: String },
    Dismiss { id: String },
    Reject { id: String },
}

impl GraphAsk {
    /// Its verb, for the destructive style.
    pub fn verb(&self) -> &'static str {
        match self {
            GraphAsk::Forget { .. } => "forget",
            GraphAsk::Dismiss { .. } => "dismiss",
            GraphAsk::Reject { .. } => "reject",
        }
    }

    /// Interfaces "Exact user-visible text" (unsanitised; drawn through `safe_text`).
    pub fn text(&self) -> String {
        match self {
            GraphAsk::Forget { id, title } => format!("forget {id} \"{title}\"? y/n"),
            GraphAsk::Dismiss { id } => format!("dismiss known failure {id}? y/n"),
            GraphAsk::Reject { id } => format!("reject {id}? y/n"),
        }
    }

    /// The id it asks about.
    pub fn id(&self) -> &str {
        match self {
            GraphAsk::Forget { id, .. } | GraphAsk::Dismiss { id } | GraphAsk::Reject { id } => id,
        }
    }

    fn request(&self, dir: PathBuf) -> LessonsRequest {
        match self {
            GraphAsk::Forget { id, .. } | GraphAsk::Dismiss { id } => LessonsRequest::Forget {
                dir,
                ids: vec![id.clone()],
            },
            GraphAsk::Reject { id } => LessonsRequest::Decide {
                dir,
                ids: vec![id.clone()],
                approve: false,
            },
        }
    }
}

/// The inspect panel (decision 48): `focus` while its keys are the panel's (`Tab`),
/// and its row (a module's list) or first line (a lesson's text).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Panel {
    pub focus: bool,
    pub row: usize,
}

/// The page's state.
#[derive(Debug, Clone, PartialEq)]
pub struct LessonsGraphScreen {
    /// The Profile screen it came from, restored on `Esc` (decision 49).
    pub back: Box<ProfileScreen>,
    pub dir: PathBuf,
    /// `None` until the first graph comes.
    pub graph: Option<LessonGraph>,
    /// One position per node of `graph`, in its order.
    pub positions: Vec<Point>,
    /// How many layouts ran: one per graph reply, never per key or frame.
    pub layouts: u64,
    pub camera: Camera,
    /// The selected node's id.
    pub selected: Option<String>,
    pub search: Search,
    pub panel: Option<Panel>,
    pub question: Option<GraphAsk>,
    /// The daemon's last refusal, in its words.
    pub error: Option<String>,
    /// The daemon's last `Done` text.
    pub message: Option<String>,
    /// The latest `Graph` request: only its reply fills the page.
    pub(crate) graph_id: Option<u64>,
}

impl LessonsGraphScreen {
    pub fn new(back: Box<ProfileScreen>) -> Self {
        LessonsGraphScreen {
            dir: back.dir.clone(),
            back,
            graph: None,
            positions: vec![],
            layouts: 0,
            camera: Camera::default(),
            selected: None,
            search: Search::default(),
            panel: None,
            question: None,
            error: None,
            message: None,
            graph_id: None,
        }
    }

    fn nodes(&self) -> &[GraphNode] {
        self.graph.as_ref().map_or(&[], |g| &g.nodes)
    }

    /// The index of the node `id`.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.nodes().iter().position(|n| n.id == id)
    }

    /// The selected node.
    pub fn selected_node(&self) -> Option<&GraphNode> {
        self.nodes().get(self.index_of(self.selected.as_deref()?)?)
    }

    /// A drawn lesson's details.
    pub fn lesson(&self, id: &str) -> Option<&LessonInfo> {
        self.graph.as_ref()?.lessons.iter().find(|l| l.id == id)
    }

    /// The ids joined to `id` by an edge.
    pub fn neighbours(&self, id: &str) -> BTreeSet<&str> {
        let Some(graph) = &self.graph else {
            return BTreeSet::new();
        };
        graph
            .edges
            .iter()
            .filter_map(|e| {
                if e.from == id {
                    Some(e.to.as_str())
                } else if e.to == id {
                    Some(e.from.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    /// The search's matches, one flag per node.
    pub fn matches(&self) -> Vec<bool> {
        match &self.graph {
            Some(graph) => search::matches(graph, &self.search.query),
            None => vec![],
        }
    }

    /// Decisions 46 and 47: one flag per node, whether it is lit. A filter in force
    /// lights its matches (and the selection); else a selection lights itself and its
    /// neighbours; else everything is lit.
    pub fn emphasis(&self) -> Vec<bool> {
        let selected = self.selected.as_deref();
        if self.search.active() {
            let lit = self.matches();
            return (self.nodes().iter().zip(lit))
                .map(|(n, m)| m || Some(n.id.as_str()) == selected)
                .collect();
        }
        match selected.filter(|id| self.index_of(id).is_some()) {
            Some(id) => {
                let near = self.neighbours(id);
                self.nodes()
                    .iter()
                    .map(|n| n.id == id || near.contains(n.id.as_str()))
                    .collect()
            }
            None => vec![true; self.nodes().len()],
        }
    }

    /// Decision 48: a module's (or the root hub's) lessons and known failures, the
    /// lessons with a `touches` edge to it, in id order.
    pub fn module_rows(&self, id: &str) -> Vec<String> {
        let Some(graph) = &self.graph else {
            return vec![];
        };
        let rows: BTreeSet<&str> = graph
            .edges
            .iter()
            .filter(|e| e.kind == GraphEdgeKind::Touches && e.to == id)
            .map(|e| e.from.as_str())
            .collect();
        rows.into_iter().map(str::to_string).collect()
    }

    /// The points `nav` reads: each node's id and position.
    fn points(&self) -> Vec<(&str, Point)> {
        (self.nodes().iter().zip(&self.positions))
            .map(|(n, p)| (n.id.as_str(), *p))
            .collect()
    }

    /// An arrow: the nearest node in its cone, or (nothing selected) the node nearest
    /// the camera's centre.
    fn arrow(&mut self, dir: Dir) {
        let points = self.points();
        let next = match self.selected.as_deref().and_then(|id| self.index_of(id)) {
            Some(from) => nav::nearest(&points, from, dir),
            None => {
                let (cx, cy) = (self.camera.cx, self.camera.cy);
                let d = |p: Point| (p.0 - cx).powi(2) + (p.1 - cy).powi(2);
                (0..points.len()).min_by(|a, b| {
                    d(points[*a].1)
                        .total_cmp(&d(points[*b].1))
                        .then(points[*a].0.cmp(points[*b].0))
                })
            }
        };
        if let Some(i) = next {
            self.selected = Some(points[i].0.to_string());
        }
    }

    /// A new graph: laid out once, the camera kept, the selection kept while its node
    /// exists (the panel closes with it). Task 15's review, M2: a question about a node no
    /// longer drawn is dropped.
    pub(crate) fn install(&mut self, graph: LessonGraph) {
        self.positions = layout::layout(&graph).positions;
        self.layouts += 1;
        self.graph = Some(graph);
        if self
            .selected
            .as_deref()
            .is_some_and(|id| self.index_of(id).is_none())
        {
            self.selected = None;
            self.panel = None;
        }
        if (self.question.as_ref()).is_some_and(|ask| self.index_of(ask.id()).is_none()) {
            self.question = None;
        }
    }

    /// A left click at `(column, row)` on the canvas `view` shows.
    pub fn click(&mut self, view: View, column: u16, row: u16) {
        let cells: Vec<(&str, Option<(u16, u16)>)> = (self.nodes().iter().zip(&self.positions))
            .map(|(n, p)| (n.id.as_str(), view.cell(*p)))
            .collect();
        let picked = nav::pick(&cells, (column, row)).map(|i| cells[i].0.to_string());
        if picked.is_none() {
            self.panel = None;
        }
        self.selected = picked;
    }

    /// `Enter` in the panel on a module's row: that node is selected.
    fn panel_enter(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if node.kind == GraphNodeKind::Lesson {
            return;
        }
        let rows = self.module_rows(&node.id.clone());
        let row = self.panel.map_or(0, |p| p.row);
        if let Some(id) = rows.get(row) {
            self.selected = Some(id.clone());
            self.panel = Some(Panel {
                focus: true,
                row: 0,
            });
        }
    }

    /// `j`/`k` in the panel.
    fn panel_move(&mut self, down: bool) {
        let last = match self.selected_node() {
            Some(n) if n.kind != GraphNodeKind::Lesson => {
                self.module_rows(&n.id.clone()).len().saturating_sub(1)
            }
            // A lesson's text, as many lines as the side panel's interior takes (the
            // lower half is wider, so it never needs more).
            Some(n) => self.lesson(&n.id).map_or(0, |l| {
                let width = crate::ui::lessons_graph::PANEL_WIDTH - 2;
                let lines = crate::ui::lessons_graph_panel::lesson_lines(
                    l,
                    width,
                    crate::theme::Palette::PLAIN,
                );
                lines.len().saturating_sub(1)
            }),
            None => 0,
        };
        if let Some(panel) = &mut self.panel {
            panel.row = if down {
                (panel.row + 1).min(last)
            } else {
                panel.row.saturating_sub(1)
            };
        }
    }
}

impl App {
    fn graph_screen_mut(&mut self) -> Option<&mut LessonsGraphScreen> {
        match &mut self.screen {
            Some(Screen::LessonsGraph(s)) => Some(s),
            _ => None,
        }
    }

    fn graph_send(&mut self, dir: PathBuf, graph: bool, request: LessonsRequest) -> Effect {
        let request = RunRequest::Lessons(request);
        let timeout = reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        self.replies
            .insert(id, PendingWhat::LessonsGraph { dir, graph }, timeout);
        effect
    }

    /// The page's `Graph`, recorded as its latest.
    fn graph_request(&mut self) -> Vec<Effect> {
        let Some(dir) = self.graph_screen_mut().map(|s| s.dir.clone()) else {
            return vec![];
        };
        let request = LessonsRequest::Graph { dir: dir.clone() };
        let effect = self.graph_send(dir, true, request);
        if let (Some(s), Effect::Send(proto::ClientMsg::RunTagged { id, .. })) =
            (self.graph_screen_mut(), &effect)
        {
            s.graph_id = Some(*id);
        }
        vec![effect]
    }

    /// `g` on the Lessons page (decision 49): the page over the Profile screen, which
    /// it keeps.
    pub(in crate::app) fn open_lessons_graph(&mut self) -> Vec<Effect> {
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return vec![];
        }
        let Some(Screen::Profile(back)) = self.screen.take() else {
            return vec![];
        };
        let screen = LessonsGraphScreen::new(back);
        self.set_screen(Some(Screen::LessonsGraph(Box::new(screen))));
        self.graph_request()
    }

    /// `Esc` with nothing left to clear: the Profile screen comes back, its Lessons
    /// page listing again (the graph may have changed it).
    fn leave_lessons_graph(&mut self) -> Vec<Effect> {
        let Some(Screen::LessonsGraph(s)) = self.screen.take() else {
            return vec![];
        };
        let page = matches!(s.back.page, Some(ProfilePage::Lessons(_)));
        self.set_screen(Some(Screen::Profile(s.back)));
        if page {
            return self.lessons_list();
        }
        vec![]
    }

    /// Sends `ask`'s request on the page's repository.
    fn graph_act(&mut self, make: impl FnOnce(PathBuf) -> LessonsRequest) -> Vec<Effect> {
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return vec![];
        }
        let Some(dir) = self.graph_screen_mut().map(|s| s.dir.clone()) else {
            return vec![];
        };
        vec![self.graph_send(dir.clone(), false, make(dir))]
    }

    /// The page's keys (decisions 46 to 49). A question takes every key until it is
    /// answered; a query being typed takes every key but its own.
    pub(in crate::app) fn on_lessons_graph_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(s) = self.graph_screen_mut() else {
            return vec![];
        };
        if let Some(ask) = s.question.clone() {
            return match key.code {
                KeyCode::Char('y') => {
                    s.question = None;
                    self.graph_act(|dir| ask.request(dir))
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    s.question = None;
                    vec![]
                }
                _ => vec![],
            };
        }
        if s.search.key(key.code) {
            return vec![];
        }
        let in_panel = s.panel.is_some_and(|p| p.focus);
        match key.code {
            KeyCode::Esc => {
                if !s.search.esc() && s.panel.take().is_none() {
                    return self.leave_lessons_graph();
                }
            }
            KeyCode::Tab => {
                if let Some(panel) = &mut s.panel {
                    panel.focus = !panel.focus;
                }
            }
            KeyCode::Enter if in_panel => s.panel_enter(),
            KeyCode::Char('j') if in_panel => s.panel_move(true),
            KeyCode::Char('k') if in_panel => s.panel_move(false),
            KeyCode::Enter => {
                if s.selected_node().is_some() {
                    s.panel = Some(Panel::default());
                }
            }
            KeyCode::Char('/') => s.search.open(),
            KeyCode::Char(c @ ('n' | 'N')) => {
                if let Some(graph) = &s.graph {
                    let lit = search::matches(graph, &s.search.query);
                    let at = s.selected.as_deref();
                    if let Some(id) = search::cycle(graph, &lit, at, c == 'n') {
                        s.selected = Some(id);
                    }
                }
            }
            KeyCode::Char('+' | '=') => s.camera.zoom_in(),
            KeyCode::Char('-') => s.camera.zoom_out(),
            KeyCode::Char('0') => s.camera.recentre(),
            KeyCode::Char('h') => s.camera.pan(-1, 0),
            KeyCode::Char('l') => s.camera.pan(1, 0),
            KeyCode::Char('k') => s.camera.pan(0, -1),
            KeyCode::Char('j') => s.camera.pan(0, 1),
            KeyCode::Left => s.arrow(Dir::Left),
            KeyCode::Right => s.arrow(Dir::Right),
            KeyCode::Up => s.arrow(Dir::Up),
            KeyCode::Down => s.arrow(Dir::Down),
            KeyCode::Char('d') => return self.graph_remove(),
            KeyCode::Char('a') => return self.graph_decide(true),
            KeyCode::Char('r') => return self.graph_decide(false),
            _ => {}
        }
        vec![]
    }

    /// `d`: asks to forget a lesson or dismiss a known failure; a hub is refused.
    fn graph_remove(&mut self) -> Vec<Effect> {
        let Some(s) = self.graph_screen_mut() else {
            return vec![];
        };
        let Some(node) = s.selected_node().cloned() else {
            return vec![];
        };
        if node.kind != GraphNodeKind::Lesson {
            self.toast_at(ToastLevel::Warn, NOT_REMOVABLE);
            return vec![];
        }
        let title = s.lesson(&node.id).map_or(node.label, |l| l.title.clone());
        s.question = Some(match node.status {
            Some(LessonStatus::Known) => GraphAsk::Dismiss { id: node.id },
            _ => GraphAsk::Forget { id: node.id, title },
        });
        vec![]
    }

    /// `a` sends `Decide` at once; `r` asks first. Only a proposed or on-trial lesson.
    fn graph_decide(&mut self, approve: bool) -> Vec<Effect> {
        let Some(s) = self.graph_screen_mut() else {
            return vec![];
        };
        let Some(node) = s.selected_node() else {
            return vec![];
        };
        if !matches!(
            node.status,
            Some(LessonStatus::Proposed | LessonStatus::OnTrial)
        ) {
            return vec![];
        }
        let id = node.id.clone();
        if !approve {
            s.question = Some(GraphAsk::Reject { id });
            return vec![];
        }
        self.graph_act(|dir| LessonsRequest::Decide {
            dir,
            ids: vec![id],
            approve: true,
        })
    }

    /// `on_click`'s share: a left click on the page's canvas (`canvas`, from
    /// `ui::Layout.lessons_canvas`) selects a node or clears the selection.
    pub(crate) fn lessons_graph_click(
        &mut self,
        column: u16,
        row: u16,
        canvas: ratatui::layout::Rect,
    ) -> Vec<Effect> {
        let Some(s) = self.graph_screen_mut() else {
            return vec![];
        };
        if s.question.is_some() || !canvas.contains((column, row).into()) {
            return vec![];
        }
        let view = View {
            x: canvas.x,
            y: canvas.y,
            width: canvas.width,
            height: canvas.height,
            camera: s.camera,
        };
        s.click(view, column, row);
        vec![]
    }
}

#[path = "lessons_graph_replies.rs"]
mod replies;
