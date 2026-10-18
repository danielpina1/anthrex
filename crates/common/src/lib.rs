//! Pure helpers, two self-contained file helpers (`logfile`, `lockfile`) that both the
//! daemon and the client use, and `localtime`, the one reader of the local zone (for the
//! clients and `fake-agent`, never the daemon; milestone 9.19 decision 3). Depends on
//! `proto`, `libc` and `anyhow` only (`lockfile`'s errors; decision 10's 2026-10-11
//! ruling): never on the daemon, `config`, `tokio` or a PTY (decision 10).

pub mod bounds;
pub mod brakes_text;
pub mod branch;
pub mod glossary;
pub mod labels;
pub mod lifecycle;
pub mod localtime;
pub mod lockfile;
pub mod logfile;
pub mod numbers;
pub mod profile_keys;
pub mod progress;
pub mod refit_render;
pub mod refusal;
pub mod text;
pub mod worktree_names;
