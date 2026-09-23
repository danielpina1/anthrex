use super::*;

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn literal_prefix_stops_at_the_first_wildcard() {
    assert_eq!(literal_prefix("crates/proto/**"), vec!["crates", "proto"]);
    assert_eq!(literal_prefix("**/*.rs"), Vec::<&str>::new());
    assert_eq!(literal_prefix("src/a[0-9].rs"), vec!["src"]);
}

#[test]
fn intersection_is_prefix_containment() {
    assert!(intersects("crates/proto/**", "crates/proto/src/*.rs"));
    assert!(!intersects("crates/proto/**", "crates/tui/**"));
    assert!(intersects("**/*.rs", "crates/tui/**"));
    assert!(intersects("**/*.rs", "docs/x.md"));
    assert!(!any_intersect(&[], &[]));
}

/// Regression for review finding 1: `intersects` compares literal-prefix path
/// *components*, not their joined string. `crates/auth` is a string-prefix of
/// `crates/authz`, but `auth` and `authz` are different single path components, so
/// these must not be reported as intersecting.
#[test]
fn intersection_compares_path_components_not_string_prefixes() {
    assert!(!intersects("crates/auth/**", "crates/authz/**"));
    assert!(!intersects("crates/auth", "crates/authz/x.rs"));
}

#[test]
fn modules_spanned_cases() {
    let modules = s(&["crates/*"]);

    assert_eq!(
        modules_spanned(&s(&["crates/proto/**"]), &modules),
        ModuleSpan::One("crates/proto".to_string())
    );
    assert_eq!(
        modules_spanned(&s(&["crates/proto/src/a.rs", "crates/tui/**"]), &modules),
        ModuleSpan::Many
    );
    assert_eq!(
        modules_spanned(&s(&["crates/**"]), &modules),
        ModuleSpan::Many
    );
    assert_eq!(
        modules_spanned(&s(&["docs/x.md"]), &modules),
        ModuleSpan::One(".".to_string())
    );
    assert_eq!(
        modules_spanned(&s(&["crates/proto/**"]), &[]),
        ModuleSpan::One(".".to_string())
    );
}

#[test]
fn owns_without_wildcard_covers_the_directory_below_it() {
    let matcher = OwnsMatcher::new(&s(&["crates/auth"])).unwrap();
    assert!(matcher.matches("crates/auth/src/lib.rs"));
    assert!(!matcher.matches("crates/authz/lib.rs"));

    let matcher = OwnsMatcher::new(&s(&["crates/auth/"])).unwrap();
    assert!(matcher.matches("crates/auth/src/lib.rs"));
    assert!(!matcher.matches("crates/authz/lib.rs"));
}

#[test]
fn glob_matching_does_not_cross_separators() {
    let matcher = OwnsMatcher::new(&s(&["src/*.rs"])).unwrap();
    assert!(matcher.matches("src/a.rs"));
    assert!(!matcher.matches("src/a/b.rs"));
}

#[test]
fn absolute_and_parent_globs_are_invalid() {
    assert!(validate_glob("/etc/passwd").is_err());
    assert!(validate_glob("crates/../secret").is_err());
    assert!(validate_glob("   ").is_err());
    assert!(validate_glob("crates/proto/**").is_ok());
}

/// Review finding 4: a backslash makes `globset` treat the next character as escaped,
/// so intersection (which never looks at an actual path) and matching disagree about
/// what the glob means. Reject it outright, with a clear message.
#[test]
fn globs_with_a_backslash_are_invalid() {
    let err = validate_glob(r"crates\auth").expect_err("backslash must be rejected");
    assert!(
        err.contains('\\'),
        "message should mention the backslash: {err}"
    );
}

#[test]
fn inside_area_cases() {
    let area = s(&["crates/daemon/**"]);
    assert!(inside_area("crates/daemon/src/**", &area));
    assert!(!inside_area("crates/tui/**", &area));
    assert!(inside_area("crates/daemon", &area));
}

#[test]
fn names_literally_cases() {
    assert!(names_literally(&s(&["AGENTS.md"]), "AGENTS.md"));
    assert!(names_literally(&s(&["./AGENTS.md"]), "AGENTS.md"));
    assert!(!names_literally(&s(&["**"]), "AGENTS.md"));
    assert!(!names_literally(&s(&["*.md"]), "AGENTS.md"));
    assert!(!names_literally(&s(&[".claude/**"]), "AGENTS.md"));
    assert!(!names_literally(&s(&[".claude"]), ".claude/settings.json"));
    assert!(!names_literally(&s(&["docs/AGENTS.md"]), "AGENTS.md"));
}

#[test]
fn builtin_protected_matches_nested_instruction_files() {
    let matcher = OwnsMatcher::new(&s(&["**/AGENTS.md"])).unwrap();
    assert!(matcher.matches("AGENTS.md"));
    assert!(matcher.matches("docs/AGENTS.md"));

    let matcher = OwnsMatcher::new(&s(&[".claude/**"])).unwrap();
    assert!(matcher.matches(".claude/settings.json"));

    let matcher = OwnsMatcher::new(&s(&[".mcp.json"])).unwrap();
    assert!(!matcher.matches("x/.mcp.json"));
}

/// M8a.5 review finding 3: an empty literal prefix is a component-prefix of every
/// module pattern, so decision 9 says it spans more than one module.
#[test]
fn an_empty_literal_prefix_spans_every_module() {
    let modules = s(&["crates/*"]);
    assert_eq!(
        modules_spanned(&s(&["**/*.rs"]), &modules),
        ModuleSpan::Many
    );
    assert_eq!(modules_spanned(&s(&["**"]), &modules), ModuleSpan::Many);
    // With no modules configured there is nothing to span.
    assert_eq!(
        modules_spanned(&s(&["**/*.rs"]), &[]),
        ModuleSpan::One(".".to_string())
    );
}

/// M8a.5 review finding 4: a glob `globset` cannot compile is invalid at submit time.
#[test]
fn globs_that_do_not_compile_are_invalid() {
    assert_eq!(
        validate_glob("crates/a/src/[x.rs"),
        Err("is not a valid glob: unclosed character class; missing ']'".to_string())
    );
    assert_eq!(validate_glob("crates/a/src/[xy].rs"), Ok(()));
}

/// M8a.5 review finding 5: `./`, `//` and `.` components are rejected, not normalised,
/// because intersection compares components literally.
#[test]
fn dot_and_empty_components_are_invalid() {
    assert_eq!(
        validate_glob("./crates/a"),
        Err("must not contain .".to_string())
    );
    assert_eq!(
        validate_glob("crates/./a"),
        Err("must not contain .".to_string())
    );
    assert_eq!(
        validate_glob("crates//a"),
        Err("must not contain //".to_string())
    );
    assert_eq!(validate_glob("crates/a/"), Ok(()));
    assert_eq!(validate_glob("crates/.github/**"), Ok(()));
}
