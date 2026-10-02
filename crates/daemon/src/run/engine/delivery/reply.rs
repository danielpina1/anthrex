//! Decision 30 (task M9.2.10): replies on a stage PR's threads. Once a push carrying
//! a review fix's merge has landed on the remote (or the remote branch was adopted,
//! which holds it), each thread the fix addressed gets `Addressed in <sha7> by task
//! <id>.` (a conversation thread's begins `@<login> `) with anthrex's marker; a
//! `reply_comment` is sent at once. A stage's due reply goes before its views, and the
//! posted comment's id is anthrex's own from its answer on. Pure.

use proto::TaskOrigin;

use super::super::Effect;
use super::super::requests::log;
use super::review::author;
use super::watch::named;
use super::{emit, host_busy, stage_mut};
use crate::host::HostError;
use crate::run::contract::sha7;
use crate::run::delivery::ReplyDue;
use crate::run::delivery::ThreadState;
use crate::run::delivery::ops::HostOp;
use crate::run::delivery::reply_edit::{marker, target};
use crate::run::model::{FixOf, Run};

/// Step 4: each merged review fix of stage `n` queues one reply per thread, once
/// (none with `reply_to_comments = false`; its threads stay `tasked`).
pub(super) fn queue_replies(run: &mut Run, n: u16) {
    let Some(number) = run.delivery.pr(n).map(|p| p.number) else {
        return;
    };
    let merged: Vec<(String, String, Vec<String>)> = (run.tasks.iter())
        .filter(|t| t.state == proto::TaskState::Merged && t.origin == TaskOrigin::Review)
        .filter_map(|t| match (&t.fixes, &t.merge_commit) {
            (Some(FixOf::Review { stage, pr, threads }), Some(commit))
                if *stage == n && *pr == number =>
            {
                Some((t.id().to_string(), commit.clone(), threads.clone()))
            }
            _ => None,
        })
        .collect();
    let on = run.delivery.limits.reply_to_comments;
    for (id, commit, threads) in merged {
        for reference in threads {
            let key = reference.split_once(':').map_or(&reference[..], |(_, k)| k);
            let tag = format!("{id}/{key}");
            if !stage_mut(run, n).auto_replies.insert(tag) || !on {
                continue;
            }
            let mut body = format!("Addressed in {} by task {id}.", sha7(&commit));
            let login = author(run, n, key);
            if !key.starts_with('t') && login != "<unknown>" {
                body = format!("@{login} {body}");
            }
            let reply = ReplyDue {
                thread: key.to_string(),
                target: target(key),
                body,
                marker: marker(&run.id, number, key, sha7(&commit)),
                task: Some(id.clone()),
                push: None,
                ready: false,
                sent: false,
            };
            stage_mut(run, n).replies.push(reply);
        }
    }
}

/// A push of stage `n`'s `sha` goes out: the replies waiting for a push wait for it.
pub(super) fn pushing(run: &mut Run, n: u16, sha: &str) {
    for r in stage_mut(run, n).replies.iter_mut().filter(|r| !r.ready) {
        r.push = Some(sha.to_string());
    }
}

/// The push of `sha` landed: its replies are due.
pub(super) fn pushed(run: &mut Run, n: u16, sha: &str) {
    let landed = |r: &ReplyDue| r.push.as_deref() == Some(sha);
    for r in stage_mut(run, n).replies.iter_mut().filter(|r| landed(r)) {
        r.ready = true;
    }
}

/// The remote stage branch was adopted: it descends from the stage head, so every
/// merge waiting for a push is on the remote already.
pub(super) fn adopted(run: &mut Run, n: u16) {
    stage_mut(run, n)
        .replies
        .iter_mut()
        .for_each(|r| r.ready = true);
}

/// Sends stage `n`'s first due reply, before anything else of the stage (so its id is
/// known before a view can show it); whether one went out.
pub(super) fn send(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) -> bool {
    let (Some(stage), Some(pr)) = (run.delivery.stage(n), run.delivery.pr(n)) else {
        return false;
    };
    let waiting = stage.retry_at.is_some_and(|t| t > now);
    let Some(i) = stage.replies.iter().position(|r| r.ready) else {
        return false;
    };
    if waiting || host_busy(run, n) {
        return false;
    }
    let r = stage.replies[i].clone();
    let op = HostOp::Reply {
        stage: n,
        number: pr.number,
        thread: format!("{}:{}", pr.number, r.thread),
        target: r.target,
        body: r.body,
        marker: r.marker,
    };
    let sent = emit(run, op, fx);
    if sent {
        stage_mut(run, n).replies[i].sent = true;
    }
    sent
}

/// A reply was posted (or found by its marker): the comment is anthrex's own, and the
/// thread it answered is `replied` (unless a new comment has made it `new` again).
pub(super) fn replied(run: &mut Run, n: u16, marker: &str, comment_id: u64, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let stage = stage_mut(run, n);
    stage.own_comments.insert(comment_id);
    let Some(i) = stage.replies.iter().position(|r| r.marker == marker) else {
        return;
    };
    let r = stage.replies.remove(i);
    if let Some(t) = stage.threads.iter_mut().find(|t| t.key == r.thread) {
        let answers = match (&t.state, &r.task) {
            (ThreadState::Tasked { task }, Some(id)) => task == id,
            (ThreadState::Replied { .. }, None) => true,
            _ => false,
        };
        if answers {
            t.state = ThreadState::Replied { comment_id };
        }
    }
    let text = format!("{}: replied on {}:{}", named(n, &pr), pr.number, r.thread);
    log(run, now, text);
}

/// A reply failed: one whose thread or PR is gone (`NotFound`) is dropped; any other
/// is retried when next due (decision 11).
pub(super) fn reply_failed(run: &mut Run, n: u16, marker: &str, error: &HostError, now: u64) {
    if !matches!(error, HostError::NotFound(_)) {
        return;
    }
    let stage = stage_mut(run, n);
    let Some(i) = stage.replies.iter().position(|r| r.marker == marker) else {
        return;
    };
    let r = stage.replies.remove(i);
    log(
        run,
        now,
        format!(
            "stage {n}: the reply on thread {} was dropped: not found",
            r.thread
        ),
    );
}
