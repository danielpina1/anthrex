//! A confined command's `cache_dirs` (final fix batch F1c round 3, N3; F1d, R3):
//! directories from the user's own `[orchestrator.cache_dirs]` a check, proof or
//! `setup` may also write. Blocking file-system work: call only from `spawn_blocking`
//! or a dedicated thread (AGENTS.md rule 2).
//!
//! An entry is absolute (`/…`, or `~/…` under the daemon's own `$HOME`); a relative one
//! is refused, at config load and again here. It is resolved without following any
//! link a run could have planted: each component is `lstat`ed, and a link inside a
//! place a run can write (the run's checkouts, the task temporary root, anthrex's data
//! directory, any other `cache_dirs` entry) or at the entry itself is refused. A link
//! elsewhere (`/tmp`, `/var`, one the user made in their own tree) is the system's or
//! the user's and is followed. The `$HOME` guards and the overlap checks then run on
//! the resolved path.

use std::path::{Component, Path, PathBuf};

/// The places a run can write, which a `cache_dirs` entry may neither run through (by
/// a link) nor overlap.
pub struct Areas<'a> {
    /// The run's checkouts' parent, `<wt>/runs/<run>` (resolved).
    pub checkouts: &'a Path,
    /// The task temporary root (`run::git::tmp_root`).
    pub tmp_root: &'a Path,
    /// anthrex's data directory (resolved).
    pub data: &'a Path,
    /// The repository's git common directory (resolved).
    pub common: &'a Path,
    /// The anthrex daemon's socket.
    pub daemon_socket: &'a Path,
}

/// Every entry of `raw`, resolved and checked; the first refusal is returned.
pub fn resolve_all(raw: &[String], areas: &Areas<'_>) -> Result<Vec<PathBuf>, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let lexical = raw
        .iter()
        .map(|entry| absolute(entry, home.as_deref()))
        .collect::<Result<Vec<_>, _>>()?;
    let fixed = [areas.checkouts, areas.tmp_root, areas.data];
    // First pass: each entry against the run's own areas.
    let resolved = raw
        .iter()
        .zip(&lexical)
        .map(|(entry, path)| walk(entry, path, &fixed))
        .collect::<Result<Vec<_>, _>>()?;
    // Second pass: no entry runs through a link inside another entry (which a check
    // granted that entry could have planted), and no two overlap.
    let mut all: Vec<&Path> = fixed.to_vec();
    all.extend(resolved.iter().map(PathBuf::as_path));
    for (i, (entry, path)) in raw.iter().zip(&lexical).enumerate() {
        walk(entry, path, &all)?;
        for (j, other) in resolved.iter().enumerate() {
            if i != j && overlaps(&resolved[i], other) {
                return Err(format!(
                    "cache_dirs entries {entry:?} and {:?} overlap; name each directory once",
                    raw[j]
                ));
            }
        }
    }
    let canonical_home = home.as_deref().map(canonical);
    for (entry, path) in raw.iter().zip(&resolved) {
        check(entry, path, areas, canonical_home.as_deref())?;
    }
    Ok(resolved)
}

/// `raw` as an absolute path: `~/…` under `home`; anything relative, or with `..`, is
/// refused.
fn absolute(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let path = if let Some(rest) = raw.strip_prefix("~/") {
        home.ok_or_else(|| format!("cache_dirs entry {raw:?}: $HOME is not set"))?
            .join(rest)
    } else if raw == "~" {
        home.ok_or_else(|| format!("cache_dirs entry {raw:?}: $HOME is not set"))?
            .to_path_buf()
    } else {
        PathBuf::from(raw)
    };
    if !path.is_absolute() {
        return Err(format!(
            "cache_dirs entry {raw:?} must be an absolute path (or start with ~/)"
        ));
    }
    if raw.trim().is_empty() || path.components().any(|c| c == Component::ParentDir) {
        return Err(format!(
            "cache_dirs entry {raw:?} must be a path without `..`"
        ));
    }
    Ok(path)
}

/// `path` resolved component by component with `lstat`: a link inside one of `areas`,
/// or at `path` itself, is refused; any other link is followed. The part that does not
/// exist yet is appended as it is.
fn walk(raw: &str, path: &Path, areas: &[&Path]) -> Result<PathBuf, String> {
    let mut resolved = PathBuf::from("/");
    let mut missing = false;
    let names: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    for (at, name) in names.iter().enumerate() {
        let next = resolved.join(name);
        if missing {
            resolved = next;
            continue;
        }
        match std::fs::symlink_metadata(&next) {
            Ok(meta) if meta.file_type().is_symlink() => {
                if at + 1 == names.len() {
                    return Err(format!(
                        "cache_dirs entry {raw:?} is itself a link ({}), which a check it was granted to could have made; name the directory it points to instead",
                        next.display()
                    ));
                }
                if let Some(area) = areas.iter().find(|a| next.starts_with(a)) {
                    return Err(format!(
                        "cache_dirs entry {raw:?} runs through the link {}, inside {}, which a run can write; name the directory it points to instead",
                        next.display(),
                        area.display()
                    ));
                }
                resolved = next
                    .canonicalize()
                    .map_err(|err| format!("cannot resolve {}: {err}", next.display()))?;
            }
            Ok(_) => resolved = next,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                missing = true;
                resolved = next;
            }
            Err(err) => return Err(format!("cannot read {}: {err}", next.display())),
        }
    }
    Ok(resolved)
}

/// The guards, on the resolved path: not `$HOME`, an ancestor of it, a dotfile
/// directory directly under it or `~/Library` itself; nothing of the run's own areas,
/// the git common directory, anthrex's data directory, or the daemon's socket.
fn check(raw: &str, path: &Path, areas: &Areas<'_>, home: Option<&Path>) -> Result<(), String> {
    if let Some(home) = home
        && let Some(why) = sensitive_reason(path, home)
    {
        return Err(format!(
            "cache_dirs entry {raw:?} resolves to {} ({why}); name a directory inside it instead",
            path.display()
        ));
    }
    for (what, other) in [
        ("the git common directory", areas.common),
        ("anthrex's data directory", areas.data),
        ("the task temporary directories", areas.tmp_root),
        ("the run's checkouts", areas.checkouts),
    ] {
        if overlaps(path, other) {
            return Err(format!(
                "profile cache_dirs entry {raw} ({}) overlaps {what} {}; a check may write nothing of it",
                path.display(),
                other.display()
            ));
        }
    }
    if areas.daemon_socket.starts_with(path) || canonical(areas.daemon_socket).starts_with(path) {
        return Err(format!(
            "cache_dirs entry {raw:?} ({}) holds the anthrex daemon's socket",
            path.display()
        ));
    }
    Ok(())
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// F1c round 3 (N3): why `path` is too sensitive to be a `cache_dirs` entry, given
/// `home`, or `None` when it is fine. Pure.
pub fn sensitive_reason(path: &Path, home: &Path) -> Option<&'static str> {
    if path == home || home.starts_with(path) {
        return Some("$HOME or an ancestor of it");
    }
    if path.parent() == Some(home) {
        let name = path.file_name().and_then(|n| n.to_str());
        if name.is_some_and(|n| n.starts_with('.')) {
            return Some("a dotfile directory directly under $HOME");
        }
        if name == Some("Library") {
            return Some("~/Library");
        }
    }
    None
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            for d in [
                "wt/runs/r1/t1",
                "tmproot",
                "data/runs/r1",
                "repo/.git",
                "caches/a",
            ] {
                std::fs::create_dir_all(root.join(d)).unwrap();
            }
            Fixture { _dir: dir, root }
        }

        fn resolve(&self, raw: &[&str]) -> Result<Vec<PathBuf>, String> {
            let r = &self.root;
            let (checkouts, tmp, data, common, socket) = (
                r.join("wt/runs/r1"),
                r.join("tmproot"),
                r.join("data"),
                r.join("repo/.git"),
                r.join("sock/d.sock"),
            );
            let raw: Vec<String> = raw.iter().map(|s| s.to_string()).collect();
            resolve_all(
                &raw,
                &Areas {
                    checkouts: &checkouts,
                    tmp_root: &tmp,
                    data: &data,
                    common: &common,
                    daemon_socket: &socket,
                },
            )
        }

        fn at(&self, rel: &str) -> String {
            self.root.join(rel).display().to_string()
        }
    }

    #[test]
    fn an_absolute_entry_resolves_and_a_missing_tail_is_kept() {
        let f = Fixture::new();
        let got = f
            .resolve(&[&f.at("caches/a"), &f.at("caches/new/deep")])
            .unwrap();
        assert_eq!(
            got,
            [f.root.join("caches/a"), f.root.join("caches/new/deep")]
        );
    }

    #[test]
    fn a_relative_entry_is_refused() {
        let f = Fixture::new();
        for raw in ["rel", ".cache/pip", "./x"] {
            let err = f.resolve(&[raw]).unwrap_err();
            assert!(err.contains("absolute"), "{raw}: {err}");
        }
    }

    #[test]
    fn a_link_a_run_could_plant_is_refused() {
        // R3: a checkout component swapped for a link to a directory outside.
        let f = Fixture::new();
        let target = f.root.join("outside/launchagents");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, f.root.join("wt/runs/r1/t1/.cache")).unwrap();
        let err = f.resolve(&[&f.at("wt/runs/r1/t1/.cache/pip")]).unwrap_err();
        assert!(err.contains("runs through the link"), "{err}");
        // Inside another entry (which a check granted it could have planted).
        std::os::unix::fs::symlink(&target, f.root.join("caches/a/b")).unwrap();
        let err = f
            .resolve(&[&f.at("caches/a"), &f.at("caches/a/b/c")])
            .unwrap_err();
        assert!(err.contains("link") || err.contains("overlap"), "{err}");
        // At the entry itself.
        std::os::unix::fs::symlink(&target, f.root.join("caches/leaf")).unwrap();
        let err = f.resolve(&[&f.at("caches/leaf")]).unwrap_err();
        assert!(err.contains("itself a link"), "{err}");
    }

    #[test]
    fn a_link_the_user_made_elsewhere_is_followed_and_the_result_checked() {
        let f = Fixture::new();
        std::os::unix::fs::symlink(f.root.join("caches"), f.root.join("mine")).unwrap();
        assert_eq!(
            f.resolve(&[&f.at("mine/a")]).unwrap(),
            [f.root.join("caches/a")]
        );
        // A user link into the data directory resolves there, and is refused.
        std::os::unix::fs::symlink(f.root.join("data"), f.root.join("todata")).unwrap();
        let err = f.resolve(&[&f.at("todata/x")]).unwrap_err();
        assert!(err.contains("overlaps anthrex's data directory"), "{err}");
    }

    #[test]
    fn overlaps_with_the_runs_areas_and_the_socket_are_refused() {
        let f = Fixture::new();
        for (raw, why) in [
            (f.at("repo"), "git common"),
            (f.at("data/runs/r2"), "data directory"),
            (f.at("tmproot/abc"), "temporary"),
            (f.at("wt/runs/r1/t2"), "checkouts"),
            (f.at("sock"), "socket"),
        ] {
            let err = f.resolve(&[&raw]).unwrap_err();
            assert!(err.contains(why), "{raw}: {err}");
        }
        let err = f
            .resolve(&[&f.at("caches"), &f.at("caches/a")])
            .unwrap_err();
        assert!(err.contains("overlap"), "{err}");
    }

    #[test]
    fn home_dotfiles_and_library_are_sensitive_but_named_subpaths_are_not() {
        let home = Path::new("/Users/me");
        for bad in [
            "/Users/me",
            "/Users",
            "/",
            "/Users/me/.ssh",
            "/Users/me/.gitconfig",
            "/Users/me/.claude",
            "/Users/me/Library",
        ] {
            assert!(
                sensitive_reason(Path::new(bad), home).is_some(),
                "{bad} was allowed"
            );
        }
        for ok in [
            "/Users/me/.cargo/registry",
            "/Users/me/Library/Caches/anthrex",
            "/Users/me/builds",
            "/tmp/cache",
        ] {
            assert!(
                sensitive_reason(Path::new(ok), home).is_none(),
                "{ok} was refused"
            );
        }
    }
}
