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

/// Ruling C-7's parser half: real outputs that used to give a strict subset.
#[test]
fn coloured_output_is_read() {
    let pytest = "\x1b[31mFAILED\x1b[0m tests/test_x.py::\x1b[1mtest_a\x1b[0m - AssertionError\r
\x1b[31m========== \x1b[31m\x1b[1m1 failed\x1b[0m, \x1b[32m1 passed\x1b[0m\x1b[31m in 0.01s\x1b[0m\x1b[31m ==========\x1b[0m\r
";
    assert_eq!(of(pytest), vec!["tests/test_x.py::test_a"]);
    let nextest = "        \x1b[31;1mFAIL\x1b[0m [   0.004s] \x1b[35;1mcrate\x1b[0m \x1b[36mtests::\x1b[0m\x1b[34;1mx\x1b[0m
     \x1b[35;1mTIMEOUT\x1b[0m [  60.004s] \x1b[35;1mcrate\x1b[0m \x1b[36mtests::\x1b[0m\x1b[34;1my\x1b[0m
";
    assert_eq!(of(nextest), vec!["tests::x", "tests::y"]);
    // Each mixed with libtest's plain output: the coloured failures are not lost.
    let libtest = "test tests::z ... FAILED\n";
    assert_eq!(
        of(&format!("{libtest}{pytest}")),
        vec!["tests::z", "tests/test_x.py::test_a"]
    );
    assert_eq!(
        of(&format!("{nextest}{libtest}")),
        vec!["tests::x", "tests::y", "tests::z"]
    );
    // A coloured summary still counts.
    let short = "\x1b[31mFAILED\x1b[0m t.py::a - x
\x1b[31m= \x1b[1m2 failed\x1b[0m\x1b[31m in 0.01s =\x1b[0m
";
    assert_eq!(of(short), Vec::<String>::new());
}

#[test]
fn a_long_pytest_run_is_counted() {
    // pytest adds `(h:mm:ss)` from 60 s on; mypy's plugin gives `.pyi` ids.
    let output = "FAILED stubs/x.pyi::mypy
FAILED tests/test_a.py::test_x - assert 1 == 2
========================= 2 failed in 65.12s (0:01:05) =========================
";
    assert_eq!(
        of(output),
        vec!["stubs/x.pyi::mypy", "tests/test_a.py::test_x"]
    );
    let more = "FAILED tests/test_a.py::test_x - assert 1 == 2
========================= 3 failed, 1 error in 65.12s (0:01:05) =========================
";
    assert_eq!(of(more), Vec::<String>::new());
}

#[test]
fn pytest_ids_may_name_any_file() {
    let output = "FAILED test_doc.txt::test_doc.txt
ERROR stubs/x.pyi::mypy - mypy-status
FAILED tests/test_a.py::test_x[a b] - boom
= 2 failed, 1 error in 0.50s =
";
    assert_eq!(
        of(output),
        vec![
            "test_doc.txt::test_doc.txt",
            "stubs/x.pyi::mypy",
            "tests/test_a.py::test_x[a b]"
        ]
    );
    // A line whose "path" has a space, or no `::` and no `.py`, is not pytest's.
    assert_eq!(
        of("ERROR could not connect::retry\nFAILED to start\n"),
        Vec::<String>::new()
    );
}

#[test]
fn a_crashed_test_binary_gives_no_names() {
    // One binary's failure is named; another aborted before printing any result.
    let named = "     Running unittests src/lib.rs (target/debug/deps/ax-1)

running 1 test
test a_fails ... FAILED

failures:

---- a_fails stdout ----
boom

failures:
    a_fails

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/d.rs (target/debug/deps/d-2)

running 1 test
test d_aborts ... ";
    let signal = format!(
        "{named}
error: test failed, to rerun pass `--lib`
error: test failed, to rerun pass `--test d`

Caused by:
  process didn't exit successfully: `/w/target/debug/deps/d-2` (signal: 6, SIGABRT: process abort signal)
"
    );
    assert_eq!(of(&signal), Vec::<String>::new());
    // Without the `Caused by` line: more failed targets than `test result: FAILED`.
    let counted = format!(
        "{named}
error: test failed, to rerun pass `--lib`
error: test failed, to rerun pass `--test d`
"
    );
    assert_eq!(of(&counted), Vec::<String>::new());
    let doctest = "test a ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
error: test failed, to rerun pass `--lib`
error: doctest failed, to rerun pass `--doc`
";
    assert_eq!(of(doctest), Vec::<String>::new());
    // One result, one error line: the ordinary case keeps its name.
    assert_eq!(
        of(include_str!("fixtures/libtest.txt")),
        vec!["tests::adds_two_and_two"]
    );
}

#[test]
fn a_go_package_that_failed_without_a_test_gives_no_names() {
    for (fixture, what) in [
        (include_str!("fixtures/go-build-failed.txt"), "build failed"),
        (include_str!("fixtures/go-timeout.txt"), "timeout"),
        (include_str!("fixtures/go-testmain.txt"), "TestMain exit"),
    ] {
        assert_eq!(of(fixture), Vec::<String>::new(), "{what}");
    }
    // Every failed package named a test: the names stand.
    assert_eq!(
        of(include_str!("fixtures/go.txt")),
        vec!["TestAddsTwoAndTwo", "TestTable", "TestTable/negative"]
    );
}
