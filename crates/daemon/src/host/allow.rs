//! Decision 7: the allow-list every host command passes before any process starts. Pure.
//!
//! It accepts exactly the argv shapes of the brief's "GhHost commands" (with rulings
//! R-2 and R-10) and refuses everything else with `HostError::Forbidden("anthrex never
//! runs: <argv>")`. So no `gh pr merge` (with or without `--auto`), no `gh pr review`,
//! `gh pr close` or `gh pr ready`, no `gh api` call but the five listed (no `PUT …/merge`,
//! no `…/reviews`, no GraphQL but [`THREADS_QUERY`]), and for `git push` no force flag,
//! no `--mirror`/`--all`/`--tags`/`--prune`, no `+` refspec, no symbolic source and no
//! destination but this run's `refs/heads/anthrex/<run>/stage-<n>` (or the preflight's
//! `--dry-run` to `refs/heads/anthrex/preflight-<nonce>`). It guards `GhHost` over every
//! runner, the real CLI and `FakeGh` alike.

use super::HostError;
use super::gh::{PR_LIST_FIELDS, PR_VIEW_FIELDS, THREADS_QUERY};
use super::remote::{host_ok, name_ok, owner_ok};
use super::runner::{Program, shown};
use crate::run::git::{NO_HOOKS, WRITE_FLAGS};

/// What a command may name: the run whose branches it may touch, the remote, the base
/// branch (preflight's `ls-remote`) and the repository (`<owner>/<name>`, every `gh`
/// repository command).
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowCtx<'a> {
    pub run_id: Option<&'a str>,
    pub remote: &'a str,
    pub base_branch: Option<&'a str>,
    pub repo: Option<&'a str>,
}

/// Ruling I1: a user's `push.followTags` must not send their tags, nor
/// `push.recurseSubmodules` submodule commits, with anthrex's push.
const NO_FOLLOW_TAGS: &str = "--no-follow-tags";
const NO_SUBMODULES: &str = "--recurse-submodules=no";

/// Refused outright wherever they appear in a `git` command.
const GIT_REFUSED: [&str; 9] = [
    "--force",
    "-f",
    "--force-with-lease",
    "--force-if-includes",
    "--mirror",
    "--all",
    "--tags",
    "--prune",
    "--no-verify",
];

pub fn check(program: Program, argv: &[String], ctx: &AllowCtx<'_>) -> Result<(), HostError> {
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let allowed = match program {
        Program::Gh => gh_allowed(&args, ctx),
        Program::Git => git_allowed(&args, ctx),
    };
    if allowed {
        Ok(())
    } else {
        Err(forbidden_text(&shown(program, argv)))
    }
}

pub(crate) fn forbidden_text(what: &str) -> HostError {
    HostError::Forbidden(format!("anthrex never runs: {what}"))
}

fn git_allowed(args: &[&str], ctx: &AllowCtx<'_>) -> bool {
    if seal_read(args, ctx) {
        return true;
    }
    let refused = |a: &&str| GIT_REFUSED.contains(a) || a.starts_with("--force");
    if args.iter().any(refused) {
        return false;
    }
    let (write, read, rest) = if args.starts_with(&WRITE_FLAGS) {
        (true, false, &args[WRITE_FLAGS.len()..])
    } else if args.starts_with(&NO_HOOKS) && args.get(NO_HOOKS.len()) != Some(&"-c") {
        (false, true, &args[NO_HOOKS.len()..])
    } else {
        return false;
    };
    let remote = |r: &str| r == ctx.remote && !r.is_empty() && !r.starts_with('-');
    match rest {
        ["config", "--get", key] => {
            read && remote(ctx.remote) && *key == format!("remote.{}.url", ctx.remote)
        }
        ["ls-remote", "--heads", r, base] => {
            read && remote(r)
                && ctx
                    .base_branch
                    .is_some_and(|b| branch_ok(b) && *base == format!("refs/heads/{b}"))
        }
        [
            "push",
            "--dry-run",
            "--porcelain",
            NO_FOLLOW_TAGS,
            NO_SUBMODULES,
            r,
            spec,
        ] => write && remote(r) && preflight_spec(spec),
        [
            "push",
            "--porcelain",
            NO_FOLLOW_TAGS,
            NO_SUBMODULES,
            r,
            "--delete",
            dst,
        ] => write && remote(r) && ctx.run_id.is_some_and(|run| is_stage_ref(dst, run)),
        [
            "push",
            "--porcelain",
            NO_FOLLOW_TAGS,
            NO_SUBMODULES,
            r,
            spec,
        ] => write && remote(r) && push_spec(spec, ctx.run_id),
        [
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-auto-maintenance",
            "--no-write-fetch-head",
            "--refmap=",
            r,
            spec,
        ] => write && remote(r) && fetch_spec(spec, ctx.run_id),
        ["rev-parse", "--verify", r] => read && rev_ref(r, ctx.run_id),
        ["merge-base", "--is-ancestor", a, b] => read && is_object_id(a) && is_object_id(b),
        // Ruling R-4: a merged PR's merge commit's parents, read locally.
        ["rev-list", "--parents", "-n", "1", oid] => read && is_object_id(oid),
        _ => false,
    }
}

/// Task M9.2.12's seal (fix round 1, I1): `git remote get-url --all <remote>` and
/// `get-url --push --all <remote>`, the run's remote only, read-only (no write flags).
/// Their `--all` lists every URL; it is not `push --all`, which stays refused.
fn seal_read(args: &[&str], ctx: &AllowCtx<'_>) -> bool {
    let Some(rest) = args.strip_prefix(&NO_HOOKS[..]) else {
        return false;
    };
    let remote = |r: &str| r == ctx.remote && !r.is_empty() && !r.starts_with('-');
    match rest {
        ["remote", "get-url", "--all", r] | ["remote", "get-url", "--push", "--all", r] => {
            remote(r)
        }
        _ => false,
    }
}

fn gh_allowed(args: &[&str], ctx: &AllowCtx<'_>) -> bool {
    let repo = |r: &str| ctx.repo == Some(r) && repo_ok(r);
    let number = |n: &str| is_number(n);
    let head = |h: &str| ctx.run_id.is_some_and(|run| stage_branch(h) == Some(run));
    let path_of = |p: &str, mid: &str| -> Option<String> {
        let full = ctx.repo?;
        p.strip_prefix(&format!("repos/{full}/{mid}"))
            .map(str::to_string)
    };
    match args {
        ["--version"] => true,
        ["auth", "status", "--hostname", h] => host_ok(h),
        ["repo", "view", r, "--json", "nameWithOwner"] => repo(r),
        [
            "pr",
            "list",
            "--repo",
            r,
            "--head",
            h,
            "--state",
            "all",
            "--json",
            fields,
        ] => repo(r) && head(h) && *fields == PR_LIST_FIELDS,
        [
            "pr",
            "create",
            "--repo",
            r,
            "--base",
            b,
            "--head",
            h,
            "--title",
            t,
            "--body-file",
            f,
        ] => repo(r) && branch_ok(b) && head(h) && title_ok(t) && f.starts_with('/'),
        ["pr", "view", n, "--repo", r, "--json", fields] => {
            number(n) && repo(r) && *fields == PR_VIEW_FIELDS
        }
        ["api", "graphql", "-f", q, "-f", o, "-f", nm, "-F", num] => {
            let Some((owner, name)) = ctx.repo.and_then(|r| r.split_once('/')) else {
                return false;
            };
            q.strip_prefix("query=") == Some(THREADS_QUERY)
                && *o == format!("owner={owner}")
                && *nm == format!("name={name}")
                && num.strip_prefix("number=").is_some_and(is_number)
        }
        ["run", "view", id, "--repo", r, "--log-failed"] => number(id) && repo(r),
        ["run", "rerun", id, "--repo", r, "--failed"] => number(id) && repo(r),
        ["api", p, "--paginate"] => {
            let listing = |mid: &str| {
                path_of(p, mid)
                    .and_then(|rest| rest.strip_suffix("/comments").map(is_number))
                    .unwrap_or(false)
            };
            listing("pulls/") || listing("issues/")
        }
        ["api", "-X", "POST", p, "-f", b] => {
            let replies = path_of(p, "pulls/").is_some_and(|rest| {
                let mut parts = rest.split('/');
                matches!(
                    (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()),
                    (Some(n), Some("comments"), Some(id), Some("replies"), None)
                        if is_number(n) && is_number(id)
                )
            });
            replies && b.starts_with("body=")
        }
        ["pr", "comment", n, "--repo", r, "--body-file", f] => {
            number(n) && repo(r) && f.starts_with('/')
        }
        ["pr", "edit", n, "--repo", r, "--base", b] => number(n) && repo(r) && branch_ok(b),
        // Task M9.2.10's fix round: the login anthrex posts with, to know its replies.
        ["api", "user"] => true,
        ["api", p] => path_of(p, "collaborators/")
            .is_some_and(|rest| rest.strip_suffix("/permission").is_some_and(login_ok)),
        _ => false,
    }
}

/// `<owner>/<name>`.
fn repo_ok(repo: &str) -> bool {
    repo.split_once('/')
        .is_some_and(|(o, n)| owner_ok(o) && name_ok(n))
}

fn login_ok(login: &str) -> bool {
    owner_ok(login)
}

fn title_ok(title: &str) -> bool {
    !title.is_empty() && title.chars().count() <= 256 && !title.contains(['\n', '\r'])
}

fn is_number(text: &str) -> bool {
    (1..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

/// A full object id: 40 (SHA-1) or 64 (SHA-256) lowercase hexadecimal digits.
pub fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A run id as the daemon makes them: letters, digits, `-` and `_`.
fn run_ok(run: &str) -> bool {
    (1..=64).contains(&run.len())
        && run
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A branch name anthrex may name as a base: `git check-ref-format --branch`'s rule,
/// close enough to refuse anything that could read as an option or a pattern.
fn branch_ok(branch: &str) -> bool {
    let bad = |c: char| c.is_ascii_control() || " ~^:?*[\\".contains(c);
    !branch.is_empty()
        && !branch.starts_with(['-', '/'])
        && !branch.ends_with(['/', '.'])
        && !branch.contains("..")
        && !branch.contains("@{")
        && !branch.contains("//")
        && !branch.chars().any(bad)
        && branch
            .split('/')
            .all(|part| !part.starts_with('.') && !part.ends_with(".lock"))
}

/// `stage-<n>`, `n` in 1..=65535 without a leading zero.
fn stage_name(name: &str) -> bool {
    name.strip_prefix("stage-")
        .is_some_and(|n| is_number(n) && !n.starts_with('0') && n.parse::<u16>().is_ok())
}

/// `anthrex/<run>/stage-<n>` → `run`.
pub(crate) fn stage_branch(branch: &str) -> Option<&str> {
    let rest = branch.strip_prefix("anthrex/")?;
    let (run, stage) = rest.split_once('/')?;
    (run_ok(run) && stage_name(stage)).then_some(run)
}

/// `refs/heads/anthrex/<run>/stage-<n>` of exactly `run`.
fn is_stage_ref(refname: &str, run: &str) -> bool {
    refname
        .strip_prefix("refs/heads/")
        .and_then(stage_branch)
        .is_some_and(|r| r == run)
}

/// `refs/anthrex/<run>/remote/<name>` → `run` (decision 13's private fetch ref).
pub(crate) fn private_ref_run(refname: &str) -> Option<&str> {
    let rest = refname.strip_prefix("refs/anthrex/")?;
    let (run, rest) = rest.split_once('/')?;
    let name = rest.strip_prefix("remote/")?;
    let name_ok = !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        && !name.contains("..");
    (run_ok(run) && name_ok).then_some(run)
}

/// The full ref of a local run branch an adoption may move: `anthrex/<run>/stage-<n>`
/// or `anthrex/<run>/integration` of exactly `run`.
pub(crate) fn local_run_ref(branch: &str, run: &str) -> Option<String> {
    let integration = format!("anthrex/{run}/integration");
    (branch == integration || stage_branch(branch) == Some(run))
        .then(|| format!("refs/heads/{branch}"))
}

fn preflight_spec(spec: &str) -> bool {
    spec.split_once(':').is_some_and(|(src, dst)| {
        is_object_id(src)
            && dst
                .strip_prefix("refs/heads/anthrex/preflight-")
                .is_some_and(|nonce| {
                    nonce.len() == 8
                        && nonce
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
    })
}

fn push_spec(spec: &str, run: Option<&str>) -> bool {
    !spec.starts_with('+')
        && spec.split_once(':').is_some_and(|(src, dst)| {
            is_object_id(src) && run.is_some_and(|run| is_stage_ref(dst, run))
        })
}

/// `+refs/heads/<branch>:refs/anthrex/<run>/remote/<name>`: the `+` only ever updates
/// anthrex's own private ref.
fn fetch_spec(spec: &str, run: Option<&str>) -> bool {
    let Some(rest) = spec.strip_prefix('+') else {
        return false;
    };
    rest.split_once(':').is_some_and(|(src, dst)| {
        src.strip_prefix("refs/heads/").is_some_and(branch_ok)
            && run.is_some_and(|run| private_ref_run(dst) == Some(run))
    })
}

/// `<ref>^{commit}` for this run's private fetch ref or one of its local branches.
fn rev_ref(spec: &str, run: Option<&str>) -> bool {
    let (Some(refname), Some(run)) = (spec.strip_suffix("^{commit}"), run) else {
        return false;
    };
    private_ref_run(refname) == Some(run)
        || refname
            .strip_prefix("refs/heads/")
            .and_then(|b| local_run_ref(b, run))
            .is_some()
}
