//! Milestone 9.0.6 decisions 33-35: the Profile screen's state, keys, requests and
//! replies. `C-b P` opens it on `goal_project()`; it asks the daemon three tagged
//! things (the status, the stored profile, the proposal), polls the status once a second
//! while a detection runs, and sends `Detect`, `Edit`, `Confirm` and `Reject` from its
//! pages. Replies come back by `request_id` through `app/replies.rs::route_reply`.
//! The view of a profile is `crate::profile_view`; drawing is `ui/profile.rs`. Pure:
//! every request leaves as an `Effect`, and the poll's clock is `screens_tick`'s `now`.

use super::plan_review::LEAVE_REVIEW_FIRST;
use super::replies::{PendingWhat, reply_timeout};
use super::runs::{capped, first_line_and_more};
use super::screens::Screen;
use super::{App, Effect, ToastLevel};
use crate::profile_view::{self, Row};
use crate::text_area::TextArea;
use proto::{
    DroppedCommand, ProfileReply, ProfileRequest, ProfileStatus, ProfileVerification,
    ProposalState, RepoProfile, RunReply, RunRequest,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Interfaces "Exact user-visible text": `C-b P` with no project.
pub const NO_PROJECT: &str = "select a Git project to see its profile";
/// Decision 34: one `Status` a second while a detection runs.
pub const POLL_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileTab {
    Status,
    Profile,
}

/// A `ProfileReply::Shown`, with its TOML read back into a profile (`None` when the text
/// does not parse: the rows are then empty, and `c` still shows the text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub toml: String,
    pub profile: Option<RepoProfile>,
    pub verification: Option<ProfileVerification>,
    pub dropped: Vec<DroppedCommand>,
}

/// One side of the screen: the stored profile or the proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    Loading,
    /// A refused `Show` (decision 34): that side's absence, with the daemon's text.
    Absent(String),
    /// No reply will come for the last `Show` (it expired, or was not sent): why. The
    /// 1 s tick asks again (final review, minor 2).
    Failed(String),
    Ready(Box<Shown>),
}

/// The editor fitted to a key's type (decision 35).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorField {
    Line(TextArea),
    List(TextArea),
    Digits(String),
    Choice {
        options: &'static [&'static str],
        at: usize,
    },
    Env {
        name: TextArea,
        value: TextArea,
        on_value: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    pub key: String,
    pub field: EditorField,
    pub error: Option<String>,
    /// Opened from the proposal view: the page says the edit starts from the stored
    /// profile and replaces the proposal (progress ruling).
    pub from_proposal: bool,
}

/// A dialog over the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfilePage {
    Detect {
        trust_project: bool,
        unconfined_checks: bool,
        focus: usize,
    },
    Reject,
    Unset {
        key: String,
    },
    Confirm {
        toml: String,
        scroll: usize,
    },
    Edit(Box<Editor>),
}

/// What a pending profile request asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileAsk {
    Status,
    Show { proposed: bool },
    Detect,
    Edit,
    Confirm,
    Reject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileScreen {
    pub dir: PathBuf,
    pub tab: ProfileTab,
    /// The Profile tab shows the proposal (`p`).
    pub proposed: bool,
    pub status: Option<ProfileStatus>,
    pub stored: Side,
    pub proposal: Side,
    pub selected: usize,
    /// The row whose check output tail is expanded (Enter).
    pub expanded: Option<String>,
    /// `s`: "store once verification passes", the CLI's `--yes`.
    pub store_on_pass: bool,
    pub page: Option<ProfilePage>,
    /// The daemon's last refusal, in the screen's error row (decision 37's among them).
    pub error: Option<String>,
    /// The daemon's last `Done` text.
    pub message: Option<String>,
    pub(crate) status_id: Option<u64>,
    pub(crate) status_sent_at: Option<Instant>,
    /// No status yet and no reply will come for the last `Status`: why (minor 2).
    pub status_failed: Option<String>,
    /// When the last `Show` of each side (stored, proposal) left.
    pub(crate) show_sent_at: [Option<Instant>; 2],
    /// The id of the last `Show` of each side: only its reply fills the side (9.0.7
    /// decision 37), so one sent before a detection never draws the old proposal.
    pub(crate) show_id: [Option<u64>; 2],
}

/// Interfaces-style refusal of `C-b a`, `C-b m` and `C-b t` over a full screen.
pub const LEAVE_SCREEN_FIRST: &str = "leave the profile first (esc)";

impl ProfileScreen {
    fn new(dir: PathBuf, proposal: bool) -> Self {
        Self {
            dir,
            tab: if proposal {
                ProfileTab::Profile
            } else {
                ProfileTab::Status
            },
            proposed: proposal,
            status: None,
            stored: Side::Loading,
            proposal: Side::Loading,
            selected: 0,
            expanded: None,
            store_on_pass: false,
            page: None,
            error: None,
            message: None,
            status_id: None,
            status_sent_at: None,
            status_failed: None,
            show_sent_at: [None; 2],
            show_id: [None; 2],
        }
    }

    fn side_mut(&mut self, proposed: bool) -> &mut Side {
        if proposed {
            &mut self.proposal
        } else {
            &mut self.stored
        }
    }

    /// The side the Profile tab shows.
    pub fn viewed(&self) -> &Side {
        if self.proposed {
            &self.proposal
        } else {
            &self.stored
        }
    }

    fn profile_of(side: &Side) -> Option<&RepoProfile> {
        match side {
            Side::Ready(shown) => shown.profile.as_ref(),
            _ => None,
        }
    }

    /// The Profile tab's rows; on the proposal view marked against the stored profile
    /// (an absent stored profile marks every set key added).
    pub fn rows(&self) -> Vec<Row> {
        // A detection under way replaces the proposal: the old one is not shown.
        if self.proposed && self.in_progress() {
            return vec![];
        }
        let Side::Ready(shown) = self.viewed() else {
            return vec![];
        };
        let Some(profile) = &shown.profile else {
            return vec![];
        };
        let none = RepoProfile::default();
        let against = match (&self.proposed, &self.stored) {
            (false, _) | (true, Side::Loading) => None,
            (true, Side::Absent(_)) => Some(&none),
            (true, side) => Self::profile_of(side),
        };
        profile_view::rows(profile, shown.verification.as_ref(), against)
    }

    /// The proposal's state, when there is one.
    pub fn proposal_state(&self) -> Option<&ProposalState> {
        Some(&self.status.as_ref()?.proposal.as_ref()?.state)
    }

    /// Decision 34: a detection or a verification is under way.
    pub fn in_progress(&self) -> bool {
        matches!(
            self.proposal_state(),
            Some(ProposalState::Preparing | ProposalState::Scouting | ProposalState::Verifying)
        )
    }
}

impl App {
    /// `C-b P` (decision 34): the screen on the chosen project, or the toast saying
    /// there is none. Refused over the plan review (decision 33).
    pub(super) fn open_profile(&mut self) -> Vec<Effect> {
        if self.plan_review.is_some() {
            self.toast(LEAVE_REVIEW_FIRST);
            return vec![];
        }
        match self.goal_project() {
            Some(dir) => self.open_profile_on(dir, false),
            None => {
                self.toast(NO_PROJECT);
                vec![]
            }
        }
    }

    /// Opens the screen on `dir` (replacing any open screen), on the proposal when
    /// `proposal` (the proposal alert, preflight F26), and asks the three things.
    pub(crate) fn open_profile_on(&mut self, dir: PathBuf, proposal: bool) -> Vec<Effect> {
        let screen = ProfileScreen::new(dir, proposal);
        self.set_screen(Some(Screen::Profile(Box::new(screen))));
        self.profile_fetch_all(Instant::now())
    }

    fn profile_screen_mut(&mut self) -> Option<&mut ProfileScreen> {
        match &mut self.screen {
            Some(Screen::Profile(s)) => Some(s),
            _ => None,
        }
    }

    fn profile_dir(&self) -> Option<PathBuf> {
        match &self.screen {
            Some(Screen::Profile(s)) => Some(s.dir.clone()),
            _ => None,
        }
    }

    /// One tagged profile request, recorded with what it asked.
    fn profile_send(&mut self, dir: PathBuf, ask: ProfileAsk, request: ProfileRequest) -> Effect {
        let request = RunRequest::Profile(request);
        let timeout = reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        self.replies
            .insert(id, PendingWhat::Profile { dir, ask }, timeout);
        effect
    }

    fn profile_status(&mut self, now: Instant) -> Vec<Effect> {
        let Some(dir) = self.profile_dir() else {
            return vec![];
        };
        let request = ProfileRequest::Status { dir: dir.clone() };
        let effect = self.profile_send(dir, ProfileAsk::Status, request);
        if let (Some(s), Effect::Send(proto::ClientMsg::RunTagged { id, .. })) =
            (self.profile_screen_mut(), &effect)
        {
            s.status_id = Some(*id);
            s.status_sent_at = Some(now);
        }
        vec![effect]
    }

    fn profile_show(&mut self, proposed: bool, now: Instant) -> Vec<Effect> {
        let Some(dir) = self.profile_dir() else {
            return vec![];
        };
        let request = ProfileRequest::Show {
            dir: dir.clone(),
            proposed,
        };
        let effect = self.profile_send(dir, ProfileAsk::Show { proposed }, request);
        if let (Some(s), Effect::Send(proto::ClientMsg::RunTagged { id, .. })) =
            (self.profile_screen_mut(), &effect)
        {
            s.show_sent_at[usize::from(proposed)] = Some(now);
            s.show_id[usize::from(proposed)] = Some(*id);
        }
        vec![effect]
    }

    /// The status, the stored profile and the proposal, in that order.
    pub(super) fn profile_fetch_all(&mut self, now: Instant) -> Vec<Effect> {
        let mut effects = self.profile_status(now);
        effects.extend(self.profile_show(false, now));
        effects.extend(self.profile_show(true, now));
        effects
    }

    /// `route_reply`'s profile arm. A reply to a profile request of ours applies to the
    /// screen still open on its project; one whose screen closed, or a late one, is
    /// shown as a toast when it reports an outcome (decision 16), and dropped when it is
    /// a view.
    pub(super) fn route_profile_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        let RunReply::Profile { reply, request_id } = reply else {
            return None;
        };
        let pending = request_id.and_then(|id| self.replies.peek(id).cloned());
        let (dir, ask) = match pending {
            Some(PendingWhat::Profile { dir, ask }) => (dir, ask),
            Some(_) => return None,
            None => {
                // A late view (its entry expired) fills the open screen still loading
                // on it, else is dropped (decision 16, minor 2); a late outcome shown.
                let late = request_id.and_then(|id| self.replies.expired_view(id).cloned());
                match late.zip(*request_id) {
                    Some((PendingWhat::Profile { dir, ask }, id))
                        if self.profile_awaits(&dir, ask, id) =>
                    {
                        return Some(self.apply_profile_reply(ask, reply));
                    }
                    Some(_) => {}
                    None if request_id.is_some() => self.toast_profile_outcome(reply),
                    None => {}
                }
                return Some(vec![]);
            }
        };
        self.replies.take(*request_id);
        // Decision 37: a `Show` that is no longer its side's latest (a re-fetch went
        // out after it) describes an older state; the re-fetch's reply fills the side.
        if let ProfileAsk::Show { proposed } = ask
            && !self.profile_latest_show(proposed, *request_id)
        {
            return Some(vec![]);
        }
        if self.profile_dir().as_ref() != Some(&dir) {
            if !matches!(ask, ProfileAsk::Status | ProfileAsk::Show { .. }) {
                self.toast_profile_outcome(reply);
            }
            return Some(vec![]);
        }
        Some(self.apply_profile_reply(ask, reply))
    }

    fn toast_profile_outcome(&mut self, reply: &ProfileReply) {
        match reply {
            ProfileReply::Done { message } => self.toast_at(ToastLevel::Info, capped(message)),
            ProfileReply::Refused { message } => {
                let text = first_line_and_more(message).unwrap_or_else(|| "profile refused".into());
                self.toast_at(ToastLevel::Error, text);
            }
            _ => {}
        }
    }

    fn apply_profile_reply(&mut self, ask: ProfileAsk, reply: &ProfileReply) -> Vec<Effect> {
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        match (ask, reply) {
            (ProfileAsk::Status, ProfileReply::Status(status)) => {
                let was_running = s.in_progress();
                s.status = Some(status.clone());
                s.status_failed = None;
                if status.proposal.is_none() {
                    s.proposal = Side::Absent("no proposal".into());
                }
                // Decision 34: leaving the running states (for `Ready`, or `Failed`:
                // minor 1) fetches the proposal once; the old one is not shown meanwhile.
                if was_running && !s.in_progress() && status.proposal.is_some() {
                    s.proposal = Side::Loading;
                    return self.profile_show(true, Instant::now());
                }
                vec![]
            }
            (
                ProfileAsk::Show { proposed },
                ProfileReply::Shown {
                    toml,
                    verification,
                    dropped,
                    ..
                },
            ) => {
                let shown = Shown {
                    toml: toml.clone(),
                    profile: toml::from_str(toml).ok(),
                    verification: verification.clone(),
                    dropped: dropped.clone(),
                };
                *s.side_mut(proposed) = Side::Ready(Box::new(shown));
                vec![]
            }
            (ProfileAsk::Show { proposed }, ProfileReply::Refused { message }) => {
                *s.side_mut(proposed) = Side::Absent(message.clone());
                vec![]
            }
            (_, ProfileReply::Refused { message }) => {
                // A refused `Status` is not asked again (minor 2's retry is for silence).
                s.status_failed = None;
                s.error = Some(message.clone());
                s.message = None;
                vec![]
            }
            (ProfileAsk::Detect, ProfileReply::Done { message }) => {
                s.message = Some(message.clone());
                s.error = None;
                self.profile_status(Instant::now())
            }
            (_, ProfileReply::Done { message }) => {
                s.message = Some(message.clone());
                s.error = None;
                if ask == ProfileAsk::Confirm {
                    s.proposed = false;
                }
                self.profile_fetch_all(Instant::now())
            }
            _ => vec![],
        }
    }
}

#[path = "profile_keys.rs"]
mod keys;
#[path = "profile_pages.rs"]
mod pages;
#[path = "profile_views.rs"]
mod views;
