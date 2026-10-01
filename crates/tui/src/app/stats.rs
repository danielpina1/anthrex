//! Milestone 9.0.6 decision 38: the run-history stats screen, opened from the menu's
//! `Stats` (decision 10's local kind). It sends one tagged `Stats { dir: run.project }`
//! and shows the reply that carries that request's id; `route_reply` brings it here
//! (preflight F29). A reply for a closed screen or an older request, and a late one,
//! changes nothing. A request no longer awaited (expired, link lost, send refused)
//! leaves the screen `Failed`, never loading. Drawing is `ui/stats.rs`. Pure: the
//! request leaves as an `Effect`.

use super::replies::{NO_REPLY, PendingWhat, reply_timeout};
use super::screens::Screen;
use super::{App, Effect};
use crossterm::event::{KeyCode, KeyEvent};
use proto::{HistoryStats, RunReply, RunRequest};
use std::path::PathBuf;

/// The other screens' refusal of `C-b a`, `C-b m` and `C-b t`, with this one's name.
pub const LEAVE_STATS_FIRST: &str = "leave the stats first (esc)";
/// The screen's text when the link went while it waited.
pub const NOT_CONNECTED: &str = "not connected";
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
}

impl StatsScreen {
    /// How many lines `ui::stats::body_lines` draws (one row each, never wrapped
    /// while ready), so the scroll stops at the last one.
    pub fn line_count(&self) -> usize {
        match &self.state {
            StatsState::Ready(stats) => line_count(stats),
            _ => 1,
        }
    }
}

/// Decision 38's lines: the records line and a blank; the header and one row per class;
/// a blank and the deciders line; a blank, the flaky heading and its rows (or `none`);
/// and, when any, a blank, `problems` and one row each.
pub fn line_count(stats: &HistoryStats) -> usize {
    let problems = if stats.problems.is_empty() {
        0
    } else {
        2 + stats.problems.len()
    };
    2 + 1 + stats.rows.len() + 2 + 2 + stats.flaky_proposals.len().max(1) + problems
}

impl App {
    /// The menu's `Stats` (decision 38): the screen on `project`, replacing any open
    /// screen, loading on one tagged `Stats { dir }`.
    pub(crate) fn open_stats(&mut self, project: PathBuf) -> Vec<Effect> {
        let (state, effects) = self.stats_request(&project);
        let screen = StatsScreen {
            project,
            state,
            scroll: 0,
        };
        self.set_screen(Some(Screen::Stats(Box::new(screen))));
        effects
    }

    fn stats_request(&mut self, dir: &std::path::Path) -> (StatsState, Vec<Effect>) {
        let request = RunRequest::Stats { dir: dir.into() };
        let timeout = reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        let what = PendingWhat::Stats { dir: dir.into() };
        self.replies.insert(id, what, timeout);
        (StatsState::Loading(id), vec![effect])
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

    /// `route_reply`'s stats arm: a `Stats` or a `Refused` whose id is a pending stats
    /// request of ours fills the screen still loading on that id; for a closed screen
    /// or an older request it is dropped, as is a late one (its entry expired).
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
        match self.replies.peek(id) {
            Some(PendingWhat::Stats { .. }) => {}
            None if self.replies.expired_quietly(id) => return Some(vec![]),
            _ => return None,
        }
        self.replies.take(Some(id));
        if let Some(s) = self.stats_screen_mut()
            && s.state == StatsState::Loading(id)
        {
            s.state = state;
            s.scroll = 0;
        }
        Some(vec![])
    }

    /// Never stuck on loading: once the screen's request is no longer awaited (it
    /// expired, the link went, or its send was refused), the screen says so. Called by
    /// `screens_tick` and right after `expire_replies`.
    pub(super) fn stats_tick(&mut self) -> Vec<Effect> {
        let gone = matches!(
            &self.screen,
            Some(Screen::Stats(s)) if matches!(s.state, StatsState::Loading(_))
        ) && self.stats_awaited().is_none();
        let text = if self.connected() {
            NO_REPLY
        } else {
            NOT_CONNECTED
        };
        if gone && let Some(s) = self.stats_screen_mut() {
            s.state = StatsState::Failed(text.into());
        }
        vec![]
    }

    /// A new connection: an open screen with no history yet asks again, once.
    pub(super) fn stats_reconnected(&mut self) -> Vec<Effect> {
        let dir = match &self.screen {
            Some(Screen::Stats(s)) if !matches!(s.state, StatsState::Ready(_)) => s.project.clone(),
            _ => return vec![],
        };
        let (state, effects) = self.stats_request(&dir);
        if let Some(s) = self.stats_screen_mut() {
            s.state = state;
            s.scroll = 0;
        }
        effects
    }

    /// The screen's keys: `j`/`k`/Down/Up a line, PgDn/PgUp a page, Esc leaves.
    pub(super) fn on_stats_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if key.code == KeyCode::Esc {
            self.set_screen(None);
            return vec![];
        }
        let Some(s) = self.stats_screen_mut() else {
            return vec![];
        };
        let last = s.line_count().saturating_sub(1);
        s.scroll = match key.code {
            KeyCode::Char('j') | KeyCode::Down => s.scroll + 1,
            KeyCode::Char('k') | KeyCode::Up => s.scroll.saturating_sub(1),
            KeyCode::PageDown => s.scroll + PAGE,
            KeyCode::PageUp => s.scroll.saturating_sub(PAGE),
            _ => s.scroll,
        }
        .min(last);
        vec![]
    }
}
