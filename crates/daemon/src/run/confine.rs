//! Confining the engine's check and proof commands (final fix batch F1c, re-review 4,
//! I2). Blocking file-system work: call only from `spawn_blocking` or a dedicated
//! thread (AGENTS.md rule 2).
//!
//! A check or a proof runs code a worker wrote (a test, a `build.rs`, a script the
//! check calls). Workers are sandboxed so that nothing they write reaches the user's
//! repository but through the engine; a check run with the daemon's own rights would
//! undo that. So when the run's workers are sandboxed (`[orchestrator] worker_sandbox`,
//! decision 54), each check, proof run and integration-candidate check runs under a
//! sandbox profile that denies every file write but to:
//! - the checkout it runs in (its files; for the integration checkout, never its git
//!   dir, which is in the user's `.git`);
//! - the checkout's own object store when it is its own repository (F1c, 3a);
//! - its per-checkout temporary directory, `<data>/runs/<run>/tasks/<name>/tmp`, which
//!   is `TMPDIR` for the command;
//! - the profile's `cache_dirs`, none of which may overlap the git common directory or
//!   anthrex's data directory;
//! - `/dev`.
//!
//! Everything else, the user's `.git`, their checkout and `$HOME` included, is
//! read-only. Every writable path is spelled the way [`super::git::private_dir`] spells
//! a worker's grant (I1): the engine's parent resolved, the leaf's name appended and
//! checked with `lstat`, never a path resolved through something the command could
//! have swapped.
//!
//! On macOS the profile is applied with `/usr/bin/sandbox-exec -p <profile> /bin/sh`,
//! which applies the profile and `exec`s the shell: the pid the engine waits for and
//! kills is the shell's, as before. Linux has no equivalent the engine can rely on, so
//! there [`AVAILABLE`] is false and the commands run unconfined (recorded in the
//! brief's implementation notes and manual check 4e).

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use super::exec::{ShellOutcome, run_confined};
use super::git::{Repo, checkout_repo_dir, private_dir};
use super::model::Run;

/// The program that applies a profile, on macOS.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Whether this platform can confine a command.
pub const AVAILABLE: bool = cfg!(target_os = "macos");

/// What a run's confined commands may write, from its frozen record: set only when its
/// workers are sandboxed and [`AVAILABLE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfineSpec {
    /// The run's data directory, `<data>/runs/<run>`.
    pub data_dir: PathBuf,
    /// The repository's git common directory.
    pub common_dir: PathBuf,
    /// The profile's `cache_dirs`, as written (`~/` is the daemon's `$HOME`; a relative
    /// path is the checkout's).
    pub cache_dirs: Vec<String>,
}

/// One command's confinement: what it may write, and its `TMPDIR`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confinement {
    writable: Vec<PathBuf>,
    tmp: PathBuf,
}

impl ConfineSpec {
    /// `run`'s, when its workers are sandboxed and this platform can confine.
    pub fn for_run(run: &Run) -> Option<Self> {
        (run.limits.worker_sandbox && AVAILABLE).then(|| ConfineSpec {
            data_dir: run.data_dir.clone(),
            common_dir: run.git_common_dir.clone(),
            cache_dirs: run.profile.cache_dirs.clone(),
        })
    }

    /// The confinement for a command in the checkout `dir`. Fails closed: a checkout,
    /// temporary directory or object store that is not a plain directory, or a cache
    /// directory that overlaps the git common directory or anthrex's data directory,
    /// is an error, and the command must not run.
    pub fn for_checkout(&self, dir: &Path) -> Result<Confinement, String> {
        let checkout = plain_dir(dir)?;
        let repo = Repo::at(&checkout_repo_dir(&self.data_dir, dir));
        let tmp = private_dir(&self.common_dir, &repo.tmp())?;
        let mut writable = vec![checkout.clone(), tmp.clone()];
        if repo.git_dir().is_dir() {
            writable.push(private_dir(&self.common_dir, &repo.objects())?);
        }
        let common = canonical(&self.common_dir);
        let data = canonical(
            self.data_dir
                .parent()
                .and_then(Path::parent)
                .unwrap_or(&self.data_dir),
        );
        for raw in &self.cache_dirs {
            let cache = cache_dir(raw, &checkout)?;
            for (what, other) in [
                ("the git common directory", &common),
                ("anthrex's data directory", &data),
            ] {
                if cache.starts_with(other) || other.starts_with(&cache) {
                    return Err(format!(
                        "profile cache_dirs entry {raw} ({}) overlaps {what} {}; a check may write nothing of it",
                        cache.display(),
                        other.display()
                    ));
                }
            }
            writable.push(cache);
        }
        Ok(Confinement { writable, tmp })
    }
}

impl Confinement {
    /// The paths the command may write, beside `/dev`.
    pub fn writable(&self) -> &[PathBuf] {
        &self.writable
    }

    /// The command's `TMPDIR`.
    pub fn tmp(&self) -> &Path {
        &self.tmp
    }

    /// The `sandbox-exec` profile: everything allowed but file writes, which are
    /// allowed only under [`Self::writable`] and `/dev`.
    pub fn profile(&self) -> Result<String, String> {
        let mut profile =
            String::from("(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write*\n");
        profile.push_str("  (subpath \"/dev\")");
        for path in &self.writable {
            profile.push_str(&format!("\n  (subpath {})", sbpl_string(path)?));
        }
        profile.push_str(")\n");
        Ok(profile)
    }
}

/// `command` in the checkout `dir`, confined as `confine` says when it is set; a
/// checkout that cannot be confined fails the command unrun.
pub fn confined(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    timeout: Duration,
    confine: Option<&ConfineSpec>,
) -> ShellOutcome {
    match confine.map(|spec| spec.for_checkout(dir)).transpose() {
        Ok(confinement) => run_confined(dir, command, env, timeout, confinement.as_ref()),
        Err(error) => ShellOutcome::refused(error),
    }
}

/// `path` as a real directory spelled with its parent resolved and its own name
/// appended (never resolved itself): a link or a file there is refused.
fn plain_dir(path: &Path) -> Result<PathBuf, String> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(format!("{} has no parent directory", path.display()));
    };
    let parent = parent
        .canonicalize()
        .map_err(|err| format!("cannot resolve {}: {err}", parent.display()))?;
    let dir = parent.join(name);
    let meta = std::fs::symlink_metadata(&dir)
        .map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} is not a plain directory; it was tampered with",
            dir.display()
        ));
    }
    Ok(dir)
}

/// A `cache_dirs` entry as an absolute path: `~` and `~/…` under `$HOME`, a relative
/// one under the checkout; `..` is refused. Its nearest existing ancestor is resolved
/// (a sandbox matches resolved paths) and the rest appended.
fn cache_dir(raw: &str, checkout: &Path) -> Result<PathBuf, String> {
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| format!("profile cache_dirs entry {raw}: $HOME is not set"))
    };
    let path = if raw == "~" {
        home()?
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home()?.join(rest)
    } else if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        checkout.join(raw)
    };
    if raw.trim().is_empty() || path.components().any(|c| c == Component::ParentDir) {
        return Err(format!(
            "profile cache_dirs entry {raw:?} must be a path without `..`"
        ));
    }
    let mut existing = path.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => break,
        }
    }
    let mut resolved = canonical(existing);
    for name in rest.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// `path` as an SBPL string literal. A path that is not UTF-8 or holds a control
/// character is refused rather than approximated.
fn sbpl_string(path: &Path) -> Result<String, String> {
    let Some(text) = path.to_str() else {
        return Err(format!("{} is not UTF-8", path.display()));
    };
    if text.chars().any(char::is_control) {
        return Err(format!("{text:?} holds a control character"));
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(root: &Path, cache_dirs: &[&str]) -> ConfineSpec {
        let common = root.join("repo/.git");
        std::fs::create_dir_all(&common).unwrap();
        ConfineSpec {
            data_dir: root.join("data/runs/r1"),
            common_dir: common,
            cache_dirs: cache_dirs.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_checkout_gets_itself_its_tmp_and_the_caches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let checkout = root.join("wt/runs/r1/t1");
        std::fs::create_dir_all(&checkout).unwrap();
        let c = spec(&root, &["/opt/cache/x", "rel"])
            .for_checkout(&checkout)
            .unwrap();
        let tmp = root.join("data/runs/r1/tasks/t1/tmp");
        assert!(tmp.is_dir());
        assert_eq!(c.tmp(), tmp);
        assert_eq!(
            c.writable(),
            [
                checkout.clone(),
                tmp,
                canonical(Path::new("/opt")).join("cache/x"),
                checkout.join("rel"),
            ]
        );
        let profile = c.profile().unwrap();
        assert!(profile.contains("(deny file-write*)"), "{profile}");
        assert!(profile.contains(&format!("(subpath \"{}\")", checkout.display())));
    }

    #[test]
    fn a_cache_over_the_repository_or_the_data_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let checkout = root.join("wt/runs/r1/t1");
        std::fs::create_dir_all(&checkout).unwrap();
        let repo = root.join("repo");
        for bad in [
            repo.display().to_string(),
            repo.join(".git/objects").display().to_string(),
            root.display().to_string(),
            root.join("data/runs/r2").display().to_string(),
            "../x".to_string(),
        ] {
            let err = spec(&root, &[&bad]).for_checkout(&checkout).unwrap_err();
            assert!(
                err.contains("overlaps") || err.contains("`..`"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn a_checkout_swapped_for_a_link_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let checkout = root.join("wt/runs/r1/t1");
        std::fs::create_dir_all(checkout.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&root, &checkout).unwrap();
        let err = spec(&root, &[]).for_checkout(&checkout).unwrap_err();
        assert!(err.contains("tampered"), "{err}");
    }

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        assert_eq!(
            sbpl_string(Path::new("/a \"b\"\\c")).unwrap(),
            r#""/a \"b\"\\c""#
        );
        assert!(sbpl_string(Path::new("/a\nb")).is_err());
    }
}
