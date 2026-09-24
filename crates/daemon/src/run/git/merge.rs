//! The merge candidate of decision 36 and the ref reads of decisions 20 and 21: a
//! `merge-tree` of the run head and the task head, the candidate commit, the
//! compare-and-swap that advances the run branch, the integration worktree's
//! detach-and-reattach around the candidate's check, the conflict hand-back into the
//! task worktree, and the guard that classifies the run and base refs. No write in a
//! worktree updates a ref through its `HEAD` (final fix batch F1, fix round 3).
//!
//! Blocking; call only from `spawn_blocking`. Writes (`commit_tree`, `materialize`,
//! `cas_update`, `reattach`, `hand_back`) carry decision 18's flags through
//! [`Git::write`] and belong behind the caller's [`super::GitQueue::write`]; reads
//! (`merge_tree`, `read_ref`, `guard_refs`, `commits_since`) do not.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::worktrees::is_ancestor;
use super::{DIFF_FLAGS, Git, LARGE_OUTPUT_BYTES, failure, nul_fields, os};

/// Decision 20: at most this many of the base's new commits are listed at accept.
pub const ACCEPT_LIST_MAX: usize = 50;

/// Decision 36 step 2: a clean `merge-tree` gives the merged tree; a conflicted one the
/// files it could not merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CandidateStep {
    Tree(String),
    Conflict(Vec<String>),
}

/// Decision 21's classification of the run and base refs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefCheck {
    Ok,
    BaseAdvanced { to: String, commits: u32 },
    Halt { reason: String },
}

/// What [`super::accept`] did. `Conflict`: `git merge --abort` already ran, and the
/// base branch and `root` are as they were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcceptOutcome {
    Merged { commit: String },
    Conflict { files: Vec<String> },
}

/// What [`hand_back`] did (ruling T14-C1): `onto` is the task branch's tip it merged
/// the run head onto (the worktree's `HEAD` before the merge), `head` the worktree's
/// `HEAD` after it (the merge commit when clean, `onto` when conflicted), and `files`
/// the unmerged paths (empty when clean).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandBack {
    pub onto: String,
    pub head: String,
    pub files: Vec<String>,
}

/// The first seven characters of a sha, as decision 21's messages spell `<old7>`.
pub(crate) fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// Decision 36 step 2: `git merge-tree --write-tree --name-only --no-messages -z
/// <run_head> <task_head>`. Exit 0 is a clean merge and its tree; exit 1 is a conflict
/// and the file names after the tree. `-z` keeps a non-ASCII or oddly named path
/// unquoted. Any other failure (a name that is not a commit) is an error.
pub fn merge_tree(
    git: &OsStr,
    root: &Path,
    run_head: &str,
    task_head: &str,
    timeout: Duration,
) -> Result<CandidateStep, String> {
    let g = Git::new(git, timeout);
    let args = [
        os("merge-tree"),
        os("--write-tree"),
        os("--name-only"),
        os("--no-messages"),
        os("-z"),
        os(run_head),
        os(task_head),
    ];
    let (output, kept) = g.read_head(root, &args, LARGE_OUTPUT_BYTES)?;
    let text = String::from_utf8_lossy(&kept.head);
    let mut fields: Vec<&str> = nul_fields(&text).collect();
    if kept.dropped() {
        // More than `LARGE_OUTPUT_BYTES` of names: the last one kept may be cut.
        fields.pop();
    }
    let mut fields = fields.into_iter();
    let is_tree = |t: &str| t.len() >= 40 && t.bytes().all(|b| b.is_ascii_hexdigit());
    match fields.next() {
        Some(tree) if output.success && is_tree(tree) => Ok(CandidateStep::Tree(tree.to_string())),
        // A failure with a tree on stdout is the conflict (exit 1); one without (a bad
        // name prints only to stderr) is an error.
        Some(tree) if !output.success && is_tree(tree) => {
            let mut files: Vec<String> = fields.map(str::to_string).collect();
            files.dedup();
            Ok(CandidateStep::Conflict(files))
        }
        _ => Err(failure(&args, &output)),
    }
}

/// Decision 36 step 3: `git commit-tree <tree> -p <parent>… -m <message>`, the
/// candidate commit. The parents' order is the caller's: `(run_head, task_head)`.
pub fn commit_tree(
    git: &OsStr,
    root: &Path,
    tree: &str,
    parents: &[&str],
    message: &str,
    timeout: Duration,
) -> Result<String, String> {
    let g = Git::new(git, timeout);
    let mut args = vec![os("commit-tree"), os(tree)];
    for parent in parents {
        args.extend([os("-p"), os(parent)]);
    }
    args.extend([os("-m"), os(message)]);
    Ok(g.write(root, &args)?.trim().to_string())
}

/// Decision 36 step 3: the candidate checked out in the integration worktree for its
/// check: `git checkout --detach --force <commit>`, then `git clean -fd` (an earlier
/// check's untracked output goes; ignored build caches stay).
pub fn materialize(
    git: &OsStr,
    integration: &Path,
    commit: &str,
    timeout: Duration,
) -> Result<(), String> {
    let g = Git::new(git, timeout);
    g.write(
        integration,
        &[
            os("checkout"),
            os("-q"),
            os("--detach"),
            os("--force"),
            os(commit),
        ],
    )?;
    g.write(integration, &[os("clean"), os("-q"), os("-fd")])?;
    Ok(())
}

/// Decision 36 step 4: `git update-ref refs/heads/<branch> <new> <old>`, a
/// compare-and-swap. `false` when the branch was not at `old` (moved, or missing); the
/// branch is then untouched.
pub fn cas_update(
    git: &OsStr,
    root: &Path,
    branch: &str,
    new: &str,
    old: &str,
    timeout: Duration,
) -> Result<bool, String> {
    let g = Git::new(git, timeout);
    let refname = format!("refs/heads/{branch}");
    let args = [os("update-ref"), os(&refname), os(new), os(old)];
    let output = g.write_raw(root, &args)?;
    if output.success {
        return Ok(true);
    }
    // git's refusal names the mismatch in words that vary by version; the ref itself
    // says whether `old` was the reason.
    match read(g, root, &refname)? {
        Some(current) if current == old => Err(failure(&args, &output)),
        _ => Ok(false),
    }
}

/// Decision 36 steps 4 and 5: the integration worktree back on the run branch,
/// `git checkout --force <branch>`, whatever a check left in tracked files.
pub fn reattach(
    git: &OsStr,
    integration: &Path,
    branch: &str,
    timeout: Duration,
) -> Result<(), String> {
    reattach_in(Git::new(git, timeout), integration, branch)
}

pub(crate) fn reattach_in(g: Git<'_>, integration: &Path, branch: &str) -> Result<(), String> {
    // The trailing `--` keeps a branch name from ever being read as a path.
    g.write(
        integration,
        &[
            os("checkout"),
            os("-q"),
            os("--force"),
            os(branch),
            os("--"),
        ],
    )?;
    Ok(())
}

/// The commit `refname` points at, or `None` when it does not exist.
pub fn read_ref(
    git: &OsStr,
    root: &Path,
    refname: &str,
    timeout: Duration,
) -> Result<Option<String>, String> {
    read(Git::new(git, timeout), root, refname)
}

pub(crate) fn read(g: Git<'_>, root: &Path, refname: &str) -> Result<Option<String>, String> {
    let args = [os("rev-parse"), os("-q"), os("--verify"), os(refname)];
    let output = g.read(root, &args)?;
    if output.success {
        Ok(Some(output.stdout.trim().to_string()))
    } else if output.stderr.trim().is_empty() {
        Ok(None)
    } else {
        Err(failure(&args, &output))
    }
}

/// Decision 21: the run ref first (only the engine writes it, so any difference halts),
/// then the base ref: equal is fine, a descendant of `base_sha` is an advance (with
/// its commit count) unless the run's own refs reach any of its new commits (final
/// fix batch F1, finding D-2: [`run_work_on_base`]), anything else (rewritten or
/// deleted, or run work) halts.
pub fn guard_refs(
    git: &OsStr,
    root: &Path,
    base_branch: &str,
    base_sha: &str,
    run_branch: &str,
    run_head: &str,
    timeout: Duration,
) -> Result<RefCheck, String> {
    let g = Git::new(git, timeout);
    let run_ref = format!("refs/heads/{run_branch}");
    match read(g, root, &run_ref)? {
        Some(head) if head == run_head => {}
        Some(head) => {
            return Ok(RefCheck::Halt {
                reason: format!(
                    "{run_ref} moved from {} to {}",
                    short(run_head),
                    short(&head)
                ),
            });
        }
        None => {
            return Ok(RefCheck::Halt {
                reason: format!("{run_ref} was deleted"),
            });
        }
    }
    let base_ref = format!("refs/heads/{base_branch}");
    let Some(to) = read(g, root, &base_ref)? else {
        return Ok(RefCheck::Halt {
            reason: format!("{base_ref} was deleted"),
        });
    };
    if to == base_sha {
        return Ok(RefCheck::Ok);
    }
    if !is_ancestor(g, root, base_sha, &to)? {
        return Ok(RefCheck::Halt {
            reason: format!(
                "{base_ref} was rewritten: {} is not an ancestor of {}",
                short(base_sha),
                short(&to)
            ),
        });
    }
    // Final fix batch F1, finding D-2: a base that "advanced" onto the run's own work
    // was moved by something inside the run (a worker's `update-ref`, a check), not by
    // someone committing on it.
    let run_id = run_id_of(run_branch);
    let from_run = work_on_base(g, root, run_id, base_sha, &to, None)?;
    if from_run > 0 {
        return Ok(RefCheck::Halt {
            reason: format!(
                "{base_ref} contains unaccepted run work ({from_run} {})",
                if from_run == 1 { "commit" } else { "commits" }
            ),
        });
    }
    let commits = count(g, root, base_sha, &to)?;
    Ok(RefCheck::BaseAdvanced { to, commits })
}

/// `<run>` of the run branch `anthrex/<run>/integration`.
fn run_id_of(run_branch: &str) -> &str {
    run_branch
        .strip_prefix("anthrex/")
        .and_then(|rest| rest.rsplit_once('/'))
        .map_or(run_branch, |(run, _)| run)
}

/// Final fix batch F1, finding D-2: how many of the base's new commits
/// (`<base_sha>..<to>`) the run's own refs reach — its branches
/// `refs/heads/anthrex/<run>/…` and its salvage refs `refs/anthrex/salvage/<run>/…` —
/// leaving out, when `merged` is given and `to` contains it, the history of `merged`:
/// the run head, merged into the base by the user (decision 20's advice after an accept
/// conflict). Any other count above zero is run work on the base that no accept put
/// there.
pub fn run_work_on_base(
    git: &OsStr,
    root: &Path,
    run_id: &str,
    base_sha: &str,
    to: &str,
    merged: Option<&str>,
    timeout: Duration,
) -> Result<u32, String> {
    work_on_base(Git::new(git, timeout), root, run_id, base_sha, to, merged)
}

fn work_on_base(
    g: Git<'_>,
    root: &Path,
    run_id: &str,
    base_sha: &str,
    to: &str,
    merged: Option<&str>,
) -> Result<u32, String> {
    let not_base = format!("^{base_sha}");
    let mut range = vec![os("rev-list"), os("--count"), os(to), os(&not_base)];
    let not_merged = match merged {
        Some(head) if is_ancestor(g, root, head, to)? => Some(format!("^{head}")),
        _ => None,
    };
    if let Some(not_merged) = &not_merged {
        range.push(os(not_merged));
    }
    let all = parse_count(&g.ok(root, &range)?)?;
    if all == 0 {
        return Ok(0);
    }
    // Without a wildcard, `--glob` matches every ref below the prefix, and only below
    // it: `anthrex/r1` never reaches `anthrex/r10/…`.
    let branches = format!("--glob=refs/heads/anthrex/{run_id}");
    let salvage = format!("--glob=refs/anthrex/salvage/{run_id}");
    range.extend([os("--not"), os(&branches), os(&salvage)]);
    let outside = parse_count(&g.ok(root, &range)?)?;
    Ok(all.saturating_sub(outside))
}

fn parse_count(text: &str) -> Result<u32, String> {
    text.trim()
        .parse::<u32>()
        .map_err(|_| format!("git rev-list --count printed {:?}", text.trim()))
}

/// `git rev-list --count <from>..<to>`.
fn count(g: Git<'_>, root: &Path, from: &str, to: &str) -> Result<u32, String> {
    let range = format!("{from}..{to}");
    parse_count(&g.ok(root, &[os("rev-list"), os("--count"), os(&range)])?)
}

/// Decision 20: the base's new commits, `<sha7> <author>: <subject>`, newest first, at
/// most `limit` of them, and how many there are in all. `--no-color` and
/// `--no-show-signature` keep a user's `color.ui=always` or `log.showSignature` out of
/// the lines.
pub fn commits_since(
    git: &OsStr,
    root: &Path,
    from: &str,
    to: &str,
    limit: usize,
    timeout: Duration,
) -> Result<(Vec<String>, u32), String> {
    let g = Git::new(git, timeout);
    let range = format!("{from}..{to}");
    let max = format!("--max-count={limit}");
    let text = g.ok(
        root,
        &[
            os("log"),
            os("--no-color"),
            os("--no-show-signature"),
            os("--abbrev=7"),
            os("--format=%h %an: %s"),
            os(&max),
            os(&range),
            os("--"),
        ],
    )?;
    let lines = text.lines().map(str::to_string).collect();
    Ok((lines, count(g, root, from, to)?))
}

/// Decision 36 step 6: the run head merged into the task worktree. A clean merge is
/// committed and gives no files. A conflict leaves its markers and `MERGE_HEAD` for the
/// worker and gives the unmerged files. Any other failure (untracked files in the way,
/// say), or a merge already in progress, is an error. The result names the tip the
/// merge was made onto (ruling T14-C1), read after the merge ran (review N4): the
/// engine re-queues a hand-back only when that tip is the claimed commit.
///
/// Final fix batch F1, fix round 3: no ref is written through `HEAD`, which a worker's
/// leftover process could point at the base between the engine's check and git's read
/// of it. `git merge --no-commit --no-ff` writes only the index, the files and the
/// merge state; the merge commit is made with `write-tree` and `commit-tree` (parents
/// explicit), and the task's own branch, the one ref its pin names, is moved with a
/// compare-and-swap `update-ref`. A merge not made onto that branch's tip, or a `HEAD`
/// that no longer names it, is an error, and nothing but that branch was written.
pub fn hand_back(
    git: &OsStr,
    worktree: &Path,
    run_head: &str,
    timeout: Duration,
) -> Result<HandBack, String> {
    let g = Git::new(git, timeout);
    let own = own_ref(worktree)?;
    // Ruling T11-N1(a): a leftover MERGE_HEAD would make git refuse the merge and
    // `unmerged` report the old merge's files as if they were this one's conflict.
    if merge_in_progress(g, worktree)? {
        return Err(format!(
            "a merge is already in progress in {}; finish it or run git merge --abort",
            worktree.display()
        ));
    }
    let run_head = read(g, worktree, &format!("{run_head}^{{commit}}"))?
        .ok_or_else(|| format!("{run_head} is not a commit"))?;
    let args = [
        os("merge"),
        os("-q"),
        os("--no-ff"),
        os("--no-commit"),
        os("--no-autostash"),
        os(&run_head),
    ];
    let output = g.write_raw(worktree, &args)?;
    // Review N4: `onto` is read after the merge ran, from the branch itself.
    let onto = read(g, worktree, &own)?.ok_or_else(|| format!("{own} does not exist"))?;
    // Every read below goes through the pin's `HEAD` check: a `HEAD` that no longer
    // names the task's branch is refused there.
    let head = read(g, worktree, "HEAD")?;
    if head.as_deref() != Some(onto.as_str()) {
        return Err(moved(worktree, &own));
    }
    if !output.success {
        let files = unmerged(g, worktree)?;
        return if files.is_empty() {
            Err(failure(&args, &output))
        } else {
            Ok(HandBack {
                head: onto.clone(),
                onto,
                files,
            })
        };
    }
    if !merge_in_progress(g, worktree)? {
        // Ruling T14-R3 (R2-2): a run head already in the task's history ("Already up
        // to date") leaves no merge state; the tip merged onto is the branch itself.
        return Ok(HandBack {
            head: onto.clone(),
            onto,
            files: Vec::new(),
        });
    }
    if read(g, worktree, "ORIG_HEAD")?.as_deref() != Some(onto.as_str()) {
        return Err(moved(worktree, &own));
    }
    let tree = g.write(worktree, &[os("write-tree")])?.trim().to_string();
    let branch = own.trim_start_matches("refs/heads/");
    let message = format!("Merge commit '{run_head}' into {branch}");
    let commit = g
        .write(
            worktree,
            &[
                os("commit-tree"),
                os(&tree),
                os("-p"),
                os(&onto),
                os("-p"),
                os(&run_head),
                os("-m"),
                os(&message),
            ],
        )?
        .trim()
        .to_string();
    let cas = [os("update-ref"), os(&own), os(&commit), os(&onto)];
    let output = g.write_raw(worktree, &cas)?;
    if !output.success {
        return Err(format!(
            "{}; {}",
            moved(worktree, &own),
            failure(&cas, &output)
        ));
    }
    // The index already holds the merge's tree; only the merge state goes.
    quit_merge(g, worktree)?;
    if read(g, worktree, "HEAD")?.as_deref() != Some(commit.as_str()) {
        return Err(moved(worktree, &own));
    }
    Ok(HandBack {
        onto,
        head: commit,
        files: Vec::new(),
    })
}

/// Fix round 3: the branch ref the engine may move in `worktree` (its pin's), or an
/// error: the engine writes no ref it cannot name.
fn own_ref(worktree: &Path) -> Result<String, String> {
    crate::worktree::pinned::own_ref(worktree).ok_or_else(|| {
        format!(
            "{} is not an engine worktree on a branch of its own; refusing to write in it",
            worktree.display()
        )
    })
}

fn moved(worktree: &Path, own: &str) -> String {
    format!(
        "{own} or {}'s HEAD moved during the hand-back; the merge left in progress there \
         was not committed",
        worktree.display()
    )
}

/// Ruling T11-N1(b): undoes a hand-back's conflicted merge in a task worktree so the
/// next hand-back starts from the task's own commit. Without a `MERGE_HEAD` there is
/// nothing to undo. Fix round 3: not `git merge --abort`, whose reset writes `HEAD`'s
/// ref; the index and files are read back from the task's own branch (`read-tree
/// --reset -u`), and `merge --quit` drops the merge state. No ref is written.
pub fn abort_merge(git: &OsStr, worktree: &Path, timeout: Duration) -> Result<(), String> {
    let g = Git::new(git, timeout);
    // Refused up front in a worktree without a branch of its own, merge or not.
    own_ref(worktree)?;
    if !merge_in_progress(g, worktree)? {
        return Ok(());
    }
    drop_merge(g, worktree)
}

/// Fix round 3: the merge in progress in `worktree` undone without writing a ref: the
/// index and files read back from the task's own branch, then the merge state dropped.
pub(crate) fn drop_merge(g: Git<'_>, worktree: &Path) -> Result<(), String> {
    let own = own_ref(worktree)?;
    let tip = read(g, worktree, &own)?.ok_or_else(|| format!("{own} does not exist"))?;
    g.write(
        worktree,
        &[os("read-tree"), os("-u"), os("--reset"), os(&tip)],
    )?;
    quit_merge(g, worktree)
}

/// `git merge --quit`: the merge state (`MERGE_HEAD`, `MERGE_MSG`, …) dropped; the
/// index, the files and every ref are left as they are.
pub(crate) fn quit_merge(g: Git<'_>, worktree: &Path) -> Result<(), String> {
    g.write(worktree, &[os("merge"), os("--quit")]).map(|_| ())
}

fn merge_in_progress(g: Git<'_>, worktree: &Path) -> Result<bool, String> {
    let args = [os("rev-parse"), os("-q"), os("--verify"), os("MERGE_HEAD")];
    Ok(g.read(worktree, &args)?.success)
}

/// The unmerged paths in `dir`'s index, in git's order.
pub(crate) fn unmerged(g: Git<'_>, dir: &Path) -> Result<Vec<String>, String> {
    let mut args = vec![os("diff")];
    args.extend(DIFF_FLAGS.map(os));
    args.extend([os("--name-only"), os("-z"), os("--diff-filter=U")]);
    let text = g.ok(dir, &args)?;
    let mut files: Vec<String> = nul_fields(&text).map(str::to_string).collect();
    files.dedup();
    Ok(files)
}
