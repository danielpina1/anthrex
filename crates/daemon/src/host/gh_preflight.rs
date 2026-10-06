//! Decision 17: [`GhHost`]'s preflight, check by check, each with its own timeout, and
//! each refusal in the brief's exact text ("Messages"). Split from `gh.rs` for AGENTS.md
//! rule 8. Blocking.

use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::allow::AllowCtx;
use super::gh::{GH_OUTPUT_MAX, GhHost, MIN_GH_VERSION};
use super::gh_parse::{self, last_line};
use super::remote;
use super::runner::{Capture, Runner};
use super::{HOST_READ_TIMEOUT, HostError, HostRepo, PUSH_TIMEOUT, PreflightReq};
use crate::run::git::{NO_HOOKS, WRITE_FLAGS};
use crate::run::messages::one_line;

impl<R: Runner> GhHost<R> {
    pub(super) fn check_preflight(&self, req: &PreflightReq) -> Result<HostRepo, HostError> {
        let remote_name = req.remote.as_str();
        let (repo, full) = self.check_repo(&req.root, remote_name, Some(&req.base_branch))?;
        let ctx = AllowCtx {
            run_id: None,
            remote: remote_name,
            base_branch: Some(&req.base_branch),
            repo: Some(&full),
            pulls: &[],
        };
        let dry = format!(
            "{}:refs/heads/anthrex/preflight-{}",
            req.base_sha, req.nonce
        );
        let pushed = self
            .git(
                &ctx,
                &repo.root,
                &WRITE_FLAGS,
                &[
                    "push",
                    "--dry-run",
                    "--porcelain",
                    "--no-follow-tags",
                    "--recurse-submodules=no",
                    remote_name,
                    &dry,
                ],
                PUSH_TIMEOUT,
            )
            .map_err(|e| check_timeout(e, "git push --dry-run", PUSH_TIMEOUT))?;
        if !pushed.success {
            let why = match last_line(&pushed.stderr) {
                line if line.is_empty() => last_line(&pushed.stdout_text()),
                line => line,
            };
            // The controller's ruling on check 5: anthrex's own sentence, then git's
            // last line quoted, on one line (`last_line` caps it at 300 characters).
            return Err(HostError::Rejected(format!(
                "a dry-run push to {remote_name} was refused, so anthrex cannot push there; git said: \"{}\"",
                one_line(&why)
            )));
        }
        let base_ref = format!("refs/heads/{}", req.base_branch);
        let listed = self
            .git(
                &ctx,
                &repo.root,
                &NO_HOOKS,
                &["ls-remote", "--heads", remote_name, &base_ref],
                HOST_READ_TIMEOUT,
            )
            .map_err(|e| check_timeout(e, "git ls-remote", HOST_READ_TIMEOUT))?;
        if !listed.success {
            return Err(HostError::Failed(format!(
                "git ls-remote: {}",
                last_line(&listed.stderr)
            )));
        }
        let suffix = format!("\t{base_ref}");
        let text = listed.stdout_text();
        let Some(remote_sha) = text
            .lines()
            .find_map(|l| l.strip_suffix(&suffix))
            .map(str::trim)
        else {
            return Err(HostError::NotFound(format!(
                "the base branch {} does not exist on {remote_name}; push it first",
                req.base_branch
            )));
        };
        self.check_not_ahead(req, &ctx, &repo.root, remote_sha)?;
        Ok(repo)
    }

    /// The final fix wave (A5, review A M5): the run's stages start from the local base,
    /// and their PRs from the remote's, so a local base with commits the remote's lacks
    /// would put them in the first stage's PR. Counted locally; when the remote's commit
    /// is not in this repository (never fetched), nothing can be counted without a
    /// fetch, and preflight passes.
    fn check_not_ahead(
        &self,
        req: &PreflightReq,
        ctx: &AllowCtx<'_>,
        root: &Path,
        remote_sha: &str,
    ) -> Result<(), HostError> {
        if remote_sha == req.base_sha {
            return Ok(());
        }
        let range = format!("{remote_sha}..{}", req.base_sha);
        let counted = self
            .git(
                ctx,
                root,
                &NO_HOOKS,
                &["rev-list", "--count", &range],
                HOST_READ_TIMEOUT,
            )
            .map_err(|e| check_timeout(e, "git rev-list", HOST_READ_TIMEOUT))?;
        let ahead: u64 = match counted.stdout_text().trim().parse() {
            Ok(n) if counted.success => n,
            _ => return Ok(()),
        };
        let (base, remote) = (&req.base_branch, &req.remote);
        match ahead {
            0 => Ok(()),
            n => Err(HostError::Rejected(format!(
                "your {base} is {n} commit{} ahead of {remote}/{base}; push it first, or start from a pushed base",
                if n == 1 { "" } else { "s" }
            ))),
        }
    }

    /// [`super::CodeHost::remote_seal`]: the two reads through [`GhHost::git`] (so the
    /// allow-list), then their digest. A failure names the remote, never a URL.
    pub(super) fn seal_remote(&self, root: &Path, remote: &str) -> Result<String, HostError> {
        let ctx = AllowCtx {
            remote,
            ..AllowCtx::default()
        };
        let read = |args: &[&str]| -> Result<String, HostError> {
            let out = self.git(&ctx, root, &NO_HOOKS, args, HOST_READ_TIMEOUT)?;
            if out.success {
                Ok(out.stdout_text())
            } else {
                Err(HostError::Failed(format!(
                    "cannot read remote {remote}'s URLs: {}",
                    last_line(&out.stderr)
                )))
            }
        };
        let fetch = read(&["remote", "get-url", "--all", remote])?;
        let push = read(&["remote", "get-url", "--push", "--all", remote])?;
        let mut hash = Sha256::new();
        for part in ["fetch", &fetch, "push", &push] {
            hash.update(part.as_bytes());
            hash.update([0]);
        }
        Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Preflight's checks 1 to 4 (decision 17), which detection also runs (decision 3):
    /// the remote's URL, `gh --version`, `gh auth status` and `gh repo view`. Answers the
    /// repository and its `<owner>/<name>`.
    pub(super) fn check_repo(
        &self,
        root: &Path,
        remote_name: &str,
        base_branch: Option<&str>,
    ) -> Result<(HostRepo, String), HostError> {
        let early = AllowCtx {
            run_id: None,
            remote: remote_name,
            base_branch,
            repo: None,
            pulls: &[],
        };
        let key = format!("remote.{remote_name}.url");
        let config = self
            .git(
                &early,
                root,
                &NO_HOOKS,
                &["config", "--get", &key],
                HOST_READ_TIMEOUT,
            )
            .map_err(|e| check_timeout(e, "git config", HOST_READ_TIMEOUT))?;
        let url = config.stdout_text().trim().to_string();
        if !config.success || url.is_empty() {
            return Err(if config.stderr.trim().is_empty() {
                HostError::NotFound(format!(
                    "remote {remote_name} is not set in this repository; use --delivery local"
                ))
            } else {
                HostError::Failed(format!("git config: {}", last_line(&config.stderr)))
            });
        }
        let not_github = || {
            HostError::Rejected(format!(
                "remote {remote_name} is not a GitHub repository ({}); use --delivery local",
                remote::redact(&url)
            ))
        };
        let parsed = remote::parse(&url).ok_or_else(not_github)?;
        let repo = HostRepo {
            host: parsed.host,
            owner: parsed.owner,
            name: parsed.name,
            remote: remote_name.to_string(),
            root: root.to_path_buf(),
        };
        let full = repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..early
        };
        let cap = Capture::Bytes(GH_OUTPUT_MAX);
        let gh_cmd = |args: &[&str], check: &str| {
            let argv = args.iter().map(|a| a.to_string()).collect();
            self.gh(&repo.host, &ctx, &repo.root, argv, HOST_READ_TIMEOUT, cap)
                .map_err(|e| check_timeout(e, check, HOST_READ_TIMEOUT))
        };
        let missing = || {
            HostError::Missing(format!(
                "gh is not installed (looked for {}); install it, or use --delivery local",
                self.gh.display()
            ))
        };
        let version = match gh_cmd(&["--version"], "gh --version") {
            Err(HostError::Missing(_)) => return Err(missing()),
            Err(other) => return Err(other),
            Ok(out) if !out.success => return Err(missing()),
            Ok(out) => out.stdout_text(),
        };
        check_version(&version)?;
        let auth = gh_cmd(
            &["auth", "status", "--hostname", &repo.host],
            "gh auth status",
        )?;
        if !auth.success {
            return Err(if repo.host == remote::GITHUB_HOST {
                HostError::Auth(format!(
                    "gh is not logged in to {}; run gh auth login, or use --delivery local",
                    repo.host
                ))
            } else {
                not_github()
            });
        }
        let seen = gh_cmd(
            &["repo", "view", &full, "--json", "nameWithOwner"],
            "gh repo view",
        )?;
        if !seen.success {
            let text = format!("gh cannot see {full}: {}", last_line(&seen.stderr));
            return Err(with_text(gh_parse::classify(&seen.stderr), text));
        }
        Ok((repo, full))
    }
}

/// Preflight's timeout text: `<check>: gh did not answer within <n> s`.
fn check_timeout(error: HostError, check: &str, timeout: Duration) -> HostError {
    match error {
        HostError::TimedOut(_) => HostError::TimedOut(format!(
            "{check}: gh did not answer within {} s",
            timeout.as_secs()
        )),
        other => other,
    }
}

fn with_text(kind: HostError, text: String) -> HostError {
    match kind {
        HostError::Missing(_) => HostError::Missing(text),
        HostError::Auth(_) => HostError::Auth(text),
        HostError::NotFound(_) => HostError::NotFound(text),
        HostError::RateLimited(_) => HostError::RateLimited(text),
        HostError::Rejected(_) => HostError::Rejected(text),
        HostError::Forbidden(_) => HostError::Forbidden(text),
        HostError::TimedOut(_) => HostError::TimedOut(text),
        HostError::Failed(_) => HostError::Failed(text),
    }
}

fn check_version(printed: &str) -> Result<(), HostError> {
    let (a, b, c) = MIN_GH_VERSION;
    match gh_parse::gh_version(printed) {
        Some(found) if found >= MIN_GH_VERSION => Ok(()),
        Some((x, y, z)) => Err(HostError::Missing(format!(
            "gh {x}.{y}.{z} is older than {a}.{b}.{c}, the oldest version anthrex is checked against; upgrade it, or use --delivery local"
        ))),
        None => Err(HostError::Missing(format!(
            "gh --version printed no version ({}); install gh {a}.{b}.{c} or later, or use --delivery local",
            last_line(printed)
        ))),
    }
}
