//! Per-role scripts (M8a.20): `<git common dir>/fake-agent/<role>-<task>-<n>.jsonl`,
//! claimed in order of `n` with `create_new`, the state that lets a session continue its
//! script across processes, and the per-script argv and stdin logs.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// A bound on the one `git rev-parse` that finds the script directory.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// The script a process runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    /// Names the per-script `.args` and `.stdin` logs.
    pub name: String,
    /// `None`: no script at all (an empty one).
    pub path: Option<PathBuf>,
    /// Whether the script was claimed, so its position and variables persist.
    claimed: bool,
}

/// What a session carries from one process to the next: `capture` values and the last
/// `mcp_call` text (`FAKE_AGENT_RESULT`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Vars {
    pub captures: BTreeMap<String, String>,
    pub result: String,
}

impl Script {
    fn sidecar(&self, suffix: &str) -> Option<PathBuf> {
        let path = self.path.as_ref().filter(|_| self.claimed)?;
        let mut name = path.clone().into_os_string();
        name.push(suffix);
        Some(PathBuf::from(name))
    }

    /// The index of the next step (`<file>.pos`), 0 when unclaimed or new.
    pub fn pos(&self) -> usize {
        self.sidecar(".pos")
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0)
    }

    pub fn save_pos(&self, pos: usize) -> Result<()> {
        match self.sidecar(".pos") {
            Some(path) => replace(&path, pos.to_string().as_bytes()),
            None => Ok(()),
        }
    }

    pub fn vars(&self) -> Vars {
        self.sidecar(".vars")
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save_vars(&self, vars: &Vars) -> Result<()> {
        match self.sidecar(".vars") {
            Some(path) => {
                let bytes = serde_json::to_vec(vars).context("encode script variables")?;
                replace(&path, &bytes)
            }
            None => Ok(()),
        }
    }
}

/// Writes `bytes` to `<path>.tmp`, then renames it over `path`, so a kill mid-write
/// never leaves a truncated file for the next process to read as position 0.
fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, bytes).with_context(|| format!("write {}", Path::new(&tmp).display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))
}

/// The script for a session. A resumed session continues the script whose claim holds
/// its id; otherwise the role's first unclaimed script is claimed for it; otherwise
/// `FAKE_AGENT_SCRIPT`.
pub fn find(role: Option<&str>, task: Option<&str>, session: &str, resume: bool) -> Result<Script> {
    let dir = script_dir();
    if resume
        && let Some(dir) = &dir
        && let Some(path) = claimed_by(dir, session)?
    {
        return Ok(claimed(path));
    }
    if let (Some(role), Some(dir)) = (role, &dir)
        && let Some(path) = claim(dir, role, task, session)?
    {
        return Ok(claimed(path));
    }
    Ok(fallback())
}

fn claimed(path: PathBuf) -> Script {
    Script {
        name: stem(&path),
        path: Some(path),
        claimed: true,
    }
}

/// `FAKE_AGENT_SCRIPT`, never claimed.
pub fn fallback() -> Script {
    match std::env::var_os("FAKE_AGENT_SCRIPT") {
        Some(path) => {
            let path = PathBuf::from(path);
            Script {
                name: stem(&path),
                path: Some(path),
                claimed: false,
            }
        }
        None => Script {
            name: "fake-agent".into(),
            path: None,
            claimed: false,
        },
    }
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "fake-agent".into())
}

/// `<git common dir>/fake-agent`, when the cwd is in a repository that has one. A run's
/// task, review and proof checkouts are each their own repository (M8a final fix batch
/// F1c) whose only object alternate is the user's store: its scripts are then found
/// next to that store, in the user's repository.
fn script_dir() -> Option<PathBuf> {
    let common = git_common_dir().ok()??;
    let dir = common.join("fake-agent");
    if dir.is_dir() {
        return Some(dir);
    }
    let alternates = std::fs::read_to_string(common.join("objects/info/alternates")).ok()?;
    let objects = PathBuf::from(alternates.lines().next()?.trim());
    let dir = objects.parent()?.join("fake-agent");
    dir.is_dir().then_some(dir)
}

/// `git rev-parse --path-format=absolute --git-common-dir` in the cwd, with
/// `--no-optional-locks` and a scrubbed environment (AGENTS.md rule 11), bounded by
/// `GIT_TIMEOUT`. `None` outside a repository.
fn git_common_dir() -> Result<Option<PathBuf>> {
    let mut child = Command::new("git")
        .args(["--no-optional-locks", "rev-parse", "--path-format=absolute"])
        .arg("--git-common-dir")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn git rev-parse")?;
    let deadline = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().context("wait for git rev-parse")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("git rev-parse timed out after {GIT_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(5));
    };
    if !status.success() {
        return Ok(None);
    }
    let mut out = String::new();
    child
        .stdout
        .take()
        .context("git stdout was not piped")?
        .read_to_string(&mut out)
        .context("read git rev-parse")?;
    Ok(Some(PathBuf::from(out.trim())))
}

/// The script whose `.claimed` file holds `session`.
fn claimed_by(dir: &Path, session: &str) -> Result<Option<PathBuf>> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry.context("read script directory")?.path();
        let Some(name) = path.to_str() else { continue };
        let Some(script) = name.strip_suffix(".jsonl.claimed") else {
            continue;
        };
        if fs::read_to_string(&path).is_ok_and(|claim| claim.trim() == session) {
            return Ok(Some(PathBuf::from(format!("{script}.jsonl"))));
        }
    }
    Ok(None)
}

/// Claims `<role>-<task>-<n>.jsonl` with the smallest unclaimed `n` (`<role>-<n>.jsonl`
/// without a task) by creating `<file>.claimed` with `create_new` and writing `session`
/// into it.
fn claim(dir: &Path, role: &str, task: Option<&str>, session: &str) -> Result<Option<PathBuf>> {
    let prefix = match task {
        Some(task) => format!("{role}-{task}-"),
        None => format!("{role}-"),
    };
    let mut candidates: Vec<(u64, PathBuf)> = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry.context("read script directory")?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let n = name
            .strip_suffix(".jsonl")
            .and_then(|stem| stem.strip_prefix(&prefix))
            .and_then(|n| n.parse().ok());
        if let Some(n) = n {
            candidates.push((n, path));
        }
    }
    candidates.sort();
    for (_, path) in candidates {
        let mut name = path.clone().into_os_string();
        name.push(".claimed");
        match OpenOptions::new().write(true).create_new(true).open(&name) {
            Ok(mut file) => {
                file.write_all(session.as_bytes())
                    .context("write the claim")?;
                return Ok(Some(path));
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("claim a script"),
        }
    }
    Ok(None)
}

/// `FAKE_AGENT_ARGS_FILE`: a file gets the argv as one JSON array, overwritten per
/// process (milestone 3); a directory gets one line per process appended to
/// `<name>.args`.
pub fn record_args(name: &str, args: &[String]) -> Result<()> {
    let encoded = serde_json::to_string(args).context("encode argv")?;
    record("FAKE_AGENT_ARGS_FILE", name, "args", &encoded, true)
}

/// `FAKE_AGENT_STDIN_FILE`: every stdin line, appended to the file, or to
/// `<name>.stdin` in a directory.
pub fn record_stdin(name: &str, line: &str) -> Result<()> {
    record("FAKE_AGENT_STDIN_FILE", name, "stdin", line, false)
}

fn record(var: &str, name: &str, ext: &str, line: &str, overwrite_file: bool) -> Result<()> {
    let Some(target) = std::env::var_os(var).map(PathBuf::from) else {
        return Ok(());
    };
    let (path, append) = if target.is_dir() {
        (target.join(format!("{name}.{ext}")), true)
    } else {
        (target, !overwrite_file)
    };
    if !append {
        return fs::write(&path, line).with_context(|| format!("write {}", path.display()));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(format!("{line}\n").as_bytes())
        .with_context(|| format!("append to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{Vars, claimed};
    use std::os::unix::fs::MetadataExt;

    /// Ruling T20-m4: a kill mid-write must never leave a truncated `.pos` or `.vars`,
    /// so each is written to a temporary file and renamed over the old one.
    #[test]
    fn state_files_are_replaced_by_a_rename() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let script = claimed(dir.path().join("worker-t1-1.jsonl"));
        let pos = dir.path().join("worker-t1-1.jsonl.pos");
        let vars = dir.path().join("worker-t1-1.jsonl.vars");
        script.save_pos(1).unwrap();
        script.save_vars(&Vars::default()).unwrap();
        let (pos_inode, vars_inode) = (
            std::fs::metadata(&pos).unwrap().ino(),
            std::fs::metadata(&vars).unwrap().ino(),
        );

        script.save_pos(2).unwrap();
        let next = Vars {
            result: "done".into(),
            ..Vars::default()
        };
        script.save_vars(&next).unwrap();

        assert_ne!(std::fs::metadata(&pos).unwrap().ino(), pos_inode);
        assert_ne!(std::fs::metadata(&vars).unwrap().ino(), vars_inode);
        assert_eq!((script.pos(), script.vars()), (2, next));
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "no temporary file is left: {names:?}");
    }
}
