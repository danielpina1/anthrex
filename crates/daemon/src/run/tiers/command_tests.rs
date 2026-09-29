//! Milestone 9.1 task M9.1.6: decision 7's placeholders and decision 25's filters.
//! Pinning: `command.rs` was written in M9.1.5, which verification needed.

use super::command::{Placeholders, filter_expr, substitute};
use super::*;

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| x.to_string()).collect()
}

fn values() -> Placeholders {
    Placeholders {
        module: Some("core".into()),
        modules: Some(s(&["web", "core"])),
        filter: Some("not (slow)".into()),
        shard: Some((2, 4)),
        test: Some("a::b".into()),
    }
}

#[test]
fn placeholders_substitute_and_quote() {
    let v = values();
    // Decision 7's table, row by row.
    assert_eq!(substitute("t {module}", &v), "t 'core'");
    assert_eq!(substitute("t {modules}", &v), "t 'core' 'web'");
    assert_eq!(substitute("t {modules:-p %}", &v), "t -p 'core' -p 'web'");
    assert_eq!(substitute("t {filter:-E %}", &v), "t -E 'not (slow)'");
    assert_eq!(
        substitute("t --partition count:{shard}/{shards}", &v),
        "t --partition count:2/4"
    );
    assert_eq!(substitute("t --exact {test}", &v), "t --exact 'a::b'");
    // `%%` is a literal `%`, inside a template only.
    assert_eq!(substitute("t {filter:-m=%%%}", &v), "t -m=%'not (slow)'");
    assert_eq!(substitute("echo 100%% {module}", &v), "echo 100%% 'core'");
    // `${VAR}`, shell brace expansion and unknown names are left alone, even a
    // shell variable spelled like a placeholder.
    assert_eq!(
        substitute("t ${HOME} {a,b} {nope} ${module} {module}", &v),
        "t ${HOME} {a,b} {nope} ${module} 'core'"
    );
    // The first `}` ends a template.
    assert_eq!(substitute("t {filter:-E %}}", &v), "t -E 'not (slow)'}");
    // A name with a quote in it is quoted with M8a's quoting.
    let quoted = Placeholders {
        module: Some("it's".into()),
        modules: Some(s(&["o'k"])),
        test: Some("x'y".into()),
        ..Placeholders::default()
    };
    assert_eq!(substitute("t {module}", &quoted), r#"t 'it'\''s'"#);
    assert_eq!(substitute("t {modules:-p %}", &quoted), r#"t -p 'o'\''k'"#);
    assert_eq!(substitute("t {test}", &quoted), r#"t 'x'\''y'"#);
    // A value that is not given keeps its placeholder; `{filter:…}` is empty.
    assert_eq!(
        substitute(
            "t {module} {modules} {modules:-p %} {shard} {test} {filter:-E %}.",
            &Placeholders::default()
        ),
        "t {module} {modules} {modules:-p %} {shard} {test} ."
    );
}

#[test]
fn filter_is_empty_without_slow_or_timing() {
    let tiers = TierProfile::default();
    for scope in [Scope::Gate, Scope::Full] {
        for timing in [false, true] {
            assert_eq!(filter_expr(&tiers, scope, timing), None);
        }
    }
    let values = Placeholders {
        filter: filter_expr(&tiers, Scope::Gate, false),
        ..Placeholders::default()
    };
    assert_eq!(substitute("pytest {filter:-m %}", &values), "pytest ");
}

#[test]
fn filter_expressions_per_scope_are_exact() {
    let both = TierProfile {
        slow_tests: Some("package(anthrex)".into()),
        timing_tests: Some("test(/timing/)".into()),
        ..TierProfile::default()
    };
    let exact = |tiers: &TierProfile, scope, timing| filter_expr(tiers, scope, timing);
    // Decision 25's four.
    assert_eq!(
        exact(&both, Scope::Gate, false).as_deref(),
        Some("not (package(anthrex)) and not (test(/timing/))")
    );
    assert_eq!(
        exact(&both, Scope::Gate, true).as_deref(),
        Some("(test(/timing/)) and not (package(anthrex))")
    );
    assert_eq!(
        exact(&both, Scope::Full, false).as_deref(),
        Some("not (test(/timing/))")
    );
    assert_eq!(
        exact(&both, Scope::Full, true).as_deref(),
        Some("(test(/timing/))")
    );
    // A missing part drops its clause.
    let slow = TierProfile {
        timing_tests: None,
        ..both.clone()
    };
    assert_eq!(
        exact(&slow, Scope::Gate, false).as_deref(),
        Some("not (package(anthrex))")
    );
    assert_eq!(exact(&slow, Scope::Full, false), None);
    let timing = TierProfile {
        slow_tests: None,
        ..both
    };
    assert_eq!(
        exact(&timing, Scope::Gate, true).as_deref(),
        Some("(test(/timing/))")
    );
    assert_eq!(
        exact(&timing, Scope::Gate, false).as_deref(),
        Some("not (test(/timing/))")
    );
}

#[test]
fn an_empty_modules_template_is_refused() {
    let tiers = TierProfile {
        build_check: Some("make".into()),
        module_tests: Some("t {modules:}".into()),
        ..TierProfile::default()
    };
    let problems: Vec<String> = validate(&tiers, None, &s(&["mods/*"]))
        .into_iter()
        .map(|(key, message)| format!("{key}: {message}"))
        .collect();
    assert_eq!(
        problems,
        vec!["module_tests: {modules:<template>} needs a non-empty template"]
    );
}
