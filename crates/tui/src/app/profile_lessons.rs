//! Milestone 9.15 decisions 35 and 36 (task M9.15.12): the Profile screen's Lessons page.
//! `l` opens it and sends `List { dir, all: false }`; `j`/`k` move, `a` approves at once,
//! `r` and `d` ask `reject <id>? y/n` / `forget <id>? y/n` and send on `y` (`n` or Esc
//! cancels), `t` lists rejected and retired lessons too, `g` opens the graph page
//! (task M9.15.15, `app/lessons_graph_screen.rs`), Esc goes back. After any reply the
//! page lists again; a refusal shows on the page in the daemon's words. The screen itself lists once when it opens,
//! for its hint line's ` · <n> to review` (decision 36); proposals raise no alert
//! (decision 38). Mounted from `app/profile_screen.rs` (decision 39). Pure: every
//! request leaves as an `Effect`.

use super::{ProfileAsk, ProfilePage, ProfileScreen};
use crate::app::{App, Effect, ToastLevel};
use crossterm::event::{KeyCode, KeyEvent};
use proto::{LessonInfo, LessonStatus, LessonsRequest, RunReply, RunRequest};
use std::path::{Path, PathBuf};

/// A verb the page asks `y/n` for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Reject,
    Forget,
}

impl Verb {
    pub fn word(self) -> &'static str {
        match self {
            Verb::Reject => "reject",
            Verb::Forget => "forget",
        }
    }
}

/// The page's question on its bottom line: `<verb> <id>? y/n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub verb: Verb,
    pub id: String,
}

impl Question {
    /// Interfaces "Exact user-visible text" (unsanitised; the page draws it through
    /// `safe_text`).
    pub fn text(&self) -> String {
        format!("{} {}? y/n", self.verb.word(), self.id)
    }
}

/// The Lessons page's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LessonsPage {
    /// `None` until the first list comes.
    pub lessons: Option<Vec<LessonInfo>>,
    /// `t`: rejected and retired lessons are listed too.
    pub all: bool,
    pub selected: usize,
    pub question: Option<Question>,
    /// The daemon's last refusal, in its words.
    pub error: Option<String>,
    /// The daemon's last `Done` text (`approved 1`, …).
    pub message: Option<String>,
    /// The latest `List`'s id: only its reply fills the page.
    pub(crate) list_id: Option<u64>,
}

impl LessonsPage {
    pub fn selected_lesson(&self) -> Option<&LessonInfo> {
        self.lessons.as_ref()?.get(self.selected)
    }
}

/// Decision 36: how many lessons await review.
pub fn to_review(lessons: &[LessonInfo]) -> usize {
    lessons
        .iter()
        .filter(|l| l.status == LessonStatus::Proposed)
        .count()
}

fn page_of(s: &mut ProfileScreen) -> Option<&mut LessonsPage> {
    match &mut s.page {
        Some(ProfilePage::Lessons(page)) => Some(page),
        _ => None,
    }
}

impl App {
    fn lessons_send(&mut self, dir: PathBuf, list: bool, request: LessonsRequest) -> Effect {
        let request = RunRequest::Lessons(request);
        let timeout = super::reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        let what = crate::app::replies::PendingWhat::Profile {
            dir,
            ask: ProfileAsk::Lessons { list },
        };
        self.replies.insert(id, what, timeout);
        effect
    }

    /// Decision 36: the screen's own `List` when it opens, with its other asks.
    pub(super) fn lessons_list_on_open(&mut self, dir: PathBuf) -> Effect {
        let request = LessonsRequest::List {
            dir: dir.clone(),
            all: false,
        };
        self.lessons_send(dir, true, request)
    }

    /// The page's `List` (with its `all`), recorded as the page's latest.
    pub(in crate::app) fn lessons_list(&mut self) -> Vec<Effect> {
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return vec![];
        }
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        let dir = s.dir.clone();
        let Some(all) = page_of(s).map(|p| p.all) else {
            return vec![];
        };
        let request = LessonsRequest::List {
            dir: dir.clone(),
            all,
        };
        let effect = self.lessons_send(dir, true, request);
        if let (Some(page), Effect::Send(proto::ClientMsg::RunTagged { id, .. })) =
            (self.profile_screen_mut().and_then(page_of), &effect)
        {
            page.list_id = Some(*id);
        }
        vec![effect]
    }

    /// `l` on the Profile screen.
    pub(super) fn open_lessons_page(&mut self) -> Vec<Effect> {
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        s.page = Some(ProfilePage::Lessons(Box::default()));
        self.lessons_list()
    }

    /// A `Decide` or `Forget` of `id`, or of the selected lesson when `None`. A
    /// question's answer passes the id it asked about: a list reply may have moved the
    /// rows under the selection meanwhile (task 12's review, I1).
    fn lessons_act(
        &mut self,
        id: Option<String>,
        make: impl FnOnce(PathBuf, Vec<String>) -> LessonsRequest,
    ) -> Vec<Effect> {
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return vec![];
        }
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        let dir = s.dir.clone();
        let Some(id) = id.or_else(|| {
            page_of(s)
                .and_then(|p| p.selected_lesson())
                .map(|l| l.id.clone())
        }) else {
            return vec![];
        };
        vec![self.lessons_send(dir.clone(), false, make(dir, vec![id]))]
    }

    /// The page's keys (decision 35); a question takes every key until it is answered.
    pub(super) fn on_lessons_page_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(page) = self.profile_screen_mut().and_then(page_of) else {
            return vec![];
        };
        if let Some(question) = page.question.clone() {
            return match key.code {
                KeyCode::Char('y') => {
                    page.question = None;
                    self.lessons_act(Some(question.id), |dir, ids| match question.verb {
                        Verb::Reject => LessonsRequest::Decide {
                            dir,
                            ids,
                            approve: false,
                        },
                        Verb::Forget => LessonsRequest::Forget { dir, ids },
                    })
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    page.question = None;
                    vec![]
                }
                _ => vec![],
            };
        }
        let last = page
            .lessons
            .as_ref()
            .map_or(0, |l| l.len().saturating_sub(1));
        let ask = |page: &mut LessonsPage, verb: Verb| {
            if let Some(lesson) = page.selected_lesson() {
                let id = lesson.id.clone();
                page.question = Some(Question { verb, id });
            }
        };
        match key.code {
            KeyCode::Esc => {
                if let Some(s) = self.profile_screen_mut() {
                    s.page = None;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => page.selected = (page.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => page.selected = page.selected.saturating_sub(1),
            KeyCode::Char('r') => ask(page, Verb::Reject),
            KeyCode::Char('d') => ask(page, Verb::Forget),
            KeyCode::Char('a') => {
                return self.lessons_act(None, |dir, ids| LessonsRequest::Decide {
                    dir,
                    ids,
                    approve: true,
                });
            }
            KeyCode::Char('t') => {
                page.all = !page.all;
                return self.lessons_list();
            }
            KeyCode::Char('g') => return self.open_lessons_graph(),
            _ => {}
        }
        vec![]
    }

    /// `route_reply`'s lessons arm: a reply to one of the screen's lessons requests.
    /// A list fills the open page (its latest only) and the hint line's count; a `Done`
    /// or a refusal shows on the open page, which then lists again; with the page
    /// closed an outcome is toasted, and a view's refusal is dropped.
    pub(in crate::app) fn route_lessons_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        let request_id = match reply {
            RunReply::Lessons { request_id, .. }
            | RunReply::Done { request_id, .. }
            | RunReply::Refused { request_id, .. } => *request_id,
            _ => return None,
        };
        let Some(crate::app::replies::PendingWhat::Profile {
            dir,
            ask: ProfileAsk::Lessons { list },
        }) = request_id.and_then(|id| self.replies.peek(id)).cloned()
        else {
            return None;
        };
        self.replies.take(request_id);
        let s = self.profile_screen_mut().filter(|s| s.dir == dir);
        let open = s
            .as_ref()
            .is_some_and(|s| matches!(s.page, Some(ProfilePage::Lessons(_))));
        match reply {
            RunReply::Lessons { lessons, .. } => {
                if let Some(s) = s {
                    s.to_review = to_review(lessons);
                    if let Some(page) = page_of(s).filter(|p| p.list_id == request_id) {
                        page.selected = page.selected.min(lessons.len().saturating_sub(1));
                        page.lessons = Some(lessons.clone());
                    }
                }
                Some(vec![])
            }
            RunReply::Done { message, .. } | RunReply::Refused { message, .. } if open => {
                let refused = matches!(reply, RunReply::Refused { .. });
                if let Some(page) = s.and_then(page_of) {
                    page.error = refused.then(|| message.clone());
                    page.message = (!refused).then(|| message.clone());
                }
                // A refused list is shown, never asked again (no loop).
                Some(if refused && list {
                    vec![]
                } else {
                    self.lessons_list()
                })
            }
            _ if list => Some(vec![]),
            RunReply::Done { message, .. } => {
                self.toast_at(ToastLevel::Info, crate::app::runs::capped(message));
                Some(self.lessons_recount(&dir))
            }
            RunReply::Refused { message, .. } => {
                let text = crate::app::runs::first_line_and_more(message)
                    .unwrap_or_else(|| "lessons refused".into());
                self.toast_at(ToastLevel::Error, text);
                Some(self.lessons_recount(&dir))
            }
            _ => Some(vec![]),
        }
    }

    /// The screen still open on `dir` (its page closed) counts again after an outcome.
    fn lessons_recount(&mut self, dir: &Path) -> Vec<Effect> {
        match self.profile_screen_mut() {
            Some(s) if s.dir == dir => vec![self.lessons_list_on_open(dir.to_path_buf())],
            _ => vec![],
        }
    }

    /// No reply will come for a lessons request on `dir`: the open page says `why`;
    /// `true` when the screen is open on `dir` and it was a list (a view: no toast), or
    /// the page showed it.
    pub(in crate::app) fn lessons_failed(&mut self, dir: &Path, list: bool, why: &str) -> bool {
        let Some(s) = self.profile_screen_mut().filter(|s| s.dir == dir) else {
            return false;
        };
        match page_of(s) {
            Some(page) => {
                page.error = Some(why.to_string());
                true
            }
            None => list,
        }
    }
}
