//! Milestone 9.13 decisions 8 and 10: gate words and their protection.

use proto::ProfileSpec;

use super::*;
use crate::run::plan::resolve_profile;

fn profile() -> Profile {
    resolve_profile(&ProfileSpec::default(), &ProfileSpec::default())
}

fn words_of(check: &str) -> Vec<String> {
    let mut p = profile();
    p.check = Some(check.to_string());
    gate_words(&p)
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn words_of_a_compound_command() {
    assert_eq!(
        words_of("cargo test && sh scripts/check.sh; python3 tools/lint.py | tee out.txt"),
        strings(&[
            "cargo",
            "test",
            "sh",
            "scripts/check.sh",
            "python3",
            "tools/lint.py",
            "tee",
            "out.txt",
        ])
    );
}

#[test]
fn env_prefix_and_dot_slash_are_stripped() {
    assert_eq!(
        words_of("RUST_LOG=debug ./scripts/t.sh"),
        strings(&["debug", "scripts/t.sh"])
    );
    assert_eq!(words_of("\"./a b.sh\""), strings(&["a", "b.sh"]));
    // `./` is removed repeatedly; a `NAME` that is not one keeps its `=`.
    assert_eq!(words_of("././x.sh 1A=b"), strings(&["x.sh", "1A=b"]));
}

#[test]
fn make_adds_its_makefile() {
    assert_eq!(
        words_of("make test"),
        strings(&["make", "test", "Makefile", "makefile", "GNUmakefile"])
    );
    assert_eq!(words_of("just ci"), strings(&["just", "ci", "justfile"]));
    assert_eq!(
        words_of("tox -e py"),
        strings(&["tox", "-e", "py", "tox.ini"])
    );
    assert_eq!(words_of("nox"), strings(&["nox", "noxfile.py"]));
    assert_eq!(words_of("npm test"), strings(&["npm", "test"]));
}

#[test]
fn absolute_and_variable_words_are_ignored() {
    let dropped = [
        "/usr/bin/env",
        "$HOME/x",
        "~/x",
        "src/*.rs",
        "{test}",
        "`pwd`",
        "../x",
        "a/../b",
    ];
    for word in dropped {
        assert_eq!(words_of(&format!("sh {word}")), strings(&["sh"]), "{word}");
    }
}

#[test]
fn every_command_key_is_read() {
    let mut p = profile();
    let k = |n: u8| Some(format!("sh k{n}.sh"));
    p.check = k(1);
    p.setup = k(2);
    p.single_test = k(3);
    p.tiers.build_check = k(4);
    p.tiers.module_test = k(5);
    p.tiers.module_tests = k(6);
    p.tiers.module_graph = GraphSource::Command("sh k7.sh".to_string());
    p.tiers.toolchain_id = k(8);
    assert_eq!(
        gate_words(&p),
        strings(&[
            "sh", "k1.sh", "k2.sh", "k3.sh", "k4.sh", "k5.sh", "k6.sh", "k7.sh", "k8.sh",
        ])
    );

    let mut cargo = profile();
    cargo.tiers.module_graph = GraphSource::Cargo;
    assert_eq!(gate_words(&cargo), Vec::<String>::new());
}

#[test]
fn at_most_64_words() {
    let command: Vec<String> = (0..100).map(|n| format!("w{n}")).collect();
    let words = words_of(&command.join(" "));
    assert_eq!(words.len(), GATE_WORDS_MAX);
    assert_eq!(GATE_WORDS_MAX, 64);
    assert_eq!(words, command[..64].to_vec());
}

/// Milestone 9.13 controller ruling: a word that names a directory pins nothing. A
/// trailing `/` is stripped, so `ls-tree` reports the directory as a tree, which is
/// not pinned (decision 9); a bare `.`, and a word that is only slashes, are dropped.
#[test]
fn a_directory_word_loses_its_trailing_slash() {
    assert_eq!(words_of("sh scripts/"), strings(&["sh", "scripts"]));
    assert_eq!(words_of("pytest tests//"), strings(&["pytest", "tests"]));
    assert_eq!(words_of("ruff check ."), strings(&["ruff", "check"]));
    assert_eq!(words_of("eslint . / // ./"), strings(&["eslint"]));
    assert_eq!(
        words_of("sh scripts/check.sh"),
        strings(&["sh", "scripts/check.sh"])
    );
}

#[test]
fn protect_appends_escaped_paths_once() {
    let mut protected = strings(&[".claude/**"]);
    let files = [
        PinnedFile {
            path: "scripts/check.sh".into(),
            blob: "a".repeat(40),
        },
        PinnedFile {
            path: "a[1].sh".into(),
            blob: "b".repeat(40),
        },
    ];
    protect(&mut protected, &files);
    assert_eq!(
        protected,
        strings(&[".claude/**", "scripts/check.sh", "a[[]1[]].sh"])
    );
    assert_eq!(protected[2], escape_path("a[1].sh"));
    protect(&mut protected, &files);
    assert_eq!(protected.len(), 3);
}
