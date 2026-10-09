//! The profile's files in a repository's data directory (decisions 4 and 7). Blocking:
//! callers run these on `spawn_blocking`, never under `daemon::lock`.
//!
//! | File | Content |
//! |------|---------|
//! | `profile.toml` | the confirmed `RepoProfile` |
//! | `profile.meta.json` | its `ProfileMeta` |
//! | `proposal.json` | the one pending `ProposalRecord` |
//!
//! Every write is [`write_atomic`]: a temp file of its own, `fsync`, rename, `fsync` of
//! the directory. A read never removes anything: a leftover of a crashed write looks
//! exactly like a concurrent writer's temp file, and readers ignore both. Leftovers are
//! removed only by [`sweep_leftovers`], where no writer can run (daemon start). Nothing
//! here ever writes inside the repository.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use proto::{ProfileMeta, ProposalRecord, RepoProfile};

pub const PROFILE_FILE: &str = "profile.toml";
pub const META_FILE: &str = "profile.meta.json";
pub const PROPOSAL_FILE: &str = "proposal.json";
/// The daemon's own record that detection work (and so its checkouts) may exist for a
/// project: written before the work makes a checkout, removed after the work has
/// discarded them. It outlives `proposal.json` when `profile reject` deletes that, so
/// a restart always knows the project to salvage into (task 11 re-review, C1).
pub const DETECTION_FILE: &str = "detection.json";

/// Decision 7: only this much of a file is hashed; its full length is still recorded.
pub const FINGERPRINT_MAX_BYTES: u64 = 4 << 20;

/// What a repository's data directory holds. Read once per request and matched at
/// once, so the size difference between its variants costs nothing; it keeps the
/// Interfaces' unboxed shape.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Stored {
    Found {
        profile: RepoProfile,
        meta: ProfileMeta,
        path: PathBuf,
    },
    /// `profile.toml` (or its meta) exists but does not parse: a run is refused
    /// (decision 6), never started with another profile.
    Unparseable {
        path: PathBuf,
        error: String,
    },
    Absent,
}

/// Milestone 9.10 decision 32: the most of an unreadable profile file's text the
/// status sends.
pub const UNREADABLE_TEXT_MAX: usize = 64 * 1024;
/// What ends a text cut at [`UNREADABLE_TEXT_MAX`].
const UNREADABLE_CUT: &str = "\n… (cut at 64 KiB)";

/// Decision 32: the text of the file `path` (the one [`Stored::Unparseable`] names),
/// at most [`UNREADABLE_TEXT_MAX`] bytes cut at a character boundary and then marked;
/// `None` when it cannot be read. Blocking; called only for an unparseable profile.
pub fn load_text(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| {
            file.take(UNREADABLE_TEXT_MAX as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .ok()?;
    let cut = bytes.len() > UNREADABLE_TEXT_MAX;
    if cut {
        bytes.truncate(UNREADABLE_TEXT_MAX);
        if let Err(error) = std::str::from_utf8(&bytes)
            && error.error_len().is_none()
        {
            bytes.truncate(error.valid_up_to());
        }
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if cut {
        text.push_str(UNREADABLE_CUT);
    }
    Some(text)
}

/// Makes every temp name of this process distinct.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp name no other write uses: `<file>.<pid>.<counter>.tmp`.
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    name.push(format!(".{}.{n}.tmp", std::process::id()));
    PathBuf::from(name)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// How many temp names [`write_atomic`] tries before giving up.
const TMP_TRIES: u32 = 64;

/// A temp file of this write's own, `create_new` with mode `0600`: a name some other
/// file already holds (a crashed process's leftover with a reused pid) is skipped for
/// the next counter, never opened or removed (ruling R-T4-2).
fn create_tmp(path: &Path) -> io::Result<(PathBuf, File)> {
    let mut last = None;
    for _ in 0..TMP_TRIES {
        let tmp = tmp_path(path);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no free temp name")))
}

/// Replaces `path` with `bytes` atomically and durably: a temp file of this write's own
/// (`create_new`, mode `0600`), `fsync`, rename, `fsync` of the directory. Concurrent
/// writes of one path never share a temp file, so each rename installs one whole
/// write. The directory is created when missing; on failure the temp file this call
/// created is removed, and no other.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let (tmp, mut file) = create_tmp(path)?;
    let written = (|| {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    sync_dir(dir)
}

/// Removes every leftover temp file of this module's files in `repo_dir` (a crashed
/// write's). Only where no writer can run: a concurrent write's temp file looks the
/// same. `ProfileService::restore` calls it at daemon start (M8b.11). A missing
/// directory has nothing to sweep.
pub fn sweep_leftovers(repo_dir: &Path) -> io::Result<()> {
    sweep_named(
        repo_dir,
        &[
            PROFILE_FILE,
            META_FILE,
            PROPOSAL_FILE,
            DETECTION_FILE,
            super::queue::QUEUE_FILE,
        ],
    )
}

/// [`sweep_leftovers`] for `files`' temp files (`<file>.<pid>.<n>.tmp`). Milestone 9.5
/// decision 36: the run restore sweeps the files the runs write before any restored op
/// can start a write (`run::driver::restore`).
pub fn sweep_named(repo_dir: &Path, files: &[&str]) -> io::Result<()> {
    let entries = match std::fs::read_dir(repo_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut removed = false;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let ours = (files.iter()).any(|file| name.starts_with(&format!("{file}.")));
        if ours && name.ends_with(".tmp") {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => removed = true,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
    }
    if removed {
        sync_dir(repo_dir)?;
    }
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// The confirmed profile of the repository whose data directory is `repo_dir`.
/// `profile.toml` is the commit point: without it there is no stored profile. A
/// missing meta file reads as a profile confirmed at 0 with no fingerprint.
pub fn load(repo_dir: &Path) -> Stored {
    let path = repo_dir.join(PROFILE_FILE);
    let meta_path = repo_dir.join(META_FILE);
    let text = match read_optional(&path) {
        Ok(Some(text)) => text,
        Ok(None) => return Stored::Absent,
        Err(error) => return Stored::Unparseable { path, error },
    };
    let profile = match toml::from_str::<RepoProfile>(&text) {
        Ok(profile) => profile,
        Err(e) => {
            return Stored::Unparseable {
                path,
                error: e.to_string().trim_end().to_string(),
            };
        }
    };
    let meta = match read_optional(&meta_path) {
        Ok(Some(text)) => match serde_json::from_str::<ProfileMeta>(&text) {
            Ok(meta) => meta,
            Err(e) => {
                return Stored::Unparseable {
                    path: meta_path,
                    error: e.to_string(),
                };
            }
        },
        Ok(None) => ProfileMeta {
            confirmed_at: 0,
            report: None,
            verification: None,
            fingerprint: BTreeMap::new(),
            edited_keys: Vec::new(),
            project: None,
        },
        Err(error) => {
            return Stored::Unparseable {
                path: meta_path,
                error,
            };
        }
    };
    Stored::Found {
        profile,
        meta,
        path,
    }
}

/// Stores a confirmed profile: the meta first, then `profile.toml`, whose rename is the
/// commit point.
pub fn save(repo_dir: &Path, profile: &RepoProfile, meta: &ProfileMeta) -> io::Result<()> {
    let toml = toml::to_string(profile).map_err(io::Error::other)?;
    let json = serde_json::to_vec_pretty(meta).map_err(io::Error::other)?;
    write_atomic(&repo_dir.join(META_FILE), &json)?;
    write_atomic(&repo_dir.join(PROFILE_FILE), toml.as_bytes())
}

/// The pending proposal, if any.
pub fn load_proposal(repo_dir: &Path) -> Result<Option<ProposalRecord>, String> {
    let path = repo_dir.join(PROPOSAL_FILE);
    match read_optional(&path)? {
        None => Ok(None),
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{} does not parse: {e}", path.display())),
    }
}

pub fn save_proposal(repo_dir: &Path, record: &ProposalRecord) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    write_atomic(&repo_dir.join(PROPOSAL_FILE), &json)
}

/// Records that detection work for `project` may leave checkouts ([`DETECTION_FILE`]).
pub fn save_detection(repo_dir: &Path, project: &Path) -> io::Result<()> {
    let json =
        serde_json::to_vec(&serde_json::json!({ "project": project })).map_err(io::Error::other)?;
    write_atomic(&repo_dir.join(DETECTION_FILE), &json)
}

/// The project [`DETECTION_FILE`] names, if it is there and reads.
pub fn load_detection(repo_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(repo_dir.join(DETECTION_FILE)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value["project"].as_str().map(PathBuf::from)
}

/// Removes [`DETECTION_FILE`]; nothing to remove is not an error.
pub fn delete_detection(repo_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(repo_dir.join(DETECTION_FILE)) {
        Ok(()) => sync_dir(repo_dir),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Deletes the pending proposal; nothing to delete is not an error.
pub fn delete_proposal(repo_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(repo_dir.join(PROPOSAL_FILE)) {
        Ok(()) => sync_dir(repo_dir),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// 64-bit FNV-1a, plain (no separators), fed incrementally. Milestone 9.1 hashes the
/// run's profile (decision 11) and the graph cache's manifests (decision 9) with it.
pub(crate) struct Fnv1a64(pub(crate) u64);

impl Fnv1a64 {
    pub(crate) fn new() -> Self {
        Fnv1a64(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// A path the fingerprint may look at: relative and never climbing out of the project.
fn inside(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

fn hash_hex(hash: &Fnv1a64, len: u64) -> String {
    format!("{:016x}:{len}", hash.0)
}

/// One file's fingerprint, never following a symlink (a model-written `manifests`
/// entry must not make the daemon read outside the repository):
/// - a regular file: `"<16 hex>:<len>"` of its contents;
/// - a symlink: `"link:<16 hex>:<len>"` of its target's name, never of what it points
///   at, so retargeting it still shows as a change;
/// - anything else is `"missing"`: absent, outside the project, under a symlinked
///   directory, not a regular file (a FIFO would block the read), or unreadable.
fn one(project: &Path, path: &str) -> String {
    const MISSING: &str = "missing";
    if !inside(path) {
        return MISSING.to_string();
    }
    let rel = Path::new(path);
    // Every directory on the way must be a real directory, not a link out.
    let mut dir = project.to_path_buf();
    if let Some(parent) = rel.parent() {
        for component in parent.components() {
            dir.push(component);
            match std::fs::symlink_metadata(&dir) {
                Ok(meta) if meta.file_type().is_dir() => {}
                _ => return MISSING.to_string(),
            }
        }
    }
    let full = project.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        return MISSING.to_string();
    };
    if meta.file_type().is_symlink() {
        let Ok(target) = std::fs::read_link(&full) else {
            return MISSING.to_string();
        };
        let bytes = target.as_os_str().as_bytes();
        let mut hash = Fnv1a64::new();
        hash.update(bytes);
        return format!("link:{}", hash_hex(&hash, bytes.len() as u64));
    }
    if !meta.file_type().is_file() {
        return MISSING.to_string();
    }
    // `O_NOFOLLOW`: a file swapped for a symlink since the check is refused, not read.
    let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&full)
    else {
        return MISSING.to_string();
    };
    let Ok(opened) = file.metadata() else {
        return MISSING.to_string();
    };
    if !opened.is_file() {
        return MISSING.to_string();
    }
    let mut hash = Fnv1a64::new();
    let mut reader = file.take(FINGERPRINT_MAX_BYTES);
    let mut buf = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hash.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return MISSING.to_string(),
        }
    }
    hash_hex(&hash, opened.len())
}

/// Decision 7: each path (relative to `project`) to its fingerprint.
pub fn fingerprint(project: &Path, paths: &[String]) -> BTreeMap<String, String> {
    paths
        .iter()
        .map(|path| (path.clone(), one(project, path)))
        .collect()
}

/// Every path in `meta.fingerprint` whose file changed, appeared or went missing since
/// the profile was confirmed, in path order.
pub fn stale(project: &Path, meta: &ProfileMeta) -> Vec<String> {
    meta.fingerprint
        .iter()
        .filter(|(path, recorded)| one(project, path) != **recorded)
        .map(|(path, _)| path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{TMP_COUNTER, write_atomic};

    fn temps(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        names.sort();
        names
    }

    /// Ruling R-T4-2: a temp name another file already holds is skipped, and that file
    /// is left exactly as it was.
    #[test]
    fn a_taken_temp_name_is_skipped_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proposal.json");
        let next = TMP_COUNTER.load(Ordering::Relaxed);
        // Other tests in this binary may take a few names meanwhile: hold many.
        let taken: Vec<_> = (next..next + 32)
            .map(|n| {
                let name = format!("proposal.json.{}.{n}.tmp", std::process::id());
                std::fs::write(dir.path().join(&name), "other").unwrap();
                name
            })
            .collect();
        write_atomic(&path, b"mine").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
        for name in &taken {
            assert_eq!(
                std::fs::read_to_string(dir.path().join(name)).unwrap(),
                "other",
                "{name}"
            );
        }
        assert_eq!(temps(dir.path()).len(), taken.len());
    }

    /// Ruling R-T4-2: a write that fails (here its rename, onto a non-empty directory)
    /// leaves no temp file behind.
    #[test]
    fn a_failed_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.toml");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inside"), "x").unwrap();
        assert!(write_atomic(&path, b"profile").is_err());
        assert_eq!(temps(dir.path()), Vec::<String>::new());
        assert!(path.join("inside").exists());
    }
}
