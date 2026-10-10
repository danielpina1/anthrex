//! Milestone 9.0.6 decisions 33-35 and milestone 9.10 decisions 27-31: the Profile
//! screen's state, requests and replies. `C-b P` opens it on `goal_project()`; it asks
//! the daemon three tagged things (the status, the stored profile, the proposal), polls
//! the status once a second while a detection or a row edit's check runs, and sends
//! `Detect`, `Edit`, `Confirm`, `Reject` and `RevertEdit` from its keys and pages. It
//! is one page: the review card while a review proposal is ready, else the profile.
//! Replies come back by `request_id` through `app/replies.rs::route_reply`.
//! The view of a profile is `crate::profile_view`; drawing is `ui/profile.rs`. Pure:
//! every request leaves as an `Effect`, and the poll's clock is `screens_tick`'s `now`.

use super::plan_review::LEAVE_REVIEW_FIRST;
use super::replies::{PendingWhat, reply_timeout};
use super::screens::Screen;
use super::{App, Effect};
use crate::profile_view::{self, ENV_ADD, Row};
use crate::text_area::TextArea;
use proto::{
    DroppedCommand, ProfileRequest, ProfileSource, ProfileStatus, ProfileVerification,
    ProposalOrigin, ProposalRecord, ProposalState, RepoProfile, RowEdit, RowEditState, RunRequest,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Interfaces "Exact user-visible text": `C-b P` with no project.
pub const NO_PROJECT: &str = "select a Git project to see its profile";
/// Decision 34: one `Status` a second while a detection runs.
pub const POLL_EVERY: Duration = Duration::from_secs(1);

/// Final review D-I2: how long after **Use this** with goals waiting the screen polls
/// the status, so a goal that could not start shows among the dropped ones.
pub const DRAIN_WATCH: Duration = Duration::from_secs(30);

/// The key of the `Advanced ▸` line among the rows (decision 29: Enter toggles it).
pub const ADVANCED_ROW: &str = "advanced";

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
    /// The side it edits, fixed when it opened: the proposal (opened on the card) or
    /// the stored profile; the card coming or going meanwhile does not move it.
    pub on_proposal: bool,
}

/// A dialog over the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfilePage {
    Detect {
        trust_project: bool,
        unconfined_checks: bool,
        focus: usize,
    },
    /// Decision 30: discard the review proposal (and its queued goals).
    Discard,
    Unset {
        key: String,
        /// As `Editor::on_proposal`.
        on_proposal: bool,
    },
    /// Decisions 30 and 32: the unreadable profile file's text, exactly.
    RawText {
        text: String,
        scroll: usize,
    },
    /// Decision 30: one row's detail.
    Row {
        key: String,
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
    Discard,
    RevertEdit,
}

/// An edit sent from the screen whose outcome is not known yet: once its check passes
/// (no row edit left) and its side's value moved from `before`, the message row says
/// `saved <label>` (decision 31). Forgotten once its outcome is known either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Saving {
    pub key: String,
    pub on_proposal: bool,
    pub before: Option<String>,
    /// Its `Done` came (the views asked after it say how it ended).
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileScreen {
    pub dir: PathBuf,
    pub status: Option<ProfileStatus>,
    pub stored: Side,
    pub proposal: Side,
    pub selected: usize,
    /// Decision 29: Advanced is open (`a`).
    pub advanced: bool,
    /// The row whose output is shown (`o`).
    pub expanded: Option<String>,
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
    pub(crate) saving: Option<Saving>,
    /// The request id of the last `s`, `r` or **Use this**: another waits for its reply.
    pub(crate) acting: Option<u64>,
    /// Until when the status is polled after **Use this** started queued goals.
    pub(crate) drain_until: Option<Instant>,
}

/// Interfaces-style refusal of `C-b a`, `C-b m` and `C-b t` over a full screen.
pub const LEAVE_SCREEN_FIRST: &str = "leave the profile first (esc)";

impl ProfileScreen {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            status: None,
            stored: Side::Loading,
            proposal: Side::Loading,
            selected: 0,
            advanced: false,
            expanded: None,
            page: None,
            error: None,
            message: None,
            status_id: None,
            status_sent_at: None,
            status_failed: None,
            show_sent_at: [None; 2],
            show_id: [None; 2],
            saving: None,
            acting: None,
            drain_until: None,
        }
    }

    fn side_mut(&mut self, proposed: bool) -> &mut Side {
        if proposed {
            &mut self.proposal
        } else {
            &mut self.stored
        }
    }

    pub(super) fn profile_of(side: &Side) -> Option<&RepoProfile> {
        match side {
            Side::Ready(shown) => shown.profile.as_ref(),
            _ => None,
        }
    }

    /// The status's proposal when it is a review proposal (decision 3: its origin is
    /// not `Edit`).
    fn review(&self) -> Option<&ProposalRecord> {
        let p = self.status.as_ref()?.proposal.as_ref()?;
        (!matches!(p.origin, ProposalOrigin::Edit { .. })).then_some(p)
    }

    /// Decision 29: `x` is offered (a review proposal exists, in any state).
    pub fn review_proposal(&self) -> bool {
        self.review().is_some()
    }

    /// Decision 27: the card shows while the review proposal is `Ready`.
    pub fn showing_card(&self) -> bool {
        self.review()
            .is_some_and(|p| p.state == ProposalState::Ready)
    }

    /// Decision 15: the row edit of the status's proposal (on the stored profile, or on
    /// the review proposal).
    pub fn row_edit(&self) -> Option<&RowEdit> {
        self.status.as_ref()?.proposal.as_ref()?.edit.as_ref()
    }

    /// Decision 31: the state of `key`'s row edit, when it has one.
    pub fn edit_of(&self, key: &str) -> Option<&RowEditState> {
        self.row_edit().filter(|e| e.key == key).map(|e| &e.state)
    }

    /// The row edit when its check failed (decision 31's ✗), whatever its row.
    pub fn failed_row_edit(&self) -> Option<&RowEdit> {
        self.row_edit()
            .filter(|e| matches!(e.state, RowEditState::Failed { .. }))
    }

    /// A row edit's check is running (the screen polls meanwhile).
    pub fn row_checking(&self) -> bool {
        self.row_edit()
            .is_some_and(|e| e.state == RowEditState::Verifying)
    }

    /// Whether the repository has a stored profile (assumed while nothing says).
    pub fn has_stored(&self) -> bool {
        match &self.status {
            Some(status) => status.source == ProfileSource::Stored,
            None => !matches!(self.stored, Side::Absent(_)),
        }
    }

    /// Decision 30: the detect page's title.
    pub fn detect_title(&self) -> &'static str {
        if self.has_stored() {
            "detect again"
        } else {
            "set up"
        }
    }

    /// Decisions 8 and 30: what the discard page says.
    pub fn discard_text(&self) -> String {
        let mut text = format!(
            "the proposal for {} is deleted; a running scout or verification stops",
            self.dir.display()
        );
        match self.status.as_ref().map_or(0, |s| s.queued.len()) {
            0 => {}
            1 => text.push_str("; 1 queued goal is dropped"),
            n => text.push_str(&format!("; {n} queued goals are dropped")),
        }
        text
    }

    /// What is listed now (decisions 27-29): the card's rows while it shows, else the
    /// stored profile's; in section order (`profile_view::rows`), the main sections, then the `Advanced` line
    /// and, open, what it holds. The changes card lists only changed rows, unfolded.
    pub fn rows(&self) -> Vec<Row> {
        let (side, against) = if self.showing_card() {
            let against = match &self.stored {
                Side::Ready(shown) => shown.profile.clone(),
                _ if self.status.as_ref().is_some_and(|s| {
                    s.source == ProfileSource::Stored && s.unparseable.is_none()
                }) =>
                {
                    return vec![];
                }
                _ => None,
            };
            (&self.proposal, Some(against))
        } else {
            (&self.stored, None)
        };
        let Side::Ready(shown) = side else {
            return vec![];
        };
        let Some(profile) = &shown.profile else {
            return vec![];
        };
        let verification = shown.verification.as_ref();
        let rows = match &against {
            Some(Some(stored)) => {
                let rows = profile_view::rows(profile, verification, Some(stored));
                return profile_view::changed(rows);
            }
            Some(None) => profile_view::rows(profile, verification, None)
                .into_iter()
                .filter(|r| r.value.is_some() && r.key != ENV_ADD)
                .collect(),
            None => profile_view::rows(profile, verification, None),
        };
        let (main, advanced): (Vec<Row>, Vec<Row>) = rows.into_iter().partition(|r| !r.advanced);
        let mut out = main;
        out.push(advanced_row());
        if self.advanced {
            out.extend(advanced);
        }
        out
    }

    /// `key`'s value as a row shows it, on the proposal or on the stored profile.
    pub(super) fn value_on(&self, on_proposal: bool, key: &str) -> Option<String> {
        let side = if on_proposal {
            &self.proposal
        } else {
            &self.stored
        };
        profile_view::rows(Self::profile_of(side)?, None, None)
            .into_iter()
            .find(|r| r.key == key)
            .and_then(|r| r.value)
    }

    /// The index of `key` among the rows, when listed.
    pub(super) fn index_of(&self, key: &str) -> Option<usize> {
        self.rows().iter().position(|r| r.key == key)
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

    /// Decision 31: an edit sent from here ended: no row edit is left for it, and the
    /// status and its side arrived after its `Done` (`settled`: nothing asked since is
    /// still out). A moved value says `saved <label>`; either way it is forgotten.
    pub(super) fn note_saved(&mut self, settled: bool) {
        let Some(saving) = &self.saving else {
            return;
        };
        if !saving.done
            || !settled
            || self.status.is_none()
            || self.row_edit().is_some_and(|e| e.key == saving.key)
        {
            return;
        }
        let side = if saving.on_proposal {
            &self.proposal
        } else {
            &self.stored
        };
        if Self::profile_of(side).is_none() {
            return;
        }
        if self.value_on(saving.on_proposal, &saving.key) != saving.before {
            self.message = Some(format!(
                "saved {}",
                crate::profile_words::label(&saving.key)
            ));
            self.error = None;
        }
        self.saving = None;
    }
}

/// The `Advanced ▸` line (decision 29), listed among the rows so it can be selected.
fn advanced_row() -> Row {
    Row {
        section: "",
        label: "Advanced".into(),
        advanced: false,
        old: None,
        key: ADVANCED_ROW.into(),
        value: None,
        mark: None,
        check: None,
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
            Some(dir) => self.open_profile_on(dir),
            None => {
                self.toast(NO_PROJECT);
                vec![]
            }
        }
    }

    /// Opens the screen on `dir` (replacing any open screen) and asks the three things;
    /// a ready review proposal shows as the card (decision 27).
    pub(crate) fn open_profile_on(&mut self, dir: PathBuf) -> Vec<Effect> {
        let screen = ProfileScreen::new(dir);
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
    pub(in crate::app) fn profile_send(
        &mut self,
        dir: PathBuf,
        ask: ProfileAsk,
        request: ProfileRequest,
    ) -> Effect {
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
}

#[path = "profile_keys.rs"]
mod keys;
#[path = "profile_pages.rs"]
mod pages;
#[path = "profile_views.rs"]
mod views;
