//! Decision 30 (task M9.2.10): the `reply_comment` plan edit, the orchestrator's (and
//! the user's, through `anthrex run edit --file`) answer on a review thread, and the
//! text and marker of every reply anthrex posts. Like M9's `message` and `refresh` it
//! is the only edit in its call (`edits_orch::one_edit_rule`). Accepted, it queues a
//! reply on the thread's stage and makes the thread `replied`; the delivery pass sends
//! it. A reply's body is agent text going to GitHub as the user's own comment, so it
//! is made safe first: no hidden characters, no `@` that would mention anyone, and no
//! `<!--` that would forge a marker (anthrex finds a reply it already posted by its
//! marker, so a forged one could suppress a real reply). Pure (design decision 1).

use proto::PrState;
use proto::safe_text::is_hidden_format;

use super::{ReplyDue, ThreadState};
use crate::host::ReplyTarget;
use crate::run::model::Run;
use crate::run::orch::EditSource;
use crate::run::plan::PlanError;

/// Decision 30: a reply's body is 1 to this many characters.
pub const REPLY_BODY_MAX: usize = 4000;

/// `<!-- anthrex:reply <run> <pr>:<key> <sha7> -->` (Interfaces "Messages").
pub fn marker(run_id: &str, pr: u64, key: &str, sha7: &str) -> String {
    format!("<!-- anthrex:reply {run_id} {pr}:{key} {sha7} -->")
}

/// Where a reply to thread `key` goes: in the review thread (`t<id>`, its first
/// comment's id), else the PR's conversation (`c<id>`, `r<id>`).
pub fn target(key: &str) -> ReplyTarget {
    match key.strip_prefix('t').and_then(|id| id.parse().ok()) {
        Some(comment_id) => ReplyTarget::Thread { comment_id },
        None => ReplyTarget::Conversation,
    }
}

/// Agent text made safe to post as the user's comment: line endings normalised, hidden
/// format characters dropped, other controls but `\n` made spaces, every `@` the
/// full-width `＠` (GitHub's mention filter does not read it, as in the PR body), and
/// `<!--` written `&lt;!--` (shown as typed; it opens no comment, so no marker).
pub fn safe_body(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text: String = (text.chars())
        .filter(|c| !is_hidden_format(*c))
        .map(|c| match c {
            '\n' => c,
            '@' => '\u{FF20}',
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => ' ',
            c => c,
        })
        .collect();
    text.replace("<!--", "&lt;!--")
}

/// A `reply_comment`'s `<sha7>`: it has no fix commit, so seven hexadecimal digits of
/// an FNV-1a hash of the thread, the time and the body. Computed once and kept with the
/// reply, so a restart re-sends the same marker and the host finds a post it made.
fn token(key: &str, now: u64, body: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{key}\n{now}\n{body}").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:07x}", hash & 0x0fff_ffff)
}

/// Decision 30's `reply_comment { pr, thread, body }`, applied to the batch's copy of
/// the run: refused with the brief's texts, in its order; accepted, a reply is queued
/// on the thread's stage and the thread is `replied`. `thread` is the key (`t9`), or
/// the ref (`142:t9`).
pub fn apply(
    run: &mut Run,
    (pr, thread, body): (u64, &str, &str),
    source: &EditSource,
    now: u64,
) -> Result<(), PlanError> {
    let error = |text: String| PlanError::new(None, "", "30", text);
    if matches!(source, EditSource::Planner { .. }) {
        return Err(error("a sub-planner cannot reply to a comment".into()));
    }
    let open =
        |n: &u16| (run.delivery.pr(*n)).is_some_and(|p| p.number == pr && p.state == PrState::Open);
    let pr_mode = run.delivery.mode == proto::DeliveryMode::Pr;
    let stages = 1..=u16::try_from(run.delivery.stages.len()).unwrap_or(u16::MAX);
    let Some(n) = stages.into_iter().find(open).filter(|_| pr_mode) else {
        return Err(error(format!(
            "pr #{pr} is not an open stage PR of run {}",
            run.id
        )));
    };
    let key = match thread.split_once(':') {
        Some((p, key)) if p == pr.to_string() => key,
        _ => thread,
    };
    let known = (run.delivery.stage(n)).is_some_and(|s| s.threads.iter().any(|t| t.key == key));
    if !known {
        return Err(error(format!("unknown thread {pr}:{key}")));
    }
    let count = body.chars().count();
    if body.trim().is_empty() || count > REPLY_BODY_MAX {
        return Err(error(format!(
            "reply_comment: body must be 1 to {REPLY_BODY_MAX} characters"
        )));
    }
    if !run.delivery.limits.reply_to_comments {
        return Err(error(
            "replies are turned off ([delivery] reply_to_comments = false)".into(),
        ));
    }
    let body = safe_body(body);
    let marker = marker(&run.id, pr, key, &token(key, now, &body));
    let Some(stage) = run.delivery.stages.get_mut(usize::from(n) - 1) else {
        return Err(error(format!("unknown thread {pr}:{key}")));
    };
    stage.replies.push(ReplyDue {
        thread: key.to_string(),
        target: target(key),
        body,
        marker,
        task: None,
        push: None,
        ready: true,
        sent: false,
    });
    if let Some(t) = stage.threads.iter_mut().find(|t| t.key == key) {
        t.state = ThreadState::Replied { comment_id: 0 };
    }
    Ok(())
}
