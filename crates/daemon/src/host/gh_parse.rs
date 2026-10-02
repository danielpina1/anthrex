//! Reading `gh`'s and `git`'s output (decisions 6, 11 and 13; rulings R-2 to R-6, R-10,
//! R-11). Pure. Only the fields M9.2.1 recorded are read; a missing or unknown one is
//! `HostError::Rejected("gh output changed: <field>")`, never a guess (Risks, "`gh` JSON
//! drift").

use serde_json::Value;

use super::runner::RunOutput;
use super::{
    Author, CheckRun, CheckStatus, Conclusion, HostError, IssueComment, MergeCommit, Mergeable,
    PrState, PrView, PushOutcome, RepoPermission, Review, ReviewState, ReviewThread, ThreadComment,
    VIEW_ITEMS_MAX, VIEW_TEXT_MAX,
};

/// Decision 11 (with ruling R-6's logged-out text), from a command's stderr.
pub fn classify(stderr: &str) -> HostError {
    let text = last_line(stderr);
    let lower = stderr.to_ascii_lowercase();
    let has = |needle: &str| lower.contains(needle);
    if has("rate limit") {
        HostError::RateLimited(text)
    } else if has("not logged in")
        || has("not logged into any accounts")
        || has("gh auth login")
        || has("authentication")
        || has("http 401")
    {
        HostError::Auth(text)
    } else if has("could not resolve to a") || has("http 404") {
        HostError::NotFound(text)
    } else if has("[rejected]") || has("non-fast-forward") || has("fetch first") {
        HostError::Rejected(text)
    } else {
        HostError::Failed(text)
    }
}

/// The last non-empty line of `text`, trimmed and cut to 300 characters.
pub fn last_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("");
    cut_chars(line, 300)
}

fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

fn changed(field: &str) -> HostError {
    HostError::Rejected(format!("gh output changed: {field}"))
}

fn json(text: &str, what: &str) -> Result<Value, HostError> {
    serde_json::from_str(text).map_err(|_| changed(what))
}

fn get<'a>(value: &'a Value, field: &str) -> Result<&'a Value, HostError> {
    value.get(field).ok_or_else(|| changed(field))
}

fn str_of<'a>(value: &'a Value, field: &str) -> Result<&'a str, HostError> {
    get(value, field)?.as_str().ok_or_else(|| changed(field))
}

fn u64_of(value: &Value, field: &str) -> Result<u64, HostError> {
    get(value, field)?.as_u64().ok_or_else(|| changed(field))
}

fn array_of<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, HostError> {
    get(value, field)?.as_array().ok_or_else(|| changed(field))
}

fn nodes<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, HostError> {
    array_of(get(value, field)?, "nodes")
}

/// Ruling R-2: `fullDatabaseId` is a `BigInt` encoded as a string.
fn full_id(value: &Value) -> Result<u64, HostError> {
    str_of(value, "fullDatabaseId")?
        .parse()
        .map_err(|_| changed("fullDatabaseId"))
}

/// Ruling R-3: a bot is `__typename == "Bot"`. A deleted account has no author; GitHub
/// shows it as `ghost`.
fn author(value: &Value) -> Result<Author, HostError> {
    match get(value, "author")? {
        Value::Null => Ok(Author {
            login: "ghost".to_string(),
            bot: false,
        }),
        a => Ok(Author {
            login: str_of(a, "login")?.to_string(),
            bot: str_of(a, "__typename")? == "Bot",
        }),
    }
}

fn body(value: &Value, field: &str) -> Result<String, HostError> {
    Ok(cut_chars(str_of(value, field)?, VIEW_TEXT_MAX))
}

/// One poll: `gh pr view --json <PR_VIEW_FIELDS>` and the [`super::THREADS_QUERY`]
/// answer.
pub fn pr_view(number: u64, view: &str, threads: &str) -> Result<PrView, HostError> {
    let v = json(view, "pr view")?;
    let state = match str_of(&v, "state")? {
        "OPEN" => PrState::Open,
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => return Err(changed("state")),
    };
    let merged_at = match get(&v, "mergedAt")? {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(parse_time(s).ok_or_else(|| changed("mergedAt"))?),
        _ => return Err(changed("mergedAt")),
    };
    let merge_commit = match get(&v, "mergeCommit")? {
        Value::Null => None,
        c => Some(MergeCommit {
            oid: str_of(c, "oid")?.to_string(),
        }),
    };
    let mergeable = match str_of(&v, "mergeable")? {
        "MERGEABLE" => Mergeable::Mergeable,
        "CONFLICTING" => Mergeable::Conflicting,
        "UNKNOWN" => Mergeable::Unknown,
        _ => return Err(changed("mergeable")),
    };
    let review_decision = match get(&v, "reviewDecision")? {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(s.clone()),
        _ => return Err(changed("reviewDecision")),
    };
    let checks = array_of(&v, "statusCheckRollup")?
        .iter()
        .map(check)
        .collect::<Result<Vec<_>, _>>()?;

    let t = json(threads, "graphql")?;
    let pr = get(get(get(&t, "data")?, "repository")?, "pullRequest")?;
    let mut reviews = nodes(pr, "reviews")?
        .iter()
        .map(|r| {
            Ok(Review {
                id: full_id(r)?,
                author: author(r)?,
                state: review_state(str_of(r, "state")?)?,
                body: body(r, "body")?,
            })
        })
        .collect::<Result<Vec<_>, HostError>>()?;
    let mut comments = nodes(pr, "comments")?
        .iter()
        .map(|c| {
            Ok(IssueComment {
                id: full_id(c)?,
                author: author(c)?,
                body: body(c, "body")?,
            })
        })
        .collect::<Result<Vec<_>, HostError>>()?;
    let mut review_threads = nodes(pr, "reviewThreads")?
        .iter()
        .map(thread)
        .collect::<Result<Vec<_>, _>>()?;
    reviews.sort_by_key(|r| std::cmp::Reverse(r.id));
    reviews.truncate(VIEW_ITEMS_MAX);
    comments.sort_by_key(|c| std::cmp::Reverse(c.id));
    comments.truncate(VIEW_ITEMS_MAX);
    let first = |t: &ReviewThread| t.comments.first().map_or(0, |c| c.id);
    review_threads.sort_by_key(|t| std::cmp::Reverse(first(t)));
    review_threads.truncate(VIEW_ITEMS_MAX);
    Ok(PrView {
        number,
        state,
        merged_at,
        merge_commit,
        base_ref: str_of(&v, "baseRefName")?.to_string(),
        head_oid: str_of(&v, "headRefOid")?.to_string(),
        mergeable,
        review_decision,
        checks,
        reviews,
        comments,
        threads: review_threads,
    })
}

fn review_state(state: &str) -> Result<ReviewState, HostError> {
    Ok(match state {
        "APPROVED" => ReviewState::Approved,
        "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
        "COMMENTED" => ReviewState::Commented,
        "DISMISSED" => ReviewState::Dismissed,
        "PENDING" => ReviewState::Pending,
        _ => return Err(changed("reviews.state")),
    })
}

fn thread(value: &Value) -> Result<ReviewThread, HostError> {
    let resolved = get(value, "isResolved")?
        .as_bool()
        .ok_or_else(|| changed("isResolved"))?;
    let path = match get(value, "path")? {
        Value::Null => None,
        p => Some(p.as_str().ok_or_else(|| changed("path"))?.to_string()),
    };
    let line = match get(value, "line")? {
        Value::Null => None,
        l => Some(
            l.as_u64()
                .and_then(|l| u32::try_from(l).ok())
                .ok_or_else(|| changed("line"))?,
        ),
    };
    let comments = nodes(value, "comments")?
        .iter()
        .map(|c| {
            Ok(ThreadComment {
                id: full_id(c)?,
                author: author(c)?,
                body: body(c, "body")?,
                diff_hunk: body(c, "diffHunk")?,
            })
        })
        .collect::<Result<Vec<_>, HostError>>()?;
    Ok(ReviewThread {
        resolved,
        path,
        line,
        comments,
    })
}

/// One `statusCheckRollup` entry: a `CheckRun` or a `StatusContext` (M9.2.1 check 2).
fn check(value: &Value) -> Result<CheckRun, HostError> {
    let (name, url, status, conclusion) = match str_of(value, "__typename")? {
        "CheckRun" => {
            let conclusion = match get(value, "conclusion")? {
                Value::String(c) => conclusion(c),
                _ => None,
            };
            let completed = str_of(value, "status")? == "COMPLETED" && conclusion.is_some();
            (
                str_of(value, "name")?,
                str_of(value, "detailsUrl")?,
                completed,
                conclusion,
            )
        }
        "StatusContext" => {
            let conclusion = match str_of(value, "state")? {
                "SUCCESS" => Some(Conclusion::Success),
                "FAILURE" => Some(Conclusion::Failure),
                "ERROR" => Some(Conclusion::Error),
                _ => None,
            };
            let url = match get(value, "targetUrl")? {
                Value::Null => "",
                u => u.as_str().ok_or_else(|| changed("targetUrl"))?,
            };
            (
                str_of(value, "context")?,
                url,
                conclusion.is_some(),
                conclusion,
            )
        }
        _ => return Err(changed("statusCheckRollup.__typename")),
    };
    Ok(CheckRun {
        name: name.to_string(),
        status: if status {
            CheckStatus::Completed
        } else {
            CheckStatus::Pending
        },
        conclusion: if status { conclusion } else { None },
        ci_run: actions_run_id(url),
        url: url.to_string(),
    })
}

/// A completed check run's conclusion; one anthrex does not know is `None` (pending,
/// ruling R-5).
fn conclusion(text: &str) -> Option<Conclusion> {
    Some(match text {
        "SUCCESS" => Conclusion::Success,
        "FAILURE" => Conclusion::Failure,
        "TIMED_OUT" => Conclusion::TimedOut,
        "CANCELLED" => Conclusion::Cancelled,
        "ACTION_REQUIRED" => Conclusion::ActionRequired,
        "STARTUP_FAILURE" => Conclusion::StartupFailure,
        "NEUTRAL" => Conclusion::Neutral,
        "SKIPPED" => Conclusion::Skipped,
        "STALE" => Conclusion::Stale,
        _ => return None,
    })
}

/// Decision 27: the GitHub Actions run of a check, from `/actions/runs/<id>` in its URL.
pub fn actions_run_id(url: &str) -> Option<u64> {
    let (_, rest) = url.split_once("/actions/runs/")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `YYYY-MM-DDTHH:MM:SS[.fff]Z` as Unix seconds.
pub fn parse_time(text: &str) -> Option<u64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.split('.').next()?;
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || min > 59 || s > 60 {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + min * 60 + s).ok()
}

/// One row of `gh pr list --json <PR_LIST_FIELDS>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedPr {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub base: String,
    pub owner: String,
    pub cross_repository: bool,
}

pub fn pr_list(text: &str) -> Result<Vec<ListedPr>, HostError> {
    let rows = json(text, "pr list")?;
    let rows = rows.as_array().ok_or_else(|| changed("pr list"))?;
    rows.iter()
        .map(|row| {
            let state = match str_of(row, "state")? {
                "OPEN" => PrState::Open,
                "MERGED" => PrState::Merged,
                "CLOSED" => PrState::Closed,
                _ => return Err(changed("state")),
            };
            Ok(ListedPr {
                number: u64_of(row, "number")?,
                url: str_of(row, "url")?.to_string(),
                state,
                base: str_of(row, "baseRefName")?.to_string(),
                owner: str_of(get(row, "headRepositoryOwner")?, "login")?.to_string(),
                cross_repository: get(row, "isCrossRepository")?
                    .as_bool()
                    .ok_or_else(|| changed("isCrossRepository"))?,
            })
        })
        .collect()
}

/// `https://<host>/<o>/<r>/pull/<n>` → `n` (`gh pr create` prints it).
pub fn pr_url_number(url: &str) -> Option<u64> {
    let (_, n) = url.trim().rsplit_once("/pull/")?;
    n.parse().ok()
}

/// `…#issuecomment-<id>` → `id` (`gh pr comment` prints it).
pub fn comment_url_id(url: &str) -> Option<u64> {
    let (_, id) = url.trim().rsplit_once("#issuecomment-")?;
    id.parse().ok()
}

/// Decision 10: the id of a comment by `author` (anthrex's own login; GitHub's logins
/// match without case) whose body holds `marker`, in `gh api --paginate`'s output (one
/// JSON array per page, printed one after another). A marker in anyone else's comment
/// was pasted, and does not stop the reply (task M9.2.10's fix round).
pub fn find_marker(text: &str, marker: &str, author: &str) -> Result<Option<u64>, HostError> {
    for page in serde_json::Deserializer::from_str(text).into_iter::<Vec<Value>>() {
        let page = page.map_err(|_| changed("comments"))?;
        for comment in &page {
            let login = str_of(get(comment, "user")?, "login")?;
            if login.eq_ignore_ascii_case(author) && str_of(comment, "body")?.contains(marker) {
                return u64_of(comment, "id").map(Some);
            }
        }
    }
    Ok(None)
}

/// `gh api user`'s `login`: the user anthrex posts as.
pub fn user_login(text: &str) -> Result<String, HostError> {
    Ok(str_of(&json(text, "user")?, "login")?.to_string())
}

/// The `id` of the comment a REST `POST` created.
pub fn created_comment_id(text: &str) -> Result<u64, HostError> {
    u64_of(&json(text, "comment")?, "id")
}

/// Ruling R-6: `role_name`; a custom role, or none, is no write access.
pub fn permission(text: &str) -> Result<RepoPermission, HostError> {
    Ok(match str_of(&json(text, "permission")?, "role_name")? {
        "admin" => RepoPermission::Admin,
        "maintain" => RepoPermission::Maintain,
        "write" => RepoPermission::Write,
        "triage" => RepoPermission::Triage,
        "read" => RepoPermission::Read,
        _ => RepoPermission::None,
    })
}

/// `gh version 2.92.0 (2026-04-28)` → `(2, 92, 0)`.
pub fn gh_version(printed: &str) -> Option<(u32, u32, u32)> {
    let first = printed.lines().next()?;
    let rest = first.strip_prefix("gh version ")?;
    let version = rest.split_whitespace().next()?;
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Decision 13 and ruling R-11, from `git push --porcelain`'s status line for `dst`
/// (`<flag>\t<src>:<dst>\t<summary>`; M9.2.1 check 3).
pub fn push_outcome(out: &RunOutput, dst: &str) -> Result<PushOutcome, HostError> {
    let stdout = out.stdout_text();
    let line = stdout.lines().find_map(|line| {
        let mut fields = line.splitn(3, '\t');
        let flag = fields.next()?;
        let spec = fields.next()?;
        let summary = fields.next().unwrap_or("");
        let (_, to) = spec.split_once(':')?;
        (to == dst).then(|| (flag.to_string(), summary.to_string()))
    });
    let branch = dst.strip_prefix("refs/heads/").unwrap_or(dst);
    let Some((flag, summary)) = line else {
        return Err(if out.success {
            HostError::Failed(format!("git push printed no status for {branch}"))
        } else {
            classify(&out.stderr)
        });
    };
    let reason = || {
        summary
            .split_once('(')
            .and_then(|(_, r)| r.strip_suffix(')'))
            .unwrap_or(summary.as_str())
            .to_string()
    };
    match flag.as_str() {
        "*" | " " | "-" => Ok(PushOutcome::Pushed),
        "=" => Ok(PushOutcome::UpToDate),
        "!" if summary.starts_with("[remote rejected]") => Ok(PushOutcome::Refused {
            reason: format!("the remote refused the push of {branch}: {}", reason()),
        }),
        "!" if summary.starts_with("[rejected]") => Ok(PushOutcome::Rejected { reason: reason() }),
        // Ruling I2: any other refusal (`[remote failure]`, …) says nothing about the
        // remote branch; it is an error, retried when next due.
        "!" => Err(HostError::Failed(format!(
            "git push of {branch} failed: {summary}"
        ))),
        _ => Err(HostError::Failed(format!(
            "git push reported {flag:?} for {branch}, which anthrex never asks for"
        ))),
    }
}
