//! Milestone 9.1 task M9.1.6: decision 32's failing-test names.

use super::FAILING_NAMES_MAX;
use super::failing::names;

fn of(output: &str) -> Vec<String> {
    names(output.lines().map(str::to_string))
}

#[test]
fn failing_names_from_libtest_nextest_pytest_and_go() {
    // Captured (M9.1.1 check 8): the `----` lines of the first `failures:` block are
    // not names; the indented lines of the second are.
    assert_eq!(
        of(include_str!("fixtures/libtest.txt")),
        vec!["tests::adds_two_and_two"]
    );
    // Captured: the node id ends before ` - `.
    assert_eq!(
        of(include_str!("fixtures/pytest.txt")),
        vec!["test_sample.py::test_adds_two_and_two"]
    );
    // Hand-written (ruling C-2): `FAIL` and `TIMEOUT` lines, the binary id left out;
    // the libtest line nextest echoes from the test's stdout names the same test.
    assert_eq!(
        of(include_str!("fixtures/nextest.txt")),
        vec!["tests::adds_two_and_two", "tests::waits_forever"]
    );
    // Hand-written: `--- FAIL:` lines, subtests included, not the package's `FAIL`.
    assert_eq!(
        of(include_str!("fixtures/go.txt")),
        vec!["TestAddsTwoAndTwo", "TestTable", "TestTable/negative"]
    );
    // A newer nextest's counter, and a retry's `TRY <n> FAIL`.
    assert_eq!(
        of(
            "        FAIL [   0.004s] (  2/100) crate::bin tests::x\n  TRY 2 FAIL [   0.004s] crate tests::y\n"
        ),
        vec!["tests::x", "tests::y"]
    );
    // Nothing that looks like a name: none, so the step is retried whole.
    assert_eq!(
        of("error[E0425]: cannot find value `x`\nFAILED\ntest result: FAILED.\n"),
        Vec::<String>::new()
    );
}

#[test]
fn failing_names_are_deduplicated_and_capped() {
    let mut output = String::new();
    for i in 0..60 {
        output.push_str(&format!("test t{i} ... FAILED\n"));
        output.push_str(&format!("test t{i} ... FAILED\n"));
    }
    output.push_str("\nfailures:\n    t0\n    t61\n\n");
    let got = of(&output);
    assert_eq!(got.len(), FAILING_NAMES_MAX);
    let want: Vec<String> = (0..50).map(|i| format!("t{i}")).collect();
    assert_eq!(got, want);
    // Deduplicated in order of first appearance.
    assert_eq!(
        of("test b ... FAILED\ntest a ... FAILED\n\nfailures:\n    a\n    b\n    c\n"),
        vec!["b", "a", "c"]
    );
}
