//! Milestone 9.15 decision 48 (task M9.15.15): the lessons graph page's replies, by
//! request id. The latest `Graph` fills the page (laid out once); a `Decide` or
//! `Forget` outcome shows on the page and the graph is asked again; a request that
//! will get no reply says so on the page. Mounted from `app/lessons_graph_screen.rs`.
//! Pure.

use crate::app::replies::PendingWhat;
use crate::app::{App, Effect, ToastLevel};
use proto::RunReply;

impl App {
    /// `route_reply`'s graph arm: a reply to one of the page's requests. The latest
    /// graph fills the page; an outcome shows on it and the graph is asked again; a
    /// reply for a page that has closed is dropped (an outcome is toasted).
    pub(in crate::app) fn route_lessons_graph_reply(
        &mut self,
        reply: &RunReply,
    ) -> Option<Vec<Effect>> {
        let request_id = match reply {
            RunReply::LessonsGraph { request_id, .. }
            | RunReply::Done { request_id, .. }
            | RunReply::Refused { request_id, .. } => *request_id,
            _ => return None,
        };
        let Some(PendingWhat::LessonsGraph { dir, graph }) =
            request_id.and_then(|id| self.replies.peek(id)).cloned()
        else {
            return None;
        };
        self.replies.take(request_id);
        let Some(s) = self.graph_screen_mut().filter(|s| s.dir == dir) else {
            match reply {
                RunReply::Done { message, .. } if !graph => {
                    self.toast_at(ToastLevel::Info, crate::app::runs::capped(message));
                }
                RunReply::Refused { message, .. } if !graph => {
                    let text = crate::app::runs::first_line_and_more(message)
                        .unwrap_or_else(|| "lessons refused".into());
                    self.toast_at(ToastLevel::Error, text);
                }
                _ => {}
            }
            return Some(vec![]);
        };
        match reply {
            RunReply::LessonsGraph { graph: g, .. } => {
                if s.graph_id == request_id {
                    s.install((**g).clone());
                    s.error = None;
                }
                Some(vec![])
            }
            RunReply::Refused { message, .. } if graph => {
                s.error = Some(message.clone());
                Some(vec![])
            }
            RunReply::Done { message, .. } | RunReply::Refused { message, .. } => {
                let refused = matches!(reply, RunReply::Refused { .. });
                s.error = refused.then(|| message.clone());
                s.message = (!refused).then(|| message.clone());
                Some(self.graph_request())
            }
            _ => Some(vec![]),
        }
    }

    /// No reply will come for a graph page request on `dir`: the open page says `why`;
    /// `true` when it did.
    pub(in crate::app) fn lessons_graph_failed(
        &mut self,
        dir: &std::path::Path,
        why: &str,
    ) -> bool {
        match self.graph_screen_mut().filter(|s| s.dir == dir) {
            Some(s) => {
                s.error = Some(why.to_string());
                true
            }
            None => false,
        }
    }
}
