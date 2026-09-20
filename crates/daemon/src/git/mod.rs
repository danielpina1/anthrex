//! Per-worktree git state: detection, probing, filesystem watching, publication.
//!
//! [`parse`] is the pure `porcelain=v2 -z` parser (M4.5.3) and [`probe`] is the
//! hardened `git` invocation that feeds it (M4.5.4). The watcher-backed registry
//! (M4.5.5+) lands in a later task; this module will grow its declaration then.

pub mod parse;
pub mod probe;
