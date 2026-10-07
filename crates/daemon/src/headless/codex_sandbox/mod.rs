//! What a Codex session may touch ([`SandboxPlan`]) and how each generation of Codex's
//! sandbox configuration is told ([`CodexSandboxDialect`]). A new Codex generation is a
//! new variant, its own renderer file and one [`DIALECTS`] row; nothing else moves.

mod legacy;
mod profiles;

use std::path::PathBuf;
use std::sync::OnceLock;

pub const PROFILE_NAME: &str = "anthrex";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    ReadOnly,
    Confined,
    FullAccess,
}

impl Mode {
    /// `limits.worker_codex_sandbox` and the role constants' values; anything unknown
    /// is the narrowest mode.
    pub fn from_codex_sandbox(value: &str) -> Mode {
        match value {
            "workspace-write" => Mode::Confined,
            "danger-full-access" => Mode::FullAccess,
            _ => Mode::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPlan {
    pub mode: Mode,
    pub cwd: PathBuf,
    /// Writable beside `cwd`.
    pub write: Vec<PathBuf>,
    /// Read-only although inside a writable path.
    pub read_only: Vec<PathBuf>,
}

impl SandboxPlan {
    /// Drops a `write` entry equal to a `read_only` one: at equal specificity Codex lets
    /// write win, and a protected path must never become writable.
    pub fn new(
        mode: Mode,
        cwd: PathBuf,
        write: Vec<PathBuf>,
        read_only: Vec<PathBuf>,
    ) -> SandboxPlan {
        let write = write
            .into_iter()
            .filter(|w| !read_only.contains(w))
            .collect();
        SandboxPlan {
            mode,
            cwd,
            write,
            read_only,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CodexSandboxDialect {
    Legacy,
    Profiles,
}

/// Ascending; the last row whose version is ≤ the CLI's wins.
pub const DIALECTS: &[((u64, u64, u64), CodexSandboxDialect)] = &[
    ((0, 0, 0), CodexSandboxDialect::Legacy),
    ((0, 160, 0), CodexSandboxDialect::Profiles),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub &'static str);

impl CodexSandboxDialect {
    /// `None` (the probe failed or has not answered) is the newest dialect.
    pub fn for_version(version: Option<(u64, u64, u64)>) -> CodexSandboxDialect {
        let Some(version) = version else {
            return DIALECTS[DIALECTS.len() - 1].1;
        };
        DIALECTS
            .iter()
            .rev()
            .find(|(min, _)| *min <= version)
            .map(|(_, d)| *d)
            .unwrap_or(DIALECTS[0].1)
    }

    pub fn expresses_read_only(self) -> bool {
        matches!(self, CodexSandboxDialect::Profiles)
    }

    pub fn render(
        self,
        plan: &SandboxPlan,
        resuming: bool,
        resume_takes_sandbox: bool,
    ) -> Result<Vec<String>, Unsupported> {
        match self {
            CodexSandboxDialect::Legacy => legacy::render(plan, resuming, resume_takes_sandbox),
            CodexSandboxDialect::Profiles => Ok(profiles::render(plan)),
        }
    }
}

/// How a `CliCaps` picks its dialect: production reads the startup probe; tests fix one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialectChoice {
    Detected,
    Fixed(CodexSandboxDialect),
}

impl DialectChoice {
    pub fn resolve(self) -> CodexSandboxDialect {
        self.resolve_with(recorded_version())
    }

    pub fn resolve_with(self, version: Option<(u64, u64, u64)>) -> CodexSandboxDialect {
        match self {
            DialectChoice::Detected => CodexSandboxDialect::for_version(version),
            DialectChoice::Fixed(dialect) => dialect,
        }
    }
}

static VERSION: OnceLock<(u64, u64, u64)> = OnceLock::new();

/// The startup probe's answer (`lifecycle::codex_version`); the first call wins.
pub fn record_version(version: (u64, u64, u64)) {
    let _ = VERSION.set(version);
}

pub fn recorded_version() -> Option<(u64, u64, u64)> {
    VERSION.get().copied()
}

#[cfg(test)]
mod tests;
