use super::*;

const METADATA: &str = include_str!("fixtures/metadata.json");

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn clean(profile: &str) -> String {
    format!("cargo clean --offline --profile {profile} -p 'app@0.1.0' -p 'vend@0.1.0'")
}

#[test]
fn path_packages_are_the_sourceless_ones() {
    // The F7 workspace (`exclude = ["vend"]`, so `vend` is a true non-member), with a
    // registry dependency (`cfg-if`) that must be left out.
    assert_eq!(
        path_packages(METADATA),
        Ok(strings(&["app@0.1.0", "vend@0.1.0"]))
    );
    assert!(path_packages("not json").is_err());
}

#[test]
fn the_plan_cleans_each_profile_dir_with_a_fingerprint() {
    let packages = strings(&["app@0.1.0", "vend@0.1.0"]);
    let entries = strings(&[
        "debug",
        "debug/.fingerprint",
        "release",
        "release/.fingerprint",
        "ci",
        "ci/.fingerprint",
        "tmp",
        "tmp/x",
    ]);
    assert_eq!(
        plan(&packages, &entries),
        CleanPlan::Commands(vec![clean("dev"), clean("release"), clean("ci")])
    );
}

#[test]
fn a_quote_in_a_spec_is_shell_quoted() {
    let packages = strings(&["it's@1.0.0"]);
    let entries = strings(&["debug", "debug/.fingerprint"]);
    assert_eq!(
        plan(&packages, &entries),
        CleanPlan::Commands(vec![
            "cargo clean --offline --profile dev -p 'it'\\''s@1.0.0'".to_string()
        ])
    );
}

#[test]
fn no_package_or_no_profile_dir_is_no_command() {
    let entries = strings(&["debug", "debug/.fingerprint"]);
    // `cargo clean` with no `-p` would clean everything.
    assert_eq!(plan(&[], &entries), CleanPlan::Commands(Vec::new()));
    let packages = strings(&["app@0.1.0"]);
    assert_eq!(plan(&packages, &[]), CleanPlan::Commands(Vec::new()));
}

#[test]
fn a_triple_dir_or_an_ambiguous_spec_is_a_wipe() {
    let packages = strings(&["app@0.1.0", "vend@0.1.0"]);
    let triple = strings(&[
        "debug",
        "debug/.fingerprint",
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu/debug",
    ]);
    assert_eq!(
        plan(&packages, &triple),
        CleanPlan::Wipe("a target triple directory".to_string())
    );
    let triple_release = strings(&["aarch64-apple-darwin", "aarch64-apple-darwin/release"]);
    assert_eq!(
        plan(&packages, &triple_release),
        CleanPlan::Wipe("a target triple directory".to_string())
    );
    let twice = strings(&["app@0.1.0", "app@0.1.0"]);
    let entries = strings(&["debug", "debug/.fingerprint"]);
    assert_eq!(
        plan(&twice, &entries),
        CleanPlan::Wipe("an ambiguous package spec".to_string())
    );
}
