//! Pure helpers and two self-contained file helpers (`logfile`, `lockfile`) that both the
//! daemon and the client use. Depends on `proto` and `libc` only: never on the daemon,
//! `config`, `tokio` or a PTY (decision 10).

pub mod bounds;
pub mod branch;
pub mod labels;
pub mod lifecycle;
pub mod lockfile;
pub mod logfile;
pub mod profile_keys;
pub mod refit_render;
pub mod refusal;
pub mod text;
pub mod worktree_names;
