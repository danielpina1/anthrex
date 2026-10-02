//! Decision 13: [`GhHost`]'s `git` side, `push`, `fetch` (with decision 24's adoption)
//! and the branch delete. Every command passes the allow-list and runs through the
//! [`Runner`] (so through `worktree::run_git`); the adoption's compare-and-swap is 9.1's
//! `git::refs_tx::cas`. Split from `gh.rs` for AGENTS.md rule 8. Blocking.

use super::allow::{self, AllowCtx};
use super::gh::{GhHost, run_ctx};
use super::gh_parse::{self, last_line};
use super::runner::Runner;
use super::{
    Adopt, DeleteBranchReq, FetchOutcome, FetchReq, HOST_READ_TIMEOUT, HostError, PUSH_TIMEOUT,
    PushOutcome, PushReq,
};
use crate::run::git::{NO_HOOKS, WRITE_FLAGS, refs_tx};

/// The fetch refspec's update marker (only ever onto anthrex's own `refs/anthrex/…`).
const FETCH_UPDATE: char = '+';

/// `refs/heads/anthrex/<run>/stage-<n>`: the only branch anthrex pushes.
pub(crate) fn stage_ref(run_id: &str, stage: u16) -> String {
    format!("refs/heads/anthrex/{run_id}/stage-{stage}")
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
        let out = self.git(
            &ctx,
            &req.repo.root,
            &WRITE_FLAGS,
            &[
                "fetch",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-auto-maintenance",
                "--no-write-fetch-head",
                "--refmap=",
                &req.repo.remote,
                &spec,
            ],
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
                Ok(FetchOutcome::Fetched { sha, parents })
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
