//! The target pool (milestone 9.14 decisions 1–15). This task (M9.14.4) brings only
//! the pure book; the pool's I/O comes in M9.14.5 and its wiring in M9.14.6.

mod book;

pub use book::{
    DirId, DirState, LEASES_VERSION, Lease, LeasedDir, LeasesFile, PoolBook, PoolClass, PoolDir,
    Released, TARGET_DIR_VAR, applies, class_of,
};
