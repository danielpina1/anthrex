//! Per-worktree git state: detection, probing, filesystem watching, publication.
//!
//! At this stage only [`parse`] exists: the pure `porcelain=v2 -z` parser (M4.5.3). The
//! hardened probe (M4.5.4) and the watcher-backed registry (M4.5.5+) land in later
//! tasks; this module will grow their declarations then.

pub mod parse;
