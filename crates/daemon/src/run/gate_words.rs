//! Milestone 9.13 decisions 8 and 10: the words of a profile's gate commands, which
//! name the tracked files a run's own gate runs, and how those files join the run's
//! protected list. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2). The files are read at run start by
//! `driver::gate_files::read`.

use serde::{Deserialize, Serialize};

use super::globs::escape_path;
use super::model::Profile;
use super::tiers::GraphSource;

/// The most words [`gate_words`] returns (decision 8).
pub const GATE_WORDS_MAX: usize = 64;

/// A tracked file the run's gate runs, with its blob id at the run's base commit
/// (decision 9). Unrelated to `crate::worktree::pinned`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct PinnedFile {
    pub path: String,
    pub blob: String,
}

/// The files a program reads without naming them, appended when a word equals the
/// program (decision 8). Nothing for `npm`, `yarn` or `pnpm` (U3).
const IMPLICIT: [(&str, &[&str]); 4] = [
    ("make", &["Makefile", "makefile", "GNUmakefile"]),
    ("just", &["justfile"]),
    ("tox", &["tox.ini"]),
    ("nox", &["noxfile.py"]),
];

/// Decision 8: the words of `check`, `setup`, `single_test`, `tiers.build_check`,
/// `tiers.module_test`, `tiers.module_tests`, a command `tiers.module_graph` and
/// `tiers.toolchain_id`, in that order; each once, first seen first, then the implicit
/// files of the programs among them; at most [`GATE_WORDS_MAX`].
pub fn gate_words(profile: &Profile) -> Vec<String> {
    let tiers = &profile.tiers;
    let graph = match &tiers.module_graph {
        GraphSource::Command(command) => Some(command),
        GraphSource::None | GraphSource::Cargo => None,
    };
    let commands = [
        profile.check.as_ref(),
        profile.setup.as_ref(),
        profile.single_test.as_ref(),
        tiers.build_check.as_ref(),
        tiers.module_test.as_ref(),
        tiers.module_tests.as_ref(),
        graph,
        tiers.toolchain_id.as_ref(),
    ];
    let mut words: Vec<String> = Vec::new();
    let mut implicit: Vec<&str> = Vec::new();
    for word in commands
        .into_iter()
        .flatten()
        .flat_map(|c| command_words(c))
    {
        if let Some((_, files)) = IMPLICIT.iter().find(|(program, _)| *program == word) {
            implicit.extend(files.iter());
        }
        if !words.contains(&word) {
            words.push(word);
        }
    }
    for file in implicit {
        if !words.iter().any(|w| w == file) {
            words.push(file.to_string());
        }
    }
    words.truncate(GATE_WORDS_MAX);
    words
}

/// One command's kept words, in order (decision 8's split, cleaning and drops).
fn command_words(command: &str) -> Vec<String> {
    command
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '<' | '>'))
        .filter_map(clean)
        .collect()
}

/// A raw word as decision 8 keeps it, or `None` when it is dropped. A trailing `/` is
/// stripped and a bare `.` dropped (controller ruling, 9.13), so a directory word
/// pins nothing.
fn clean(raw: &str) -> Option<String> {
    let unquoted: String = raw.chars().filter(|c| !matches!(c, '\'' | '"')).collect();
    let mut word = strip_assignment(&unquoted);
    while let Some(rest) = word.strip_prefix("./") {
        word = rest;
    }
    // Controller ruling (9.13): a word naming a directory pins nothing. Without its
    // trailing `/`, `ls-tree` reports a directory as a tree, which `read` skips; a
    // bare `.` would list the root's files, so it is dropped.
    let absolute = word.starts_with('/');
    word = word.trim_end_matches('/');
    let dropped = word.is_empty()
        || word == "."
        || word.ends_with("/.")
        || absolute
        || word.starts_with('~')
        || word.contains(['$', '*', '?', '[', ']', '{', '}', '`'])
        || word.split('/').any(|part| part == "..");
    (!dropped).then(|| word.to_string())
}

/// `word` without a leading `NAME=` (`[A-Za-z_][A-Za-z0-9_]*=`); the value stays.
fn strip_assignment(word: &str) -> &str {
    let Some((name, value)) = word.split_once('=') else {
        return word;
    };
    let mut chars = name.chars();
    let starts = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if starts && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        value
    } else {
        word
    }
}

/// Decision 10: appends `escape_path(path)` for each gate file `protected` does not
/// already list.
pub fn protect(protected: &mut Vec<String>, files: &[PinnedFile]) {
    for file in files {
        let entry = escape_path(&file.path);
        if !protected.contains(&entry) {
            protected.push(entry);
        }
    }
}

#[cfg(test)]
#[path = "gate_words_tests.rs"]
mod tests;
