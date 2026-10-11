//! Pure helpers and two self-contained file helpers (`logfile`, `lockfile`) that both the
//! daemon and the client use. Depends on `proto` and `libc` only: never on the daemon,
//! `config`, `tokio` or a PTY (decision 10).
