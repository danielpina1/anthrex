//! The daemon's integration tests, one binary (milestone 9.12, decision 13). Each file
//! that was its own binary is a module here, so a test's name keeps the old binary's
//! name as its first segment. The files kept alone (decision 14) are their own `[[test]]`
//! entries in `crates/daemon/Cargo.toml`, each with its reason.

#[path = "../support/mod.rs"]
mod support;

mod agent_hooks;
mod conversation;
mod conversation_frame;
mod conversation_resume;
mod conversation_resume_races;
mod git_probe;
mod git_registry;
mod git_registry_arming;
mod git_schedule;
mod headless_sessions;
mod headless_turns;
mod headless_windows;
mod history_io;
mod history_reverts;
mod lifecycle;
mod manager;
mod manager_worktree;
mod otlp_server;
mod profile_store;
mod project;
mod reconcile_journal_pid;
#[cfg(target_os = "macos")]
mod run_confine;
#[cfg(target_os = "macos")]
mod run_confine_escape;
#[cfg(target_os = "macos")]
mod run_confine_network;
#[cfg(target_os = "macos")]
mod run_confine_protected;
mod run_exec;
mod run_gate_pins;
mod run_git;
mod run_git_accept;
mod run_git_accept_safety;
mod run_git_base_guard;
mod run_git_bwrap;
mod run_git_checkout;
mod run_git_crown;
mod run_git_docs;
mod run_git_done;
mod run_git_done_symlinks;
mod run_git_escape;
mod run_git_finish;
mod run_git_grant;
mod run_git_grant_link;
mod run_git_grant_shape;
mod run_git_handback;
mod run_git_handback_recovery;
mod run_git_identity;
mod run_git_import;
mod run_git_index;
mod run_git_legacy_config;
mod run_git_live_race;
mod run_git_merge;
mod run_git_merge_handback;
mod run_git_nested;
mod run_git_non_ascii;
mod run_git_owner_access;
mod run_git_planted;
mod run_git_pointers;
mod run_git_pseudo_refs;
mod run_git_reflogs;
mod run_git_review;
mod run_git_salvage_gone;
mod run_git_sandbox;
mod run_git_staging;
mod run_git_summary;
mod run_git_symref;
mod run_git_users_git;
mod run_git_worktrees;
mod run_journal;
mod run_journal_format;
mod run_proof_slots;
mod run_refresh;
mod server;
mod server_git;
mod server_restore_git;
mod server_runs;
mod server_winch;
mod source_rules;
mod subprocess;
mod test_rigs_pin_agents;
mod tuning_io;
mod window;
mod worktree;
