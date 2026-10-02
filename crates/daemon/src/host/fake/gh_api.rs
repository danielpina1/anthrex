//! `FakeGh`'s `gh api` calls: the one GraphQL query (`THREADS_QUERY`, ruling R-2), the
//! two comment listings (`--paginate`: one JSON array per page of 30, printed one after
//! another, as gh does without `--slurp`), the thread reply `POST` and the permission
//! read. Parsed strictly; GitHub's REST and GraphQL shapes as M9.2.1 recorded them.

use serde_json::{Value, json};

use super::gh_pr::{no_pr, no_repo, pr_url};
use super::github::{FAKE_LOGIN, FakeGithub, FakeReply, FakeRepo, FakeThreadComment};
use super::{Answer, Cmd};
use crate::host::{RepoPermission, ReviewState, THREADS_QUERY};

/// GitHub's REST page size, which `--paginate` walks.
const PAGE: usize = 30;

pub(super) fn parse(args: &[&str]) -> Option<Cmd> {
    let ["api", rest @ ..] = args else {
        return None;
    };
    let mut method = None;
    let mut paginate = false;
    let mut fields: Vec<(&str, &str)> = Vec::new();
    let mut paths = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "-X" | "--method" => {
                i += 1;
                method.replace(*rest.get(i)?).is_none().then_some(())?;
            }
            "-f" | "--raw-field" | "-F" | "--field" => {
                i += 1;
                fields.push(rest.get(i)?.split_once('=')?);
            }
            "--paginate" if !paginate => paginate = true,
            arg if !arg.starts_with('-') => paths.push(arg),
            _ => return None,
        }
        i += 1;
    }
    let [path] = paths[..] else { return None };
    // gh's default method: GET, or POST once a field is given.
    let method = method.unwrap_or(if fields.is_empty() { "GET" } else { "POST" });
    let field = |key: &str| {
        let mut found = fields.iter().filter(|(k, _)| *k == key);
        let value = found.next()?.1;
        found.next().is_none().then(|| value.to_string())
    };
    if path == "graphql" {
        (fields.len() == 4 && !paginate && field("query")? == THREADS_QUERY).then_some(())?;
        return Some(Cmd::Threads {
            owner: field("owner")?,
            name: field("name")?,
            number: field("number")?.parse().ok()?,
        });
    }
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let repo = |o: &str, r: &str| format!("{o}/{r}");
    match (method, &segments[..]) {
        ("GET", ["repos", o, r, kind @ ("pulls" | "issues"), n, "comments"])
            if paginate && fields.is_empty() =>
        {
            Some(Cmd::ListComments {
                repo: repo(o, r),
                number: n.parse().ok()?,
                review: *kind == "pulls",
            })
        }
        ("POST", ["repos", o, r, "pulls", n, "comments", id, "replies"])
            if !paginate && fields.len() == 1 =>
        {
            Some(Cmd::Reply {
                repo: repo(o, r),
                number: n.parse().ok()?,
                comment: id.parse().ok()?,
                body: field("body")?,
            })
        }
        ("GET", ["repos", o, r, "collaborators", user, "permission"])
            if !paginate && fields.is_empty() =>
        {
            Some(Cmd::Permission {
                repo: repo(o, r),
                user: user.to_string(),
            })
        }
        _ => None,
    }
}

/// REST's 404, as gh prints it (M9.2.1 check 3's shape).
fn not_found() -> Answer {
    Answer {
        ok: false,
        stdout: r#"{"message":"Not Found","documentation_url":"https://docs.github.com/rest","status":"404"}"#
            .to_string(),
        stderr: "gh: Not Found (HTTP 404)\n".to_string(),
    }
}

fn is_bot(user: &str) -> bool {
    user.ends_with("[bot]")
}

/// REST shows a bot's login with `[bot]` and `type: Bot`.
fn rest_user(user: &str) -> Value {
    json!({ "login": user, "type": if is_bot(user) { "Bot" } else { "User" } })
}

/// GraphQL shows a bot without `[bot]`, typed `Bot` (M9.2.1 check 2, ruling R-3).
fn author(user: &str) -> Value {
    json!({
        "__typename": if is_bot(user) { "Bot" } else { "User" },
        "login": user.strip_suffix("[bot]").unwrap_or(user),
    })
}

fn review_state(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Approved => "APPROVED",
        ReviewState::ChangesRequested => "CHANGES_REQUESTED",
        ReviewState::Commented => "COMMENTED",
        ReviewState::Dismissed => "DISMISSED",
        ReviewState::Pending => "PENDING",
    }
}

/// `[..][..]`: one array per page.
fn pages(items: Vec<Value>) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    items
        .chunks(PAGE)
        .map(|page| Value::Array(page.to_vec()).to_string())
        .collect()
}

/// The last `n` of `items`, in their order (GraphQL's `last: n`).
fn last<T>(items: &[T], n: usize) -> &[T] {
    &items[items.len().saturating_sub(n)..]
}

pub(super) fn answer(state: &mut FakeGithub, cmd: &Cmd, host: &str) -> Result<Answer, String> {
    let named = match cmd {
        Cmd::Threads { owner, name, .. } => format!("{owner}/{name}"),
        Cmd::ListComments { repo, .. } | Cmd::Reply { repo, .. } | Cmd::Permission { repo, .. } => {
            repo.clone()
        }
        other => return Err(format!("FakeGh: {other:?} is not an api call")),
    };
    let Some(repo) = state.repo.clone().filter(|_| state.is_repo(&named)) else {
        return Ok(match cmd {
            Cmd::Threads { .. } => no_repo(&named),
            _ => not_found(),
        });
    };
    match cmd {
        Cmd::Threads { number, .. } => Ok(match threads(state, *number) {
            Some(v) => Answer::out(v.to_string() + "\n"),
            None => no_pr(*number),
        }),
        Cmd::ListComments { number, review, .. } => {
            Ok(match listing(state, &repo, host, *number, *review) {
                Some(text) => Answer::out(text),
                None => not_found(),
            })
        }
        Cmd::Reply {
            number,
            comment,
            body,
            ..
        } => {
            let id = state.next_id();
            let url = state.pr(*number).map(|_| pr_url(host, &repo, *number));
            let Some((pr, url)) = state.pr_mut(*number).zip(url) else {
                return Ok(not_found());
            };
            // Replies to replies are not supported (M9.2.1 check 4): only a thread's
            // first comment takes one.
            let Some(thread) = pr
                .threads
                .iter_mut()
                .find(|t| t.comments.first().is_some_and(|c| c.id == *comment))
            else {
                return Ok(not_found());
            };
            let diff_hunk = thread.comments[0].diff_hunk.clone();
            thread.comments.push(FakeThreadComment {
                id,
                user: FAKE_LOGIN.to_string(),
                body: body.clone(),
                diff_hunk: diff_hunk.clone(),
            });
            let created = json!({
                "id": id,
                "in_reply_to_id": comment,
                "path": thread.path,
                "line": thread.line,
                "diff_hunk": diff_hunk,
                "body": body,
                "user": rest_user(FAKE_LOGIN),
                "html_url": format!("{url}#discussion_r{id}"),
            });
            pr.replies.push(FakeReply {
                id,
                thread: Some(*comment),
                body: body.clone(),
            });
            Ok(Answer::out(created.to_string() + "\n"))
        }
        Cmd::Permission { user, .. } => {
            let role = state.permissions.get(user).copied();
            let (permission, role_name) = match role {
                Some(RepoPermission::Admin) => ("admin", "admin"),
                Some(RepoPermission::Maintain) => ("write", "maintain"),
                Some(RepoPermission::Write) => ("write", "write"),
                Some(RepoPermission::Triage) => ("read", "triage"),
                Some(RepoPermission::Read) => ("read", "read"),
                Some(RepoPermission::None) | None => return Ok(not_found()),
            };
            Ok(Answer::out(
                json!({
                    "permission": permission,
                    "role_name": role_name,
                    "user": rest_user(user),
                })
                .to_string()
                    + "\n",
            ))
        }
        other => Err(format!("FakeGh: {other:?} is not an api call")),
    }
}

/// `THREADS_QUERY`'s answer: `reviewThreads(first: 100)` with `comments(first: 50)`,
/// `reviews(last: 100)`, `comments(last: 100)`; ids as `fullDatabaseId` strings.
fn threads(state: &FakeGithub, number: u64) -> Option<Value> {
    let pr = state.pr(number)?;
    let threads: Vec<Value> = pr
        .threads
        .iter()
        .take(100)
        .map(|t| {
            let comments: Vec<Value> = t
                .comments
                .iter()
                .take(50)
                .map(|c| {
                    json!({
                        "fullDatabaseId": c.id.to_string(),
                        "body": c.body,
                        "diffHunk": c.diff_hunk,
                        "author": author(&c.user),
                    })
                })
                .collect();
            json!({
                "isResolved": t.resolved,
                "path": t.path,
                "line": t.line,
                "comments": { "nodes": comments },
            })
        })
        .collect();
    let reviews: Vec<Value> = last(&pr.reviews, 100)
        .iter()
        .map(|r| {
            json!({
                "fullDatabaseId": r.id.to_string(),
                "state": review_state(r.state),
                "body": r.body,
                "author": author(&r.user),
            })
        })
        .collect();
    let comments: Vec<Value> = last(&pr.comments, 100)
        .iter()
        .map(|c| {
            json!({
                "fullDatabaseId": c.id.to_string(),
                "body": c.body,
                "author": author(&c.user),
            })
        })
        .collect();
    Some(json!({ "data": { "repository": { "pullRequest": {
        "reviewThreads": { "nodes": threads },
        "reviews": { "nodes": reviews },
        "comments": { "nodes": comments },
    } } } }))
}

/// `repos/<o>/<r>/pulls/<n>/comments` (every thread comment, `in_reply_to_id` on all
/// but a thread's first) or `…/issues/<n>/comments`, oldest first.
fn listing(
    state: &FakeGithub,
    repo: &FakeRepo,
    host: &str,
    number: u64,
    review: bool,
) -> Option<String> {
    let pr = state.pr(number)?;
    let url = pr_url(host, repo, number);
    let mut items: Vec<(u64, Value)> = Vec::new();
    if review {
        for t in &pr.threads {
            let first = t.comments.first().map(|c| c.id);
            for c in &t.comments {
                let mut row = json!({
                    "id": c.id,
                    "path": t.path,
                    "line": t.line,
                    "diff_hunk": c.diff_hunk,
                    "body": c.body,
                    "user": rest_user(&c.user),
                    "html_url": format!("{url}#discussion_r{}", c.id),
                });
                if Some(c.id) != first {
                    row["in_reply_to_id"] = json!(first);
                }
                items.push((c.id, row));
            }
        }
    } else {
        for c in &pr.comments {
            let row = json!({
                "id": c.id,
                "html_url": format!("{url}#issuecomment-{}", c.id),
                "user": rest_user(&c.user),
                "body": c.body,
            });
            items.push((c.id, row));
        }
    }
    items.sort_by_key(|(id, _)| *id);
    Some(pages(items.into_iter().map(|(_, v)| v).collect()))
}
