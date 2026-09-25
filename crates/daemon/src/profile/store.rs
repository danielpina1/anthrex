//! The profile's files in a repository's data directory (decisions 4 and 7). Blocking:
//! callers run these on `spawn_blocking`, never under `daemon::lock`.
//!
//! | File | Content |
//! |------|---------|
//! | `profile.toml` | the confirmed `RepoProfile` |
//! | `profile.meta.json` | its `ProfileMeta` |
//! | `proposal.json` | the one pending `ProposalRecord` |
//!
//! Every write is [`write_atomic`]: a temp file, `fsync`, rename, `fsync` of the
//! directory. A leftover temp file from a crash is never read, and [`load`] removes it.
//! Nothing here ever writes inside the repository.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use proto::{ProfileMeta, ProposalRecord, RepoProfile};

pub const PROFILE_FILE: &str = "profile.toml";
pub const META_FILE: &str = "profile.meta.json";
pub const PROPOSAL_FILE: &str = "proposal.json";

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

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Replaces `path` with `bytes` atomically and durably: `<path>.tmp` (mode `0600`,
/// truncating any leftover), `fsync`, rename, `fsync` of the directory. The directory
/// is created when missing.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = tmp_path(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    sync_dir(dir)
}

/// Removes a leftover temp file of `path`, if any; a failure only leaves it ignored.
fn remove_leftover(path: &Path) {
    let _ = std::fs::remove_file(tmp_path(path));
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
    remove_leftover(&path);
    remove_leftover(&meta_path);
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
    remove_leftover(&path);
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

/// Deletes the pending proposal; nothing to delete is not an error.
pub fn delete_proposal(repo_dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(repo_dir.join(PROPOSAL_FILE)) {
        Ok(()) => sync_dir(repo_dir),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// 64-bit FNV-1a, plain (no separators), fed incrementally.
struct Fnv1a64(u64);

impl Fnv1a64 {
    fn new() -> Self {
        Fnv1a64(0xcbf2_9ce4_8422_2325)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// A path the fingerprint may read: relative and never climbing out of the project.
fn inside(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// One file's fingerprint: `"<16 hex>:<len>"`, or `"missing"` for a file that is absent,
/// not a regular file (a FIFO would block the read), outside the project, or unreadable.
fn one(project: &Path, path: &str) -> String {
    const MISSING: &str = "missing";
    if !inside(path) {
        return MISSING.to_string();
    }
    let full = project.join(path);
    let Ok(meta) = std::fs::metadata(&full) else {
        return MISSING.to_string();
    };
    if !meta.is_file() {
        return MISSING.to_string();
    }
    let Ok(file) = File::open(&full) else {
        return MISSING.to_string();
    };
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
    format!("{:016x}:{}", hash.0, meta.len())
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
