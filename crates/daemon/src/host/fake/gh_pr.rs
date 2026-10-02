//! `FakeGh`'s `gh auth`, `gh repo`, `gh pr` and `gh run` commands: parsed strictly
//! (only the flags `GhHost` builds; anything else is "unsupported") and answered in the
//! shapes M9.2.1 recorded from gh 2.92.0 (`host/fixtures/`).

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::github::{Bare, FAKE_LOGIN, FakeComment, FakeGithub, FakePr, FakeReply, FakeRepo, iso};
use super::rules;
use super::{Answer, Cmd};
use crate::host::{Conclusion, PrState, ReviewState};

/// The `--json` fields `FakeGh` knows for `gh pr view` and `gh pr list`.
const PR_FIELDS: [&str; 15] = [
    "number",
    "url",
    "state",
    "title",
    "body",
    "baseRefName",
    "headRefName",
    "headRefOid",
    "mergedAt",
    "mergeCommit",
    "mergeable",
    "reviewDecision",
    "statusCheckRollup",
    "headRepositoryOwner",
    "isCrossRepository",
];

/// Positional arguments and `--flag value` pairs (a flag in `bools` takes no value and
/// maps to `""`); `None` on any other flag, a flag given twice, or a missing value.
pub(super) fn flags<'a>(
    args: &[&'a str],
    values: &[&str],
    bools: &[&str],
) -> Option<(Vec<&'a str>, BTreeMap<&'a str, &'a str>)> {
    let mut positional = Vec::new();
    let mut set = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if !arg.starts_with('-') {
            positional.push(arg);
        } else if bools.contains(&arg) {
            set.insert(arg, "").is_none().then_some(())?;
        } else if values.contains(&arg) {
            i += 1;
            set.insert(arg, *args.get(i)?).is_none().then_some(())?;
        } else {
            return None;
        }
        i += 1;
    }
    Some((positional, set))
}

fn number(text: &str) -> Option<u64> {
    (!text.starts_with('+'))
        .then(|| text.parse().ok())
        .flatten()
}

fn pr_fields(list: &str) -> Option<Vec<String>> {
    let fields: Vec<String> = list.split(',').map(str::to_string).collect();
    fields
        .iter()
        .all(|f| PR_FIELDS.contains(&f.as_str()))
        .then_some(fields)
}

pub(super) fn parse(args: &[&str]) -> Option<Cmd> {
    match args {
        ["--version"] => Some(Cmd::Version),
        ["auth", "status", rest @ ..] => {
            let (pos, f) = flags(rest, &["--hostname"], &[])?;
            pos.is_empty().then_some(())?;
            Some(Cmd::AuthStatus {
                host: f.get("--hostname")?.to_string(),
            })
        }
        ["repo", "view", rest @ ..] => {
            let (pos, f) = flags(rest, &["--json"], &[])?;
            let [repo] = pos[..] else { return None };
            (f.get("--json") == Some(&"nameWithOwner")).then_some(())?;
            Some(Cmd::RepoView {
                repo: repo.to_string(),
            })
        }
        ["pr", sub, rest @ ..] => pr(sub, rest),
        ["run", sub @ ("view" | "rerun"), rest @ ..] => {
            let wanted = if *sub == "view" {
                "--log-failed"
            } else {
                "--failed"
            };
            let (pos, f) = flags(rest, &["--repo"], &[wanted])?;
            let [id] = pos[..] else { return None };
            f.contains_key(wanted).then_some(())?;
            let (id, repo) = (number(id)?, f.get("--repo")?.to_string());
            Some(if *sub == "view" {
                Cmd::RunView { id, repo }
            } else {
                Cmd::RunRerun { id, repo }
            })
        }
        _ => None,
    }
}

fn pr(sub: &str, rest: &[&str]) -> Option<Cmd> {
    let get = |f: &BTreeMap<&str, &str>, k: &str| f.get(k).map(|v| v.to_string());
    match sub {
        "list" => {
            let (pos, f) = flags(rest, &["--repo", "--head", "--state", "--json"], &[])?;
            pos.is_empty().then_some(())?;
            let state = get(&f, "--state").unwrap_or_else(|| "open".to_string());
            ["all", "open", "closed", "merged"]
                .contains(&state.as_str())
                .then_some(())?;
            Some(Cmd::PrList {
                repo: get(&f, "--repo")?,
                head: get(&f, "--head")?,
                state,
                fields: pr_fields(f.get("--json")?)?,
            })
        }
        "create" => {
            let names = ["--repo", "--base", "--head", "--title", "--body-file"];
            let (pos, f) = flags(rest, &names, &[])?;
            (pos.is_empty() && f.len() == names.len()).then_some(())?;
            Some(Cmd::PrCreate {
                repo: get(&f, "--repo")?,
                base: get(&f, "--base")?,
                head: get(&f, "--head")?,
                title: get(&f, "--title")?,
                body_file: get(&f, "--body-file")?,
            })
        }
        "view" | "comment" | "edit" => {
            let second = match sub {
                "view" => "--json",
                "comment" => "--body-file",
                _ => "--base",
            };
            let (pos, f) = flags(rest, &["--repo", second], &[])?;
            let [n] = pos[..] else { return None };
            let (number, repo, value) = (number(n)?, get(&f, "--repo")?, get(&f, second)?);
            Some(match sub {
                "view" => Cmd::PrView {
                    number,
                    repo,
                    fields: pr_fields(&value)?,
                },
                "comment" => Cmd::PrComment {
                    number,
                    repo,
                    body_file: value,
                },
                _ => Cmd::PrEdit {
                    number,
                    repo,
                    base: value,
                },
            })
        }
        _ => None,
    }
}

/// GitHub's answer when `--repo` names another repository.
pub(super) fn no_repo(repo: &str) -> Answer {
    Answer::fail(format!(
        "GraphQL: Could not resolve to a Repository with the name '{repo}'. (repository)\n"
    ))
}

pub(super) fn no_pr(number: u64) -> Answer {
    Answer::fail(format!(
        "GraphQL: Could not resolve to a PullRequest with the number of {number}. (repository.pullRequest)\n"
    ))
}

pub(super) fn pr_url(host: &str, repo: &FakeRepo, number: u64) -> String {
    format!("https://{host}/{}/{}/pull/{number}", repo.owner, repo.name)
}

pub(super) fn answer(state: &mut FakeGithub, cmd: &Cmd, host: &str) -> Result<Answer, String> {
    let repo_named = match cmd {
        Cmd::AuthStatus { host } => {
            return Ok(Answer::out(format!(
                "{host}\n  ✓ Logged in to {host} account {FAKE_LOGIN} (keyring)\n  - Active account: true\n  - Git operations protocol: https\n"
            )));
        }
        Cmd::RepoView { repo }
        | Cmd::PrList { repo, .. }
        | Cmd::PrCreate { repo, .. }
        | Cmd::PrView { repo, .. }
        | Cmd::PrComment { repo, .. }
        | Cmd::PrEdit { repo, .. }
        | Cmd::RunView { repo, .. }
        | Cmd::RunRerun { repo, .. } => repo,
        other => return Err(format!("FakeGh: {other:?} is not a pr command")),
    };
    let Some(repo) = state.repo.clone().filter(|_| state.is_repo(repo_named)) else {
        return Ok(match cmd {
            Cmd::RunView { id, .. } | Cmd::RunRerun { id, .. } => Answer::fail(format!(
                "failed to get run: HTTP 404: Not Found (actions/runs/{id})\n"
            )),
            _ => no_repo(repo_named),
        });
    };
    let bare_dir = repo.bare.clone();
    let bare = Bare::new(&bare_dir);
    match cmd {
        Cmd::RepoView { .. } => Ok(Answer::out(
            json!({ "nameWithOwner": format!("{}/{}", repo.owner, repo.name) }).to_string() + "\n",
        )),
        Cmd::PrList {
            head,
            state: wanted,
            fields,
            ..
        } => {
            let mut numbers: Vec<u64> = state
                .prs
                .iter()
                .filter(|p| p.head == *head && state_matches(p.state, wanted))
                .map(|p| p.number)
                .collect();
            numbers.sort_unstable_by(|a, b| b.cmp(a));
            let rows = numbers
                .into_iter()
                .map(|n| pr_json(state, &bare, &repo, n, host, fields))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Answer::out(Value::Array(rows).to_string() + "\n"))
        }
        Cmd::PrCreate {
            base,
            head,
            title,
            body_file,
            ..
        } => create(state, &bare, &repo, host, base, head, title, body_file),
        Cmd::PrView { number, fields, .. } => match state.pr(*number) {
            None => Ok(no_pr(*number)),
            Some(_) => {
                let view = pr_json(state, &bare, &repo, *number, host, fields)?;
                Ok(Answer::out(view.to_string() + "\n"))
            }
        },
        Cmd::PrComment {
            number, body_file, ..
        } => {
            let Ok(body) = std::fs::read_to_string(body_file) else {
                return Ok(Answer::fail(format!(
                    "open {body_file}: no such file or directory\n"
                )));
            };
            let id = state.next_id();
            let Some(pr) = state.pr_mut(*number) else {
                return Ok(no_pr(*number));
            };
            pr.comments.push(FakeComment {
                id,
                user: FAKE_LOGIN.to_string(),
                body: body.clone(),
            });
            pr.replies.push(FakeReply {
                id,
                thread: None,
                body,
            });
            let url = pr_url(host, &repo, *number);
            Ok(Answer::out(format!("{url}#issuecomment-{id}\n")))
        }
        Cmd::PrEdit { number, base, .. } => {
            let heads = bare.heads()?;
            let Some(pr) = state.pr_mut(*number) else {
                return Ok(no_pr(*number));
            };
            let url = pr_url(host, &repo, *number);
            if pr.base == *base {
                return Ok(Answer::out(url + "\n"));
            }
            // Not observed (M9.2.1 had no write call): GitHub's GraphQL texts, as
            // constructed for the manual check to confirm.
            if !heads.contains_key(base) {
                return Ok(Answer::fail(format!(
                    "GraphQL: Could not resolve to a Ref with the name 'refs/heads/{base}'. (updatePullRequest)\n"
                )));
            }
            if pr.state != PrState::Open {
                return Ok(Answer::fail(
                    "GraphQL: Cannot change the base branch of a closed pull request. (updatePullRequest)\n"
                        .to_string(),
                ));
            }
            pr.base = base.clone();
            Ok(Answer::out(url + "\n"))
        }
        Cmd::RunView { id, .. } => Ok(match rules::failed_log(state, *id) {
            Ok(log) => Answer::out(log),
            Err(e) => Answer::fail(e + "\n"),
        }),
        Cmd::RunRerun { id, .. } => Ok(match rules::rerun(state, &bare, *id)? {
            Ok(()) => Answer::out(String::new()),
            Err(e) => Answer::fail(e + "\n"),
        }),
        other => Err(format!("FakeGh: {other:?} is not a pr command")),
    }
}

fn state_matches(state: PrState, wanted: &str) -> bool {
    match wanted {
        "all" => true,
        "open" => state == PrState::Open,
        "closed" => state == PrState::Closed,
        _ => state == PrState::Merged,
    }
}

#[allow(clippy::too_many_arguments)]
fn create(
    state: &mut FakeGithub,
    bare: &Bare<'_>,
    repo: &FakeRepo,
    host: &str,
    base: &str,
    head: &str,
    title: &str,
    body_file: &str,
) -> Result<Answer, String> {
    let Ok(body) = std::fs::read_to_string(body_file) else {
        return Ok(Answer::fail(format!(
            "open {body_file}: no such file or directory\n"
        )));
    };
    let heads = bare.heads()?;
    let (Some(base_oid), Some(head_oid)) = (heads.get(base), heads.get(head)) else {
        return Ok(Answer::fail(format!(
            "pull request create failed: GraphQL: Head sha can't be blank, Base sha can't be blank, No commits between {base} and {head}, Head ref must be a branch (createPullRequest)\n"
        )));
    };
    if let Some(open) = state
        .prs
        .iter()
        .find(|p| p.head == head && p.base == base && p.state == PrState::Open)
    {
        return Ok(Answer::fail(format!(
            "a pull request for branch \"{head}\" into branch \"{base}\" already exists:\n{}\n",
            pr_url(host, repo, open.number)
        )));
    }
    if bare.ok(&["rev-list", "--count", &format!("{base_oid}..{head_oid}")])? == "0" {
        return Ok(Answer::fail(format!(
            "pull request create failed: GraphQL: No commits between {base} and {head} (createPullRequest)\n"
        )));
    }
    state.last_pr += 1;
    let number = state.last_pr;
    state.prs.push(FakePr {
        number,
        base: base.to_string(),
        head: head.to_string(),
        head_oid: head_oid.clone(),
        title: title.to_string(),
        body,
        state: PrState::Open,
        merged_at: None,
        merge_commit: None,
        comments: Vec::new(),
        reviews: Vec::new(),
        threads: Vec::new(),
        replies: Vec::new(),
    });
    Ok(Answer {
        ok: true,
        stdout: pr_url(host, repo, number) + "\n",
        stderr: format!(
            "\nCreating pull request for {head} into {base} in {}/{}\n\n",
            repo.owner, repo.name
        ),
    })
}

/// The requested `--json` fields of PR `number`, in gh's shapes.
fn pr_json(
    state: &mut FakeGithub,
    bare: &Bare<'_>,
    repo: &FakeRepo,
    number: u64,
    host: &str,
    fields: &[String],
) -> Result<Value, String> {
    let pr = state.pr(number).cloned().ok_or("no such pull request")?;
    let mut obj = Map::new();
    for field in fields {
        let value = match field.as_str() {
            "number" => json!(pr.number),
            "url" => json!(pr_url(host, repo, pr.number)),
            "state" => json!(match pr.state {
                PrState::Open => "OPEN",
                PrState::Merged => "MERGED",
                PrState::Closed => "CLOSED",
            }),
            "title" => json!(pr.title),
            "body" => json!(pr.body),
            "baseRefName" => json!(pr.base),
            "headRefName" => json!(pr.head),
            "headRefOid" => json!(pr.head_oid),
            "mergedAt" => pr.merged_at.map_or(Value::Null, |t| json!(iso(t))),
            "mergeCommit" => pr
                .merge_commit
                .as_ref()
                .map_or(Value::Null, |oid| json!({ "oid": oid })),
            "mergeable" => json!(mergeable(bare, &pr)?),
            "reviewDecision" => json!(review_decision(&pr)),
            "statusCheckRollup" => checks(state, bare, repo, &pr, host)?,
            "headRepositoryOwner" => json!({ "id": "O_fake", "login": repo.owner }),
            "isCrossRepository" => json!(false),
            other => return Err(format!("FakeGh: field {other}")),
        };
        obj.insert(field.clone(), value);
    }
    Ok(Value::Object(obj))
}

/// `MERGEABLE` when the head merges into the base branch without a conflict
/// (`git merge-tree`), `CONFLICTING` when not; a PR that is not open reads `UNKNOWN`,
/// as M9.2.1's merged fixture does.
fn mergeable(bare: &Bare<'_>, pr: &FakePr) -> Result<&'static str, String> {
    if pr.state != PrState::Open {
        return Ok("UNKNOWN");
    }
    let Some(base) = bare.branch(&pr.base)? else {
        return Ok("UNKNOWN");
    };
    Ok(match bare.merge_tree(&base, &pr.head_oid, None)? {
        Some(_) => "MERGEABLE",
        None => "CONFLICTING",
    })
}

/// Each reviewer's latest approving or change-requesting review decides; none is `""`.
fn review_decision(pr: &FakePr) -> &'static str {
    let mut latest: BTreeMap<&str, ReviewState> = BTreeMap::new();
    for r in &pr.reviews {
        if matches!(
            r.state,
            ReviewState::Approved | ReviewState::ChangesRequested | ReviewState::Dismissed
        ) {
            latest.insert(&r.user, r.state);
        }
    }
    if latest.values().any(|s| *s == ReviewState::ChangesRequested) {
        "CHANGES_REQUESTED"
    } else if latest.values().any(|s| *s == ReviewState::Approved) {
        "APPROVED"
    } else {
        ""
    }
}

/// `statusCheckRollup`: one `CheckRun` per scripted check (a rule concluding `Error`
/// is a `StatusContext`, the only shape with that state).
fn checks(
    state: &mut FakeGithub,
    bare: &Bare<'_>,
    repo: &FakeRepo,
    pr: &FakePr,
    host: &str,
) -> Result<Value, String> {
    let runs = rules::runs_for(state, bare, &pr.head_oid, super::github::now())?;
    let rows = runs
        .iter()
        .map(|run| {
            let url = format!(
                "https://{host}/{}/{}/actions/runs/{}/job/{}",
                repo.owner, repo.name, run.id, run.job
            );
            let pending = run.pending > 0;
            if run.conclusion == Conclusion::Error {
                return json!({
                    "__typename": "StatusContext",
                    "context": run.check,
                    "startedAt": iso(run.started_at),
                    "state": if pending { "PENDING" } else { "ERROR" },
                    "targetUrl": url,
                });
            }
            json!({
                "__typename": "CheckRun",
                "completedAt": if pending { "0001-01-01T00:00:00Z".to_string() } else { iso(run.started_at + 60) },
                "conclusion": if pending { "" } else { conclusion_text(run.conclusion) },
                "detailsUrl": url,
                "name": run.check,
                "startedAt": iso(run.started_at),
                "status": if pending { "IN_PROGRESS" } else { "COMPLETED" },
                "workflowName": "CI",
            })
        })
        .collect();
    Ok(Value::Array(rows))
}

fn conclusion_text(c: Conclusion) -> &'static str {
    match c {
        Conclusion::Success => "SUCCESS",
        Conclusion::Failure => "FAILURE",
        Conclusion::TimedOut => "TIMED_OUT",
        Conclusion::Cancelled => "CANCELLED",
        Conclusion::ActionRequired => "ACTION_REQUIRED",
        Conclusion::StartupFailure => "STARTUP_FAILURE",
        Conclusion::Neutral => "NEUTRAL",
        Conclusion::Skipped => "SKIPPED",
        Conclusion::Stale => "STALE",
        Conclusion::Error => "ERROR",
    }
}
