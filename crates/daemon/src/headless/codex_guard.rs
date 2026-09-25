//! M8a final fix batch F2 (review C, I1): Codex re-reads a checkout's `.codex/` on every
//! turn, because every turn is a new `codex exec` (or `exec resume`) process, and
//! nothing on its command line excludes project config (M8a.1 item 7a). Decision 53's
//! `run start` check sees only the base commit's tree. A worker could therefore write
//! `.codex/config.toml` in one turn and have its next turn (or a later task's session,
//! once the file is merged) load it: an MCP server, `notify`, a wider sandbox.
//!
//! So before every Codex process starts, its checkout's `.codex` must be exactly the
//! base commit's: the same files and links with the same contents, which `run start`
//! refused (or the user trusted with `--trust-project`). Anything else refuses the
//! launch, and the engine blocks the task as `blocked(environment)`. The comparison is
//! by git object id, so the base side needs only `git ls-tree`, and the checkout side
//! only this process's own no-follow reads: nothing in the checkout is run or followed.
//!
//! Empty directories are ignored: Codex loads files. File modes are not compared (only
//! file against link), so a checkout that did not keep an executable bit is not
//! refused.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The directory Codex reads project config from, relative to the checkout (M8a.1 item
/// 7a: `.codex/config.toml` and `.codex/hooks.json` are the paths it was seen to read;
/// the whole directory is guarded).
pub const CODEX_DIR: &str = ".codex";

/// More entries than this under `.codex` refuse the launch (invented: a real one has a
/// handful).
pub const MAX_ENTRIES: usize = 1000;
/// A file larger than this refuses the launch (invented).
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Which hash the repository names its objects with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectFormat {
    Sha1,
    Sha256,
}

impl ObjectFormat {
    /// The format of an object id: 40 hex digits for SHA-1, 64 for SHA-256.
    pub fn of(oid: &str) -> ObjectFormat {
        if oid.len() == 64 {
            ObjectFormat::Sha256
        } else {
            ObjectFormat::Sha1
        }
    }

    /// Git's blob id of `content` (`<hash>("blob <len>\0" + content)`), lower-case hex.
    pub fn blob_id(self, content: &[u8]) -> String {
        let header = format!("blob {}\0", content.len());
        let digest: Vec<u8> = match self {
            ObjectFormat::Sha1 => {
                let mut bytes = header.into_bytes();
                bytes.extend_from_slice(content);
                sha1(&bytes).to_vec()
            }
            ObjectFormat::Sha256 => {
                let mut hasher = Sha256::new();
                hasher.update(header.as_bytes());
                hasher.update(content);
                hasher.finalize().to_vec()
            }
        };
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// What an entry is: a file (any mode) or a symbolic link, whose content is its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Link,
    /// A gitlink (a submodule) in the base tree. A checkout never matches one, so a base
    /// tracking a submodule under `.codex` refuses every Codex launch.
    Gitlink,
}

/// One file or link under `.codex`: its path relative to the checkout, `/`-separated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GuardEntry {
    pub path: String,
    pub kind: EntryKind,
    pub oid: String,
}

/// The base commit's `.codex`, which every Codex session's checkout must match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexConfigGuard {
    pub format: ObjectFormat,
    /// Sorted by path.
    pub entries: Vec<GuardEntry>,
}

impl CodexConfigGuard {
    /// `Ok` when `checkout`'s `.codex` is the base's; otherwise the reason, naming each
    /// path that differs. Blocking (reads files): call it from a blocking thread.
    pub fn check(&self, checkout: &Path) -> Result<(), String> {
        let found = scan(checkout, self.format)?;
        let expected: BTreeMap<&str, &GuardEntry> =
            self.entries.iter().map(|e| (e.path.as_str(), e)).collect();
        let actual: BTreeMap<&str, &GuardEntry> =
            found.iter().map(|e| (e.path.as_str(), e)).collect();
        let mut differ: Vec<&str> = Vec::new();
        for (path, entry) in &actual {
            if expected.get(path) != Some(entry) {
                differ.push(path);
            }
        }
        for path in expected.keys() {
            if !actual.contains_key(path) {
                differ.push(path);
            }
        }
        if differ.is_empty() {
            return Ok(());
        }
        differ.sort();
        Err(mismatch(checkout, &differ.join(", ")))
    }
}

fn mismatch(checkout: &Path, what: &str) -> String {
    format!(
        "{} has Codex project config this run did not start with ({what}); Codex loads a \
         checkout's .codex on every turn, so no Codex session starts there. Revert it, \
         or route the task to Claude (decision 53). A tracked file the checkout converts \
         (a line-ending or filter attribute) differs from the base too",
        checkout.display()
    )
}

/// The files and links under `checkout/.codex`, sorted by path, never following a link.
pub fn scan(checkout: &Path, format: ObjectFormat) -> Result<Vec<GuardEntry>, String> {
    let mut entries = Vec::new();
    walk(checkout, CODEX_DIR, format, &mut entries)?;
    entries.sort();
    Ok(entries)
}

fn walk(
    checkout: &Path,
    rel: &str,
    format: ObjectFormat,
    out: &mut Vec<GuardEntry>,
) -> Result<(), String> {
    let path = checkout.join(rel);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(unreadable(checkout, rel, &error.to_string())),
    };
    if out.len() >= MAX_ENTRIES {
        return Err(mismatch(
            checkout,
            &format!("more than {MAX_ENTRIES} entries"),
        ));
    }
    let kind = meta.file_type();
    if kind.is_symlink() {
        let target = std::fs::read_link(&path)
            .map_err(|error| unreadable(checkout, rel, &error.to_string()))?;
        out.push(GuardEntry {
            path: rel.to_string(),
            kind: EntryKind::Link,
            oid: format.blob_id(target.as_os_str().as_encoded_bytes()),
        });
    } else if kind.is_dir() {
        let listing = std::fs::read_dir(&path)
            .map_err(|error| unreadable(checkout, rel, &error.to_string()))?;
        for child in listing {
            let child = child.map_err(|error| unreadable(checkout, rel, &error.to_string()))?;
            let Some(name) = child.file_name().to_str().map(str::to_string) else {
                return Err(mismatch(
                    checkout,
                    &format!("{rel}/<a name that is not UTF-8>"),
                ));
            };
            walk(checkout, &format!("{rel}/{name}"), format, out)?;
        }
    } else if kind.is_file() {
        out.push(GuardEntry {
            path: rel.to_string(),
            kind: EntryKind::File,
            oid: format.blob_id(&read_file(&path).map_err(|e| unreadable(checkout, rel, &e))?),
        });
    } else {
        let what = if kind.is_fifo() {
            "a FIFO"
        } else {
            "not a file"
        };
        return Err(mismatch(checkout, &format!("{rel} is {what}")));
    }
    Ok(())
}

fn unreadable(checkout: &Path, rel: &str, error: &str) -> String {
    format!(
        "could not check {} for Codex project config ({rel}: {error}); no Codex session \
         starts there",
        checkout.display()
    )
}

/// A regular file's bytes, opened without following a link and without blocking on a
/// FIFO swapped in after the `lstat`, and checked to be a regular file once open.
fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| error.to_string())?;
    let meta = file.metadata().map_err(|error| error.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".to_string());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("larger than {MAX_FILE_BYTES} bytes"));
    }
    let mut bytes = Vec::new();
    File::take(file, MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("larger than {MAX_FILE_BYTES} bytes"));
    }
    Ok(bytes)
}

/// The base commit's entries from `git ls-tree -r -z --full-tree <base> -- .codex`
/// output (`<mode> <type> <oid>\t<path>\0`...). A tree entry cannot appear under `-r`.
pub fn parse_ls_tree(listing: &str) -> Result<Vec<GuardEntry>, String> {
    let mut entries = Vec::new();
    for record in listing.split('\0').filter(|r| !r.is_empty()) {
        let (meta, path) = record
            .split_once('\t')
            .ok_or_else(|| format!("unexpected ls-tree record {record:?}"))?;
        let mut fields = meta.split(' ');
        let (mode, oid) = match (fields.next(), fields.next(), fields.next()) {
            (Some(mode), Some(_), Some(oid)) => (mode, oid),
            _ => return Err(format!("unexpected ls-tree record {record:?}")),
        };
        let kind = match mode {
            "120000" => EntryKind::Link,
            "160000" => EntryKind::Gitlink,
            _ => EntryKind::File,
        };
        entries.push(GuardEntry {
            path: path.to_string(),
            kind,
            oid: oid.to_string(),
        });
    }
    entries.sort();
    Ok(entries)
}

/// SHA-1 (FIPS 180-4), for git's SHA-1 object ids. Not used for anything else; the
/// ids it computes are compared with ids git computed, never trusted on their own.
fn sha1(message: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut data = message.to_vec();
    let bit_len = (message.len() as u64).wrapping_mul(8);
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    let (blocks, _) = data.as_chunks::<64>();
    for block in blocks {
        let mut w = [0u32; 80];
        let (words, _) = block.as_chunks::<4>();
        for (i, word) in words.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut out = [0u8; 20];
    let (chunks, _) = out.as_chunks_mut::<4>();
    for (chunk, word) in chunks.iter_mut().zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
#[path = "codex_guard_tests.rs"]
mod tests;
