//! `exec.rs`'s unit tests, moved out to keep it under the 600-line rule (milestone
//! 9.1 task M9.1.9's preparatory move; no behaviour change).

use super::*;
use std::ffi::OsStr;

fn tail_of(chunks: &[&[u8]]) -> String {
    let mut sink = LineTail::new(None);
    for chunk in chunks {
        sink.push(chunk);
    }
    sink.finish()
}

#[test]
fn lines_split_across_reads_are_joined() {
    assert_eq!(tail_of(&[b"ab", b"c\nd", b"e\n"]), "abc\nde");
    assert_eq!(tail_of(&[b"a\n\nb"]), "a\n\nb");
    assert_eq!(tail_of(&[b""]), "");
}

#[test]
fn a_character_split_across_reads_is_kept_whole() {
    let wide = "世".as_bytes();
    assert_eq!(tail_of(&[&wide[..1], &wide[1..], b"\n"]), "世");
}

#[test]
fn a_match_beyond_the_cut_counts() {
    let pattern = Regex::new("PASS t").unwrap();
    let mut sink = LineTail::new(Some(&pattern));
    let mut line = vec![b'x'; 1000];
    line.extend_from_slice(b"PASS t\n");
    sink.push(&line);
    assert!(sink.matched);
    assert_eq!(sink.finish().len(), LINE_MAX_CHARS);
}

#[test]
fn a_pid_that_is_not_our_child_is_gone() {
    // pid 1 is never this process's child: `waitid` gives `ECHILD`.
    assert_eq!(leader_state(1), Leader::Gone);
}

#[test]
fn summary_of_an_empty_tail_is_empty() {
    assert_eq!(summary(""), "");
}

#[test]
fn engine_env_is_applied_to_the_command() {
    let mut command = Command::new("env");
    let a = [("A".to_string(), "1".to_string())];
    let b = [("A".to_string(), "2".to_string())];
    engine_env(&mut command, (&a, &b), false);
    let envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("CLAUDECODE"), None)));
    assert!(envs.contains(&(OsStr::new("ANTHREX_WINDOW_ID"), None)));
    assert!(envs.contains(&(OsStr::new("GIT_DIR"), None)));
    // Milestone 9.1 decision 26: `extra` is set after `env`, so it wins.
    assert!(
        envs.contains(&(OsStr::new("A"), Some(OsStr::new("2")))),
        "{envs:?}"
    );
    // T14-P1 (F4): `GIT_NO_REPLACE_OBJECTS` is for the engine's own git calls; a
    // check, proof or `setup` runs the project's commands with the user's git.
    assert!(
        !envs
            .iter()
            .any(|(name, _)| *name == OsStr::new("GIT_NO_REPLACE_OBJECTS")),
        "{envs:?}"
    );
    // Unconfined, other `ANTHREX_*` variables are left alone.
    assert!(!envs.contains(&(OsStr::new("ANTHREX_SOCKET"), None)));

    // F1c round 3 (N1): a confined command loses `ANTHREX_SOCKET`/`ANTHREX_DATA_DIR`.
    let mut confined = Command::new("env");
    engine_env(&mut confined, (&[], &[]), true);
    let confined_envs: Vec<(&OsStr, Option<&OsStr>)> = confined.get_envs().collect();
    assert!(confined_envs.contains(&(OsStr::new("ANTHREX_SOCKET"), None)));
    assert!(confined_envs.contains(&(OsStr::new("ANTHREX_DATA_DIR"), None)));
}
