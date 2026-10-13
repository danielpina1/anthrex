//! Milestone 9.15: lessons. Task M9.15.2 added the model-free failure signatures
//! (decisions 5 and 6); task M9.15.3 the store, its I/O and its fold (decisions 2 to
//! 4); task M9.15.4 consolidation (decisions 7, 8, 17 and 25); the distiller follows.

pub mod consolidate;
pub mod fold;
pub mod signature;
pub mod store;

/// The lessons store's file in a repository's data directory (decision 2).
pub const LESSONS_FILE: &str = "lessons.jsonl";
