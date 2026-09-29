//! Milestone 9.1 task M9.1.6: decision 40's test-weakening signals from a `-U0` diff.

use super::weakening::{signals, signals_and_rest};
use super::*;

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| x.to_string()).collect()
}

const TEST_PATHS: &[&str] = &["crates/*/tests/**", "**/test_*.py"];
const MARKERS: &[&str] = &["#[ignore]", "@pytest.mark.skip"];

/// The shape `git diff -U0 --no-renames --no-color` prints.
const DIFF: &str = "\
diff --git a/crates/x/tests/old.rs b/crates/x/tests/old.rs
deleted file mode 100644
index 1111111..0000000
--- a/crates/x/tests/old.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-#[test]
-fn a() { assert!(true); }
-
diff --git a/crates/x/tests/t.rs b/crates/x/tests/t.rs
index 2222222..3333333 100644
--- a/crates/x/tests/t.rs
+++ b/crates/x/tests/t.rs
@@ -10,2 +9,0 @@ fn b() {
-    assert_eq!(a, 1);
-    assert_eq!(b, 2);
@@ -20,0 +19 @@ fn c() {
+#[ignore]
@@ -30 +29 @@ fn d() {
-    assert!(x);
+    assert!(x, \"why\");
diff --git a/crates/x/src/lib.rs b/crates/x/src/lib.rs
index 4444444..5555555 100644
--- a/crates/x/src/lib.rs
+++ b/crates/x/src/lib.rs
@@ -40 +40 @@ mod tests {
-        assert!(ok);
+        let _ = ok;
diff --git a/py/test_calc.py b/py/test_calc.py
index 6666666..7777777 100644
--- a/py/test_calc.py
+++ b/py/test_calc.py
@@ -5,2 +5,2 @@ def test_div():
-    with pytest.raises(ZeroDivisionError):
-        assert div(1, 0)
+    div(1, 1)
+    assert True
@@ -12,0 +13,2 @@ def test_add():
+
+@pytest.mark.skip(reason=\"flaky\")
diff --git a/web/app.test.ts b/web/app.test.ts
index 8888888..9999999 100644
--- a/web/app.test.ts
+++ b/web/app.test.ts
@@ -3 +3 @@
-  expect(total).toBe(3);
+  total;
";

#[test]
fn signals_deleted_test_file_skip_marker_and_assertion_loss() {
    let got = signals(
        DIFF,
        &s(&["crates/x/src/lib.rs"]),
        &s(&[TEST_PATHS, &["web/**"][..]].concat()),
        &s(MARKERS),
    );
    assert_eq!(
        got,
        vec![
            Signal::DeletedTestFile {
                path: "crates/x/tests/old.rs".into()
            },
            Signal::SkipMarker {
                path: "crates/x/tests/t.rs".into(),
                line: 19,
                marker: "#[ignore]".into()
            },
            // Three removed assertions, one added: the first removed one's old line.
            Signal::AssertionLoss {
                path: "crates/x/tests/t.rs".into(),
                line: 10,
                removed: 3,
                added: 1
            },
            // Not in test_paths, but `#[cfg(test)]` is in it at the head.
            Signal::AssertionLoss {
                path: "crates/x/src/lib.rs".into(),
                line: 40,
                removed: 1,
                added: 0
            },
            Signal::SkipMarker {
                path: "py/test_calc.py".into(),
                line: 14,
                marker: "@pytest.mark.skip".into()
            },
            // `.py` counts `pytest.raises` and `assert`: two removed, one added.
            Signal::AssertionLoss {
                path: "py/test_calc.py".into(),
                line: 5,
                removed: 2,
                added: 1
            },
            // `.ts` counts `expect(`.
            Signal::AssertionLoss {
                path: "web/app.test.ts".into(),
                line: 3,
                removed: 1,
                added: 0
            },
        ]
    );
    // No markers configured: no skip marker signals.
    let got = signals(DIFF, &[], &s(TEST_PATHS), &[]);
    assert!(
        got.iter()
            .all(|signal| !matches!(signal, Signal::SkipMarker { .. }))
    );
}

#[test]
fn signals_ignore_files_outside_test_paths_without_cfg_test() {
    let diff = "\
diff --git a/src/y.rs b/src/y.rs
deleted file mode 100644
index 1111111..0000000
--- a/src/y.rs
+++ /dev/null
@@ -1 +0,0 @@
-fn y() { assert!(true); }
diff --git a/crates/x/src/lib.rs b/crates/x/src/lib.rs
index 4444444..5555555 100644
--- a/crates/x/src/lib.rs
+++ b/crates/x/src/lib.rs
@@ -40 +40 @@
-        assert!(ok);
+        let _ = ok;
diff --git a/crates/x/tests/data.bin b/crates/x/tests/data.bin
deleted file mode 100644
index 1111111..0000000
Binary files a/crates/x/tests/data.bin and /dev/null differ
";
    // `lib.rs` has no `#[cfg(test)]` at the head, and `src/y.rs` matches no test path;
    // only the deleted binary under a test path is a signal.
    assert_eq!(
        signals(diff, &[], &s(TEST_PATHS), &s(MARKERS)),
        vec![Signal::DeletedTestFile {
            path: "crates/x/tests/data.bin".into()
        }]
    );
    assert_eq!(signals(diff, &[], &[], &s(MARKERS)), vec![]);
}

#[test]
fn signals_are_capped_at_twenty_deleted_first() {
    let mut diff = String::new();
    for i in 0..25 {
        diff.push_str(&format!(
            "diff --git a/crates/x/tests/t{i}.rs b/crates/x/tests/t{i}.rs\n\
             index 1111111..2222222 100644\n\
             --- a/crates/x/tests/t{i}.rs\n\
             +++ b/crates/x/tests/t{i}.rs\n\
             @@ -0,0 +1 @@\n\
             +#[ignore]\n"
        ));
    }
    for name in ["z1", "z2"] {
        diff.push_str(&format!(
            "diff --git a/crates/x/tests/{name}.rs b/crates/x/tests/{name}.rs\n\
             deleted file mode 100644\n\
             index 1111111..0000000\n\
             --- a/crates/x/tests/{name}.rs\n\
             +++ /dev/null\n\
             @@ -1 +0,0 @@\n\
             -fn z() {{}}\n"
        ));
    }
    let (got, rest) = signals_and_rest(&diff, &[], &s(TEST_PATHS), &s(MARKERS));
    assert_eq!(got.len(), SIGNALS_MAX);
    assert_eq!(rest, 7);
    assert_eq!(
        got[..2],
        [
            Signal::DeletedTestFile {
                path: "crates/x/tests/z1.rs".into()
            },
            Signal::DeletedTestFile {
                path: "crates/x/tests/z2.rs".into()
            },
        ]
    );
    assert_eq!(
        got[2],
        Signal::SkipMarker {
            path: "crates/x/tests/t0.rs".into(),
            line: 1,
            marker: "#[ignore]".into()
        }
    );
    assert_eq!(signals(&diff, &[], &s(TEST_PATHS), &s(MARKERS)), got);
}
