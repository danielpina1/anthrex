//! Milestone 9.15: lessons. Task M9.15.2 added the model-free failure signatures
//! (decisions 5 and 6); task M9.15.3 the store, its I/O and its fold (decisions 2 to
//! 4); task M9.15.4 consolidation (decisions 7, 8, 17 and 25); task M9.15.6 selection
//! (decision 22) and the marker breaking of decision 11; task M9.15.8 the dossier and
//! its redaction (decision 11); task M9.15.10 the distill worker (decision 12).

pub mod consolidate;
pub mod dossier;
pub mod fold;
pub mod graph;
pub mod pending;
pub mod redact;
pub mod select;
pub mod service;
pub mod signature;
pub mod store;

/// The lessons store's file in a repository's data directory (decision 2).
pub const LESSONS_FILE: &str = "lessons.jsonl";
