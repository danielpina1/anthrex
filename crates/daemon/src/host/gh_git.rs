//! Decision 13: [`GhHost`]'s `git` side, `push`, `fetch` (with decision 24's adoption)
//! and the branch delete. Every command passes the allow-list and runs through the
//! [`Runner`] (so through `worktree::run_git`); the adoption's compare-and-swap is 9.1's
//! `git::refs_tx::cas`. Split from `gh.rs` for AGENTS.md rule 8. Blocking.

use std::time::Instant;

use super::allow::{self, AllowCtx};
use super::gh::{GhHost, run_ctx};
use super::gh_parse::{self, last_line};
use super::runner::Runner;
use super::{
    Adopt, Contains, DeleteBranchReq, FetchOutcome, FetchReq, HOST_READ_TIMEOUT, HostError,
    PUSH_TIMEOUT, PushOutcome, PushReq,
};
use crate::run::git::{NO_HOOKS, WRITE_FLAGS, refs_tx};

/// The fetch refspec's update marker (only ever onto anthrex's own `refs/anthrex/…`).
const FETCH_UPDATE: char = '+';

/// `refs/heads/anthrex/<run>/stage-<n>`: the only branch anthrex pushes.
pub(crate) fn stage_ref(run_id: &str, stage: u16) -> String {
    format!("refs/heads/anthrex/{run_id}/stage-{stage}")
}

/// The one shape of anthrex's fetch (the allow-list's fetch rule): `spec` from `remote`.
fn fetch_argv<'a>(remote: &'a str, spec: &'a str) -> [&'a str; 10] {
    [
        "fetch",
        "--no-tags",
        "--no-prune",
        "--no-prune-tags",
        "--no-recurse-submodules",
        "--no-auto-maintenance",
        "--no-write-fetch-head",
        "--refmap=",
        remote,
        spec,
    ]
}

impl<R: Runner> GhHost<R> {
    pub(super) fn push_stage(&self, req: &PushReq) -> Result<PushOutcome, HostError> {
        let ctx = run_ctx(&req.repo, Some(&req.run_id));
        let full = req.repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..ctx
        };
        let dst = stage_ref(&req.run_id, req.stage);
        let spec = format!("{}:{dst}", req.sha);
        let out = self.git(
            &ctx,
            &req.repo.root,
            &WRITE_FLAGS,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                &req.repo.remote,
                &spec,
            ],
            PUSH_TIMEOUT,
        )?;
        gh_parse::push_outcome(&out, &dst)
    }

    pub(super) fn fetch_ref(&self, req: &FetchReq) -> Result<FetchOutcome, HostError> {
        let ctx = run_ctx(&req.repo, Some(&req.run_id));
        let spec = format!("{FETCH_UPDATE}refs/heads/{}:{}", req.branch, req.into);
        // `--no-prune --no-prune-tags` (deferred from task 4): a user's `fetch.prune`
        // or `pruneTags` never makes anthrex's fetch delete a ref.
        let out = self.git(
            &ctx,
            &req.repo.root,
            &WRITE_FLAGS,
            &fetch_argv(&req.repo.remote, &spec),
            PUSH_TIMEOUT,
        )?;
        if !out.success {
            return if out.stderr.contains("couldn't find remote ref") {
                Ok(FetchOutcome::Missing)
            } else {
                Err(gh_parse::classify(&out.stderr))
            };
        }
        let at = self.git(
            &ctx,
            &req.repo.root,
            &NO_HOOKS,
            &["rev-parse", "--verify", &format!("{}^{{commit}}", req.into)],
            HOST_READ_TIMEOUT,
        )?;
        let sha = at.stdout_text().trim().to_string();
        if !at.success || !allow::is_object_id(&sha) {
            return Err(HostError::Failed(format!(
                "git rev-parse {}: {}",
                req.into,
                last_line(&at.stderr)
            )));
        }
        match &req.adopt {
            None => {
                let parents = match &req.parents_of {
                    Some(oid) => self.parents(&ctx, req, oid)?,
                    None => None,
                };
                let contains = req
                    .contains
                    .as_ref()
                    .and_then(|c| self.contains(&ctx, req, c, parents));
                Ok(FetchOutcome::Fetched {
                    sha,
                    parents,
                    contains,
                })
            }
            Some(adopt) => self.adopt(req, &ctx, adopt, &sha),
        }
    }

    /// Ruling R-4: how many parents commit `oid` has, read locally after the fetch
    /// (`None` when the repository does not have it).
    fn parents(
        &self,
        ctx: &AllowCtx<'_>,
        req: &FetchReq,
        oid: &str,
    ) -> Result<Option<u32>, HostError> {
        // Fix round 1 (m2): an oid the allow-list would refuse fails nothing.
        if !allow::is_object_id(oid) {
            return Ok(None);
        }
        let out = self.git(
            ctx,
            &req.repo.root,
            &NO_HOOKS,
            &["rev-list", "--parents", "-n", "1", oid],
            HOST_READ_TIMEOUT,
        )?;
        if !out.success {
            return Ok(None);
        }
        let text = out.stdout_text();
        let mut words = text.split_whitespace();
        let found = words.next() == Some(oid);
        Ok(found.then(|| u32::try_from(words.count()).unwrap_or(u32::MAX)))
    }

    /// Milestone 9.7 decision 6 (DH §1.2): whether merged head `c.merged` holds stage
    /// `c.stage`'s local head `c.head`, read after the base fetch, inside the same op (so
    /// on the git queue). Unless the merge commit has two parents (its second, the merged
    /// head, came with the base fetch), the stage branch is fetched into the stage's own
    /// private ref first. Every failure here is `None` ("could not check"), never the
    /// op's error: only the base fetch's own failure fails the op (ruling R1 counts it).
    fn contains(
        &self,
        ctx: &AllowCtx<'_>,
        req: &FetchReq,
        c: &Contains,
        parents: Option<u32>,
    ) -> Option<bool> {
        let run = &req.run_id;
        // Only the asked stage's branch, only into its `remote/stage-<n>`; two object ids.
        let own = c.branch == format!("anthrex/{run}/stage-{}", c.stage)
            && c.into == format!("refs/anthrex/{run}/remote/stage-{}", c.stage);
        if !own || !allow::is_object_id(&c.head) || !allow::is_object_id(&c.merged) {
            return None;
        }
        if parents.is_none_or(|n| n < 2) {
            let spec = format!("{FETCH_UPDATE}refs/heads/{}:{}", c.branch, c.into);
            let out = self
                .git(
                    ctx,
                    &req.repo.root,
                    &WRITE_FLAGS,
                    &fetch_argv(&req.repo.remote, &spec),
                    PUSH_TIMEOUT,
                )
                .ok()?;
            if !out.success {
                return None;
            }
        }
        let out = self
            .git(
                ctx,
                &req.repo.root,
                &NO_HOOKS,
                &["merge-base", "--is-ancestor", &c.head, &c.merged],
                HOST_READ_TIMEOUT,
            )
            .ok()?;
        // Exit 1 with nothing on stderr is "not an ancestor"; any other failure (128, an
        // object the repository lacks) is unknown.
        if out.success {
            Some(true)
        } else if out.stderr.trim().is_empty() {
            Some(false)
        } else {
            None
        }
    }

    /// Decision 24: the remote head descends from `expected_local`; move the local ref
    /// (and `integration`) to it in one compare-and-swap.
    fn adopt(
        &self,
        req: &FetchReq,
        ctx: &AllowCtx<'_>,
        adopt: &Adopt,
        fetched: &str,
    ) -> Result<FetchOutcome, HostError> {
        let repo = &req.repo;
        let ancestor = self.git(
            ctx,
            &repo.root,
            &NO_HOOKS,
            &[
                "merge-base",
                "--is-ancestor",
                &adopt.expected_local,
                fetched,
            ],
            HOST_READ_TIMEOUT,
        )?;
        if !ancestor.success {
            return if ancestor.stderr.trim().is_empty() {
                Ok(FetchOutcome::NotDescendant {
                    remote: fetched.to_string(),
                })
            } else {
                Err(gh_parse::classify(&ancestor.stderr))
            };
        }
        let run = ctx.run_id.unwrap_or_default();
        let moved = allow::local_run_ref(&adopt.local_ref, run)
            .ok_or_else(|| allow::forbidden_text(&format!("adopt into {}", adopt.local_ref)))?;
        // Fix wave A2: the swap needs its own bound inside the op's; without it, the op
        // may already have answered `TimedOut`, and a ref moved now is one the engine
        // was told never moved.
        if req
            .deadline
            .is_some_and(|deadline| Instant::now() + HOST_READ_TIMEOUT > deadline)
        {
            return Err(HostError::TimedOut(format!(
                "fetch ran out of its bound before adopting into {}; nothing was moved",
                adopt.local_ref
            )));
        }
        let mut updates = vec![(moved, fetched.to_string(), adopt.expected_local.clone())];
        if adopt.also_integration {
            updates.push((
                format!("refs/heads/anthrex/{run}/integration"),
                fetched.to_string(),
                adopt.expected_local.clone(),
            ));
        }
        let swap = refs_tx::cas(
            self.git.as_os_str(),
            &repo.root,
            &updates,
            &[],
            HOST_READ_TIMEOUT,
        )
        .map_err(HostError::Failed)?;
        match swap {
            refs_tx::Swap::Done => Ok(FetchOutcome::Adopted {
                sha: fetched.to_string(),
            }),
            refs_tx::Swap::Moved(refname) => {
                let at = self.git(
                    ctx,
                    &repo.root,
                    &NO_HOOKS,
                    &["rev-parse", "--verify", &format!("{refname}^{{commit}}")],
                    HOST_READ_TIMEOUT,
                )?;
                let local = if at.success {
                    at.stdout_text().trim().to_string()
                } else {
                    String::new()
                };
                Ok(FetchOutcome::LocalMoved { local })
            }
        }
    }

    pub(super) fn delete_stage(&self, req: &DeleteBranchReq) -> Result<(), HostError> {
        let ctx = run_ctx(&req.repo, Some(&req.run_id));
        let dst = stage_ref(&req.run_id, req.stage);
        let out = self.git(
            &ctx,
            &req.repo.root,
            &WRITE_FLAGS,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                &req.repo.remote,
                "--delete",
                &dst,
            ],
            PUSH_TIMEOUT,
        )?;
        // Decision 10: a branch already gone is success.
        if out.success || out.stderr.contains("remote ref does not exist") {
            Ok(())
        } else {
            match gh_parse::push_outcome(&out, &dst)? {
                PushOutcome::Refused { reason } | PushOutcome::Rejected { reason } => {
                    Err(HostError::Rejected(reason))
                }
                PushOutcome::Pushed | PushOutcome::UpToDate => Ok(()),
            }
        }
    }
}
