//! The final fix wave (A3): decision 14's last guard for `git`. Whether a `git` argv is
//! a push that could move a branch other than the run's own stage branches: a force (a
//! `+` refspec or any `--force*`/`-f`), a push of every ref (`--mirror`, `--all`,
//! `--branches`, `--tags`), `--prune`, a destination outside
//! `refs/heads/anthrex/<run>/stage-<n>` and preflight's `refs/heads/anthrex/preflight-…`
//! (which covers the base branch, whatever it is called), a refspec with no
//! destination, no refspec at all, or a delete of anything but a stage branch. A
//! superset of what the allow-list refuses: `GhHost` only ever pushes a full object id
//! to a stage branch, dry-runs to the preflight ref, and deletes a stage branch.

use crate::host::allow::stage_branch;

/// `git`'s global options that take the next argument as their value.
const GLOBAL_VALUE: [&str; 2] = ["-c", "-C"];

/// `git push`'s long options that take the next argument as their value (when not
/// glued with `=`).
const PUSH_VALUE: [&str; 4] = ["--push-option", "--repo", "--receive-pack", "--exec"];

/// The verb of the panic text, or `None` when `args` is not such a push.
pub(super) fn landing(args: &[&str]) -> Option<String> {
    let rest = push_args(args)?;
    let mut delete = false;
    let mut positional = Vec::new();
    let mut i = 0;
    while let Some(arg) = rest.get(i) {
        i += 1;
        if let Some(long) = arg.strip_prefix("--") {
            let name = long.split_once('=').map_or(long, |(n, _)| n);
            match name {
                "" => {
                    positional.extend(rest[i..].iter().copied());
                    break;
                }
                n if n.starts_with("force") => return Some("force-push".into()),
                "mirror" | "all" | "branches" | "tags" => return Some("push every ref".into()),
                "prune" => return Some("prune remote branches".into()),
                "delete" => delete = true,
                n if PUSH_VALUE.contains(&format!("--{n}").as_str()) && !long.contains('=') => {
                    i += 1;
                }
                _ => {}
            }
        } else if let Some(shorts) = arg.strip_prefix('-').filter(|s| !s.is_empty()) {
            for (at, c) in shorts.char_indices() {
                match c {
                    'f' => return Some("force-push".into()),
                    'd' => delete = true,
                    // `-o <option>`: the rest of the cluster, or the next argument.
                    'o' => {
                        if at + 1 == shorts.len() {
                            i += 1;
                        }
                        break;
                    }
                    _ => {}
                }
            }
        } else {
            positional.push(arg);
        }
    }
    let specs = positional.get(1..).unwrap_or_default();
    if specs.is_empty() {
        return Some("push without a refspec".into());
    }
    for spec in specs {
        if spec.starts_with('+') {
            return Some("force-push".into());
        }
        if delete {
            let dst = full(spec);
            if !is_stage(&dst) {
                return Some(format!("delete {dst}"));
            }
            continue;
        }
        let Some((src, dst)) = spec.split_once(':') else {
            return Some(format!("push {spec} without a destination"));
        };
        let dst = full(dst);
        if src.is_empty() {
            if !is_stage(&dst) {
                return Some(format!("delete {dst}"));
            }
            continue;
        }
        if !is_stage(&dst) && !is_preflight(&dst) {
            return Some(format!("push to {dst}"));
        }
    }
    None
}

/// The arguments after `push`, when `args` is a `git push`.
fn push_args<'a>(args: &'a [&'a str]) -> Option<&'a [&'a str]> {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if GLOBAL_VALUE.contains(arg) {
            i += 2;
        } else if arg.starts_with('-') {
            i += 1;
        } else {
            return (*arg == "push").then(|| &args[i + 1..]);
        }
    }
    None
}

/// A destination as git reads it: a short name is a branch.
fn full(dst: &str) -> String {
    if dst.starts_with("refs/") {
        dst.to_string()
    } else {
        format!("refs/heads/{dst}")
    }
}

/// `refs/heads/anthrex/<run>/stage-<n>` of any run (the allow-list holds it to this one).
fn is_stage(refname: &str) -> bool {
    refname
        .strip_prefix("refs/heads/")
        .and_then(stage_branch)
        .is_some()
}

fn is_preflight(refname: &str) -> bool {
    refname
        .strip_prefix("refs/heads/anthrex/preflight-")
        .is_some_and(|nonce| !nonce.is_empty() && !nonce.contains('/'))
}
