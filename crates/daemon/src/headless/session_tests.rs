//! Milestone 9.8 fix round 1 (M3): a spawn that found no program is marked
//! [`ProgramNotFound`]; one whose directory is gone (also `ENOENT`) is not, so the run
//! does not blame the role table's model for a vanished worktree. Never a real agent:
//! the program is a nonexistent path.

use std::ffi::OsStr;
use std::path::Path;

use proto::Runtime;

use super::*;

const NO_PROGRAM: &str = "/nonexistent/anthrex-test/claude";

fn refused_in(cwd: &Path) -> anyhow::Error {
    let spawned = HeadlessHandle::spawn(
        Runtime::Claude,
        OsStr::new(NO_PROGRAM),
        &[],
        cwd,
        &[],
        &[],
        |_, _| {},
    );
    match spawned {
        Ok(_) => panic!("a missing program is refused"),
        Err(error) => error,
    }
}

fn marked(error: &anyhow::Error) -> bool {
    error.chain().any(|e| e.is::<ProgramNotFound>())
}

#[test]
fn a_missing_program_is_marked_but_a_missing_directory_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let error = refused_in(tmp.path());
    assert!(marked(&error), "{error:#}");
    assert_eq!(error.to_string(), format!("could not start {NO_PROGRAM}"));
    let error = refused_in(&tmp.path().join("gone"));
    assert!(!marked(&error), "{error:#}");
    assert_eq!(error.to_string(), format!("could not start {NO_PROGRAM}"));
}
