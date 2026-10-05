//! Milestone 9.6 task M9.6.16: the design flow's commands, driven as the real binary
//! against the run harness's isolated daemon (its own socket, data dir and config under
//! a temp dir; `fake-agent` as both runtimes; every other agent and `gh` pinned to
//! nonexistent paths). The design flow itself is task M9.6.20's end-to-end tests; this
//! pins the CLI's path: each command reaches the daemon, and the daemon's refusal is
//! printed as its own text, exit 1.

mod support;

use support::run_harness::RunHarness;
use support::run_plans::*;

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

/// `anthrex <args> --dir <repo>`.
fn anthrex(h: &RunHarness, args: &[&str]) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = args.to_vec();
    all.extend_from_slice(&["--dir", &repo]);
    let output = h.anthrex_input(&all, "");
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Exit 1 with `message` as the whole of stderr's last line, nothing on stdout.
fn refused_with(out: &Out, message: &str, what: &str) {
    assert_eq!(
        out.code, 1,
        "{what}: stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert_eq!(out.stderr.lines().last(), Some(message), "{what}");
    assert_eq!(out.stdout, "", "{what}");
}

/// `run start --plan … --design full` is the daemon's refusal (DF §1: a plan file never
/// runs the design flow), `--design off` starts the run, an unknown value is a usage
/// error; then every design command on that run reaches the daemon and prints its
/// refusal, `run <id> does not use the design flow`.
#[test]
fn design_commands_reach_the_daemon_and_print_its_refusals() {
    let h = RunHarness::new("");
    let plan = h.plan(&plan("", &[task("t1", &["a.txt"], "")]));
    let plan = plan.display().to_string();

    let out = anthrex(&h, &["run", "start", "--plan", &plan, "--design", "full"]);
    refused_with(
        &out,
        "the design flow runs only for planned code or docs goals; this goal is a plan file",
        "start --design full",
    );
    let out = anthrex(&h, &["run", "start", "--plan", &plan, "--design", "maybe"]);
    assert_eq!(out.code, 2, "a usage error: {}", out.stderr);

    let out = anthrex(&h, &["run", "start", "--plan", &plan, "--design", "off"]);
    assert_eq!(out.code, 0, "{}{}", out.stderr, h.log_tail());
    let id = out.stdout.trim().to_string();
    let h4 = &id[id.len() - 4..];

    let doc = h.repo.join("spec.md");
    std::fs::write(&doc, "# Reset\n").unwrap();
    let doc = doc.display().to_string();
    let not_design = format!("run {id} does not use the design flow");
    let commands: [&[&str]; 7] = [
        &["run", "approve", h4, "--gate", "spec"],
        &["run", "changes", h4, "--gate", "spec", "--note", "n"],
        &["run", "edit-doc", h4, "--gate", "spec", "--file", &doc],
        &["run", "rethink", h4, "--note", "n"],
        &["run", "back", h4, "--gate", "plan", "--note", "n"],
        &["run", "show", h4, "--doc", "spec"],
        &["run", "show", h4, "--doc", "plan", "--diff", "--findings"],
    ];
    for args in commands {
        let out = anthrex(&h, args);
        refused_with(&out, &not_design, &args.join(" "));
    }

    // The run is untouched: still at its plan gate, and its status has no design lines.
    let out = anthrex(&h, &["run", "status", h4]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("awaiting_approval"), "{}", out.stdout);
    assert!(!out.stdout.contains("design:"), "{}", out.stdout);
    assert!(!out.stdout.contains("waiting for you"), "{}", out.stdout);
}
