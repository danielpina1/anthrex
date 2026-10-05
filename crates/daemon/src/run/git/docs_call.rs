//! The documents commit's git calls (`docs.rs`, split out move-only by the final fix
//! wave): each flagged for its mode, with [`PROTECT_FLAGS`](super::PROTECT_FLAGS) first,
//! within its timeout, the driver's index set on the index commands only.

use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

use super::super::{NO_HOOKS, WRITE_FLAGS, failure, os};
use super::PROTECT_FLAGS;
use crate::worktree::{GitOutput, run_git_head_tail, run_git_with_index};

/// The most an earlier spec, or one tree's listing, may be (ruling WB-B m1).
pub(super) const READ_CAP: usize = 1024 * 1024;

/// How a call is flagged: a read, a write, or a write in the caller's index.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Read,
    Write,
    Indexed,
}

pub(super) struct Call<'a> {
    pub(super) git: &'a OsStr,
    pub(super) timeout: Duration,
    pub(super) index: &'a Path,
}

impl Call<'_> {
    /// `args` with the mode's flags and [`PROTECT_FLAGS`] first.
    fn full<'b>(mode: Mode, args: &[&'b str]) -> Vec<&'b str> {
        let flags: &[&'static str] = match mode {
            Mode::Read => &NO_HOOKS,
            Mode::Write | Mode::Indexed => &WRITE_FLAGS,
        };
        let mut full: Vec<&str> = flags.iter().chain(&PROTECT_FLAGS).copied().collect();
        full.extend_from_slice(args);
        full
    }

    pub(super) fn run(
        &self,
        dir: &Path,
        mode: Mode,
        args: &[&str],
        input: Option<&[u8]>,
    ) -> Result<GitOutput, String> {
        let full: Vec<&OsStr> = Self::full(mode, args).into_iter().map(os).collect();
        let index = (mode == Mode::Indexed).then_some(self.index);
        let deadline = Instant::now() + self.timeout;
        run_git_with_index(self.git, dir, &full, deadline, input, index).map_err(|e| e.to_string())
    }

    /// A call that must succeed; its stdout.
    pub(super) fn ok(
        &self,
        dir: &Path,
        mode: Mode,
        args: &[&str],
        input: Option<&[u8]>,
    ) -> Result<String, String> {
        let output = self.run(dir, mode, args, input)?;
        match output.success {
            true => Ok(output.stdout),
            false => Err(self.failure(mode, args, &output)),
        }
    }

    /// Ruling WB-B m1: a read that must succeed, its stdout as raw bytes, never
    /// converted; `Err(total)` when it is over [`READ_CAP`].
    pub(super) fn bytes(&self, dir: &Path, args: &[&str]) -> Result<Result<Vec<u8>, u64>, String> {
        let full: Vec<&OsStr> = Self::full(Mode::Read, args).into_iter().map(os).collect();
        let deadline = Instant::now() + self.timeout;
        let (output, kept) = run_git_head_tail(self.git, dir, &full, deadline, READ_CAP, 0)
            .map_err(|e| e.to_string())?;
        if !output.success {
            return Err(failure(&full, &output));
        }
        match kept.dropped() {
            true => Ok(Err(kept.total)),
            false => Ok(Ok(kept.head)),
        }
    }

    pub(super) fn failure(&self, mode: Mode, args: &[&str], output: &GitOutput) -> String {
        let full: Vec<&OsStr> = Self::full(mode, args).into_iter().map(os).collect();
        failure(&full, output)
    }
}
