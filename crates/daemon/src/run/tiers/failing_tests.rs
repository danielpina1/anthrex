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
        of(
            "test b ... FAILED\ntest a ... FAILED\n\nfailures:\n\n---- b stdout ----\n\nfailures:\n    a\n    b\n    c\n"
        ),
        vec!["b", "a", "c"]
    );
}

/// Ruling C-6: a list of names is never a strict subset of the failures.
#[test]
fn failing_names_keep_whole_names_and_every_failure_kind() {
    // A doc-test's name holds spaces.
    let doc = "running 2 tests
test src/lib.rs - f (line 3) ... FAILED
test src/lib.rs - g (line 9) ... ok

failures:

---- src/lib.rs - f (line 3) stdout ----
Test executable failed (exit status: 101).

failures:
    src/lib.rs - f (line 3)

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";
    assert_eq!(of(doc), vec!["src/lib.rs - f (line 3)"]);
    // A parametrised pytest id with a space, and one with ` - ` inside its brackets;
    // `ERROR` lines are failures too.
    let pytest = "=========================== short test summary info ============================
FAILED tests/test_x.py::test_a[a b] - AssertionError: no
FAILED tests/test_x.py::test_b[x - y] - AssertionError
ERROR tests/test_x.py::test_c - fixture 'db' not found
==================== 2 failed, 1 passed, 1 error in 0.10s =====================
";
    assert_eq!(
        of(pytest),
        vec![
            "tests/test_x.py::test_a[a b]",
            "tests/test_x.py::test_b[x - y]",
            "tests/test_x.py::test_c"
        ]
    );
    // nextest's other terminal statuses; SLOW and LEAK are not failures.
    let nextest = "        SLOW [> 60.000s] crate tests::slow
     SIGSEGV [   0.010s] crate tests::crash
       ABORT [   0.020s] crate tests::abort
     SIGABRT [   0.020s] crate tests::sigabrt
        LEAK [   0.003s] crate tests::leaky
   LEAK-FAIL [   0.003s] crate tests::leak_fail
        PASS [   0.003s] crate tests::slow
";
    assert_eq!(
        of(nextest),
        vec![
            "tests::crash",
            "tests::abort",
            "tests::sigabrt",
            "tests::leak_fail"
        ]
    );
}

#[test]
fn failing_names_are_none_when_they_may_be_incomplete() {
    // libtest's summary counts more failures than the names read.
    let libtest = "test a ... FAILED
test b ... FAILED
test result: FAILED. 0 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";
    assert_eq!(of(libtest), Vec::<String>::new());
    // Counts are summed over every test binary.
    let two_binaries = "test a ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";
    assert_eq!(of(two_binaries), Vec::<String>::new());
    // pytest's `failed` and `errors` together.
    let pytest = "FAILED t.py::a - x
ERROR t.py::b - y
= 1 failed, 2 errors in 0.10s =
";
    assert_eq!(of(pytest), Vec::<String>::new());
    // A failure line whose name cannot be read.
    let unreadable = "        FAIL [   0.004s] crate tests::x
        FAIL [   0.004s] crate
";
    assert_eq!(of(unreadable), Vec::<String>::new());
}

#[test]
fn a_test_printing_failures_is_not_read_as_the_name_list() {
    let output = "test tests::a ... FAILED

failures:

---- tests::a stdout ----
failures:
    bogus_name

failures:
    tests::a

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";
    assert_eq!(of(output), vec!["tests::a"]);
    // Without a `---- … stdout ----` block before it, a `failures:` list is not read.
    assert_eq!(of("failures:\n    bogus_name\n"), Vec::<String>::new());
}
