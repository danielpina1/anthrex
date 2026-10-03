//! Milestone 9.3 task M9.3.8: `run iterate`, `run start --continue` and `anthrex ls`,
//! driven as the real binary against the run harness's isolated daemon (`fake-agent` as
//! both runtimes, every other agent and `gh` pinned to nonexistent paths). The rounds
//! and chains themselves are task M9.3.11's end-to-end tests; this pins the CLI's path:
//! the run resolved by a suffix, the request sent, the daemon's refusal printed as its
//! own text, exit 1.

mod support;

use support::run_harness::RunHarness;
use support::run_plans::*;

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

/// `anthrex <args> --dir <repo>` with `input` on stdin.
fn anthrex(h: &RunHarness, args: &[&str], input: &str) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = args.to_vec();
    all.extend_from_slice(&["--dir", &repo]);
    let output = h.anthrex_input(&all, input);
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Exit 1 with `message` as the whole of stderr's last line, nothing on stdout.
fn refused_with(out: &Out, message: &str) {
    assert_eq!(
        out.code, 1,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert_eq!(out.stderr.lines().last(), Some(message), "{}", out.stderr);
    assert_eq!(out.stdout, "");
}

/// A plan-file run has no orchestrator: `run iterate` (from an argument and from stdin)
/// and `run start --goal … --continue` are refused with the daemon's texts (decisions 9
/// and 22), and `anthrex ls` lists no idle orchestrator, so its output is the window
/// table alone.
#[test]
fn iterate_and_continue_print_the_daemons_refusals() {
    let h = RunHarness::new("");
    let plan = h.plan(&plan("", &[task("t1", &["a.txt"], "")]));
    let plan = plan.display().to_string();
    let out = anthrex(&h, &["run", "start", "--plan", &plan], "");
    assert_eq!(out.code, 0, "{}{}", out.stderr, h.log_tail());
    let id = out.stdout.trim().to_string();
    let h4 = &id[id.len() - 4..];

    let no_orchestrator = format!("run {h4} has no orchestrator; start a new goal for more work");
    let out = anthrex(&h, &["run", "iterate", h4, "also add b"], "");
    refused_with(&out, &no_orchestrator);
    let out = anthrex(&h, &["run", "iterate", h4, "-"], "also add b\n");
    refused_with(&out, &no_orchestrator);

    let out = anthrex(&h, &["run", "iterate", h4], "");
    assert_eq!(out.code, 2, "a usage error: {}", out.stderr);

    let out = anthrex(
        &h,
        &["run", "start", "--goal", "next", "--continue", h4],
        "",
    );
    refused_with(
        &out,
        &format!("run {h4} has no orchestrator to continue; start a new goal without --continue"),
    );

    let out = anthrex(&h, &["ls"], "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.starts_with("ID "), "{}", out.stdout);
    assert!(!out.stdout.contains("orchestrator  idle"), "{}", out.stdout);
    let out = anthrex(&h, &["ls", "--json"], "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    let windows: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert!(windows.is_array(), "{}", out.stdout);
}
