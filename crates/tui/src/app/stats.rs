//! Milestone 9.0.6 decision 38: the run-history stats screen, opened from the menu's
//! `Stats` (decision 10's local kind). It sends one tagged `Stats { dir: run.project }`
//! and shows the reply that carries that request's id; `route_reply` brings it here
//! (preflight F29). A reply for a closed screen or an older request changes nothing.
//! A request no longer awaited (expired, link lost, send refused) leaves the screen
//! `Failed`, never loading; a late reply to the screen's own last request still fills
//! it (decision 16, ruling R-b). Drawing is `ui/stats.rs`. Pure: the request leaves as
//! an `Effect`.

use super::replies::{NO_REPLY, PendingWhat, reply_timeout};
use super::screens::Screen;
use super::{App, Effect};
use crossterm::event::{KeyCode, KeyEvent};
use proto::{HistoryStats, RunReply, RunRequest};
use std::path::PathBuf;

/// The other screens' refusal of `C-b a`, `C-b m` and `C-b t`, with this one's name.
pub const LEAVE_STATS_FIRST: &str = "leave the stats first (esc)";
/// The screen's texts when the link went while it waited, and when its request could
/// not be sent while connected (shared with the other screens and the answer form).
pub use super::replies::{NOT_CONNECTED, NOT_SENT};
/// PgUp/PgDn move this many lines, as on the Profile screen.
const PAGE: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatsState {
    /// Waiting on the tagged request with this id.
    Loading(u64),
    Ready(Box<HistoryStats>),
    /// The daemon's refusal, or why no reply will come.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsScreen {
    pub project: PathBuf,
    pub state: StatsState,
    /// The first line shown.
    pub scroll: usize,
    /// The id of the screen's last request: its reply fills the screen even after the
    /// screen stopped waiting (decision 16, ruling R-b).
    pub request: u64,
}

impl App {
    /// The menu's `Stats` (decision 38): the screen on `project`, replacing any open
    /// screen, loading on one tagged `Stats { dir }`.
    pub(crate) fn open_stats(&mut self, project: PathBuf) -> Vec<Effect> {
        let (request, effect) = self.stats_request(&project);
        let screen = StatsScreen {
            project,
            state: StatsState::Loading(request),
            scroll: 0,
            request,
        };
        self.set_screen(Some(Screen::Stats(Box::new(screen))));
        vec![effect]
    }

    fn stats_request(&mut self, dir: &std::path::Path) -> (u64, Effect) {
        let request = RunRequest::Stats {
            dir: dir.into(),
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: false,
        };
        let timeout = reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        let what = PendingWhat::Stats { dir: dir.into() };
        self.replies.insert(id, what, timeout);
        (id, effect)
    }

    fn stats_screen_mut(&mut self) -> Option<&mut StatsScreen> {
        match &mut self.screen {
            Some(Screen::Stats(s)) => Some(s),
            _ => None,
        }
    }

    /// The id the open screen still waits on, while it is pending.
    pub(super) fn stats_awaited(&self) -> Option<u64> {
        match &self.screen {
            Some(Screen::Stats(s)) => match s.state {
                StatsState::Loading(id) if self.replies.contains(id) => Some(id),
                _ => None,
            },
            _ => None,
        }
    }

    /// `route_reply`'s stats arm. A `Stats` or a `Refused` for the open screen's own
    /// last request fills it, whether it still waits or already gave up (decision 16,
    /// ruling R-b); a reply to any other stats request of ours (a closed screen, an
    /// older request), pending or expired, is dropped.
    pub(super) fn route_stats_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        let (id, state) = match reply {
            RunReply::Stats { stats, request_id } => {
                (*request_id, StatsState::Ready(Box::new(stats.clone())))
            }
            RunReply::Refused {
                message,
                request_id,
                ..
            } => (*request_id, StatsState::Failed(message.clone())),
            _ => return None,
        };
        let id = id?;
        let own = matches!(
            &self.screen,
            Some(Screen::Stats(s)) if s.request == id && !matches!(s.state, StatsState::Ready(_))
        );
        match self.replies.peek(id) {
            Some(PendingWhat::Stats { .. }) => {
                self.replies.take(Some(id));
            }
            Some(_) => return None,
            None if own || self.replies.expired_quietly(id) => {}
            None => return None,
        }
        if own && let Some(s) = self.stats_screen_mut() {
            s.state = state;
            s.scroll = 0;
        }
        Some(vec![])
    }

    /// Never stuck on loading: once the screen's request is no longer awaited (it
    /// expired, or the link went), the screen says so. Called by `screens_tick` and
    /// right after `expire_replies`.
    pub(super) fn stats_tick(&mut self) -> Vec<Effect> {
        let text = if self.connected() {
            NO_REPLY
        } else {
            NOT_CONNECTED
        };
        self.stats_give_up(None, text);
        vec![]
    }

    /// `on_send_failed`: the request `id` was never sent, so it is not waited on; a
    /// screen loading on it says so at once.
    pub(super) fn stats_not_sent(&mut self, id: u64) {
        self.replies.take(Some(id));
        let text = if self.connected() {
            NOT_SENT
        } else {
            NOT_CONNECTED
        };
        self.stats_give_up(Some(id), text);
    }

    /// A screen loading on a request no longer awaited (on `id`, when given) fails
    /// with `text`.
    fn stats_give_up(&mut self, id: Option<u64>, text: &str) {
        let waiting = self.stats_awaited();
        if let Some(s) = self.stats_screen_mut()
            && let StatsState::Loading(on) = s.state
            && waiting.is_none()
            && id.is_none_or(|id| id == on)
        {
            s.state = StatsState::Failed(text.into());
        }
    }

    /// A new connection: an open screen with no history yet asks again, once.
    pub(super) fn stats_reconnected(&mut self) -> Vec<Effect> {
        let dir = match &self.screen {
            Some(Screen::Stats(s)) if !matches!(s.state, StatsState::Ready(_)) => s.project.clone(),
            _ => return vec![],
        };
        let (request, effect) = self.stats_request(&dir);
        if let Some(s) = self.stats_screen_mut() {
            s.state = StatsState::Loading(request);
            s.request = request;
            s.scroll = 0;
        }
        vec![effect]
    }

    /// The screen's keys: `j`/`k`/Down/Up a line, PgDn/PgUp a page, Esc leaves. The
    /// scroll stops where the view stops: at `ui::stats::max_scroll` over the body the
    /// renderer last reported, clamped first (a resize or a new reply may have moved
    /// it), as the plan review's page keys are.
    pub(super) fn on_stats_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if key.code == KeyCode::Esc {
            self.set_screen(None);
            return vec![];
        }
        let last = match &self.screen {
            Some(Screen::Stats(s)) => crate::ui::stats::max_scroll(self, s, self.body_area),
            _ => return vec![],
        };
        let Some(s) = self.stats_screen_mut() else {
            return vec![];
        };
        let from = s.scroll.min(last);
        s.scroll = match key.code {
            KeyCode::Char('j') | KeyCode::Down => from + 1,
            KeyCode::Char('k') | KeyCode::Up => from.saturating_sub(1),
            KeyCode::PageDown => from + PAGE,
            KeyCode::PageUp => from.saturating_sub(PAGE),
            _ => from,
        }
        .min(last);
        vec![]
    }
}
