//! Decision 14's control API: a test (or the smoke stage, through the hidden `anthrex
//! run fake-github`) acts as GitHub and as its users. Every method takes the lock,
//! changes `github.json` (and, for a commit or a merge, the bare repository), and
//! returns; a test waits for the daemon to see it with a deadline loop. A method that
//! cannot do what it is asked panics: it is test setup.

use std::path::{Path, PathBuf};

use super::github::{
    self, Bare, CALLS, FORBIDDEN, FakeComment, FakeGithub, FakePr, FakeRepo, FakeReview,
    FakeThread, FakeThreadComment, now,
};
use super::rules::CiRule;
use crate::host::{PrState, RepoPermission, ReviewState};

/// How the user merges on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethodArg {
    Merge,
    Squash,
    Rebase,
}

/// The author of a commit a user makes on GitHub, and of every merge.
const USER: &str = "fake-user";

pub struct FakeGithubCtl {
    dir: PathBuf,
}

impl FakeGithubCtl {
    pub fn open(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        FakeGithubCtl {
            dir: dir.to_path_buf(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn change<T>(&self, f: impl FnOnce(&mut FakeGithub) -> Result<T, String>) -> T {
        let answer = github::with_state(&self.dir, |state| {
            state.refresh()?;
            f(state)
        });
        match answer {
            Ok(Ok(value)) => value,
            Ok(Err(e)) | Err(e) => panic!("FakeGithubCtl: {e}"),
        }
    }

    fn pr(state: &mut FakeGithub, number: u64) -> Result<&mut FakePr, String> {
        state
            .pr_mut(number)
            .ok_or_else(|| format!("no pull request #{number}"))
    }

    pub fn create_repo(&self, owner: &str, name: &str, bare: &Path, default_branch: &str) {
        self.change(|s| {
            s.repo = Some(FakeRepo {
                owner: owner.to_string(),
                name: name.to_string(),
                bare: bare.to_path_buf(),
                default_branch: default_branch.to_string(),
            });
            s.permissions
                .insert(owner.to_string(), RepoPermission::Admin);
            Ok(())
        })
    }

    /// `gh auth status --hostname <host>` succeeds, and `gh` answers for it.
    pub fn log_in(&self, host: &str) {
        self.change(|s| {
            if !s.logged_in.iter().any(|h| h == host) {
                s.logged_in.push(host.to_string());
            }
            Ok(())
        })
    }

    pub fn set_permission(&self, user: &str, permission: RepoPermission) {
        self.change(|s| {
            s.permissions.insert(user.to_string(), permission);
            Ok(())
        })
    }

    /// Replaces the rules; runs already made keep their verdicts.
    pub fn set_ci(&self, rules: Vec<CiRule>) {
        self.change(|s| {
            s.ci_used = vec![0; rules.len()];
            s.ci = rules;
            Ok(())
        })
    }

    /// A conversation comment.
    pub fn comment(&self, pr: u64, user: &str, body: &str) -> u64 {
        self.change(|s| {
            let id = s.next_id();
            Self::pr(s, pr)?.comments.push(FakeComment {
                id,
                user: user.to_string(),
                body: body.to_string(),
            });
            Ok(id)
        })
    }

    /// A new review thread on `path`'s `line` of the head; the id of its first comment.
    pub fn review_comment(&self, pr: u64, user: &str, path: &str, line: u32, body: &str) -> u64 {
        self.change(|s| {
            let id = s.next_id();
            let bare = s.repo.as_ref().ok_or("no repository")?.bare.clone();
            let p = Self::pr(s, pr)?;
            let text = Bare::new(&bare)
                .file(&p.head_oid, path)?
                .unwrap_or_default();
            let at = text
                .lines()
                .nth(line.saturating_sub(1) as usize)
                .unwrap_or("");
            p.threads.push(FakeThread {
                path: path.to_string(),
                line,
                resolved: false,
                comments: vec![FakeThreadComment {
                    id,
                    user: user.to_string(),
                    body: body.to_string(),
                    diff_hunk: format!("@@ -{line},1 +{line},1 @@\n+{at}"),
                }],
            });
            Ok(id)
        })
    }

    pub fn reply_in_thread(&self, pr: u64, first_comment: u64, user: &str, body: &str) -> u64 {
        self.change(|s| {
            let id = s.next_id();
            let thread = thread(Self::pr(s, pr)?, first_comment)?;
            let diff_hunk = thread.comments[0].diff_hunk.clone();
            thread.comments.push(FakeThreadComment {
                id,
                user: user.to_string(),
                body: body.to_string(),
                diff_hunk,
            });
            Ok(id)
        })
    }

    pub fn review(&self, pr: u64, user: &str, state: ReviewState, body: &str) -> u64 {
        self.change(|s| {
            let id = s.next_id();
            Self::pr(s, pr)?.reviews.push(FakeReview {
                id,
                user: user.to_string(),
                state,
                body: body.to_string(),
            });
            Ok(id)
        })
    }

    pub fn resolve(&self, pr: u64, first_comment: u64) {
        self.change(|s| {
            thread(Self::pr(s, pr)?, first_comment)?.resolved = true;
            Ok(())
        })
    }

    /// The user merges `pr` on GitHub, which moves the base branch of the bare
    /// repository; with `delete_branch` GitHub then deletes the head branch and
    /// retargets the open PRs based on it (M9.2.1 check 5, in `FakeGithub::refresh`).
    pub fn merge(&self, pr: u64, method: MergeMethodArg, delete_branch: bool) {
        self.change(|s| merge(s, pr, method, delete_branch))
    }

    pub fn close(&self, pr: u64) {
        self.change(|s| {
            let p = Self::pr(s, pr)?;
            if p.state != PrState::Open {
                return Err(format!("#{pr} is not open"));
            }
            p.state = PrState::Closed;
            Ok(())
        })
    }

    pub fn reopen(&self, pr: u64) {
        self.change(|s| {
            let bare = s.repo.as_ref().ok_or("no repository")?.bare.clone();
            let heads = Bare::new(&bare).heads()?;
            let p = Self::pr(s, pr)?;
            if p.state != PrState::Closed || !heads.contains_key(&p.head) {
                return Err(format!("#{pr} cannot be reopened"));
            }
            p.state = PrState::Open;
            Ok(())
        })
    }

    /// A user's commit on the remote `branch`, writing `content` to `path`.
    pub fn commit(&self, branch: &str, path: &str, content: &str, message: &str) -> String {
        self.change(|s| {
            let bare_dir = s.repo.as_ref().ok_or("no repository")?.bare.clone();
            let bare = Bare::new(&bare_dir);
            let tip = bare.branch(branch)?.ok_or(format!("no branch {branch}"))?;
            let blob =
                bare.ok_input(&["hash-object", "-w", "--stdin"], Some(content.as_bytes()))?;
            let root = bare.ok(&["rev-parse", &format!("{tip}^{{tree}}")])?;
            let parts: Vec<&str> = path.split('/').collect();
            let tree = put(&bare, Some(&root), &parts, &blob)?;
            let sha = bare.commit_tree(&tree, &[&tip], USER, message)?;
            bare.set_branch(branch, &sha, Some(&tip))?;
            s.refresh()?;
            Ok(sha)
        })
    }

    /// `branch` replaced by a commit that does not descend from it, as a force-push
    /// would leave it.
    pub fn force_rewrite(&self, branch: &str) -> String {
        self.change(|s| {
            let bare_dir = s.repo.as_ref().ok_or("no repository")?.bare.clone();
            let bare = Bare::new(&bare_dir);
            let tip = bare.branch(branch)?.ok_or(format!("no branch {branch}"))?;
            let tree = bare.ok(&["rev-parse", &format!("{tip}^{{tree}}")])?;
            let parent = bare.run(
                &["rev-parse", "--verify", "--quiet", &format!("{tip}^")],
                None,
            )?;
            let parent = parent.stdout.trim().to_string();
            let parents: Vec<&str> = if parent.is_empty() {
                vec![]
            } else {
                vec![&parent]
            };
            let sha = bare.commit_tree(&tree, &parents, USER, &format!("rewrite {branch}"))?;
            bare.set_branch(branch, &sha, Some(&tip))?;
            s.refresh()?;
            Ok(sha)
        })
    }

    /// The next `calls` `gh` calls that reach GitHub fail with its rate-limit text.
    pub fn rate_limit(&self, calls: u32) {
        self.change(|s| {
            s.rate_limited = calls;
            Ok(())
        })
    }

    pub fn prs(&self) -> Vec<FakePr> {
        self.change(|s| Ok(s.prs.clone()))
    }

    /// `calls.jsonl`: every `gh` argv (without the program), in order.
    pub fn calls(&self) -> Vec<Vec<String>> {
        github::read_lines(&self.dir, CALLS).unwrap_or_else(|e| panic!("{e}"))
    }

    /// `forbidden.jsonl`: every ask to land something. Must stay empty.
    pub fn forbidden(&self) -> Vec<Vec<String>> {
        github::read_lines(&self.dir, FORBIDDEN).unwrap_or_else(|e| panic!("{e}"))
    }
}

fn thread(pr: &mut FakePr, first_comment: u64) -> Result<&mut FakeThread, String> {
    pr.threads
        .iter_mut()
        .find(|t| t.comments.first().is_some_and(|c| c.id == first_comment))
        .ok_or_else(|| format!("#{} has no thread {first_comment}", pr.number))
}

fn merge(
    s: &mut FakeGithub,
    number: u64,
    method: MergeMethodArg,
    delete: bool,
) -> Result<(), String> {
    let repo = s.repo.clone().ok_or("no repository")?;
    let bare = Bare::new(&repo.bare);
    let pr = s
        .pr(number)
        .cloned()
        .ok_or(format!("no pull request #{number}"))?;
    if pr.state != PrState::Open {
        return Err(format!("#{number} is not open"));
    }
    let base = bare
        .branch(&pr.base)?
        .ok_or(format!("no base {}", pr.base))?;
    let conflict = || format!("#{number} does not merge cleanly into {}", pr.base);
    let new = match method {
        MergeMethodArg::Merge => {
            let tree = bare
                .merge_tree(&base, &pr.head_oid, None)?
                .ok_or_else(conflict)?;
            let message = format!(
                "Merge pull request #{number} from {}/{}\n\n{}",
                repo.owner, pr.head, pr.title
            );
            bare.commit_tree(&tree, &[&base, &pr.head_oid], USER, &message)?
        }
        MergeMethodArg::Squash => {
            let tree = bare
                .merge_tree(&base, &pr.head_oid, None)?
                .ok_or_else(conflict)?;
            let message = format!("{} (#{number})", pr.title);
            bare.commit_tree(&tree, &[&base], USER, &message)?
        }
        MergeMethodArg::Rebase => {
            let listed =
                bare.ok(&["rev-list", "--reverse", &format!("{base}..{}", pr.head_oid)])?;
            let mut at = base.clone();
            for commit in listed.lines() {
                let first_parent = format!("{commit}^");
                let tree = bare
                    .merge_tree(&at, commit, Some(&first_parent))?
                    .ok_or_else(conflict)?;
                let message = bare.ok(&["log", "-1", "--format=%B", commit])?;
                at = bare.commit_tree(&tree, &[&at], USER, &message)?;
            }
            at
        }
    };
    bare.set_branch(&pr.base, &new, Some(&base))?;
    if delete && let Some(head) = bare.branch(&pr.head)? {
        bare.delete_branch(&pr.head, &head)?;
    }
    let p = s.pr_mut(number).ok_or("gone")?;
    p.state = PrState::Merged;
    p.merged_at = Some(now());
    p.merge_commit = Some(new);
    s.refresh()
}

/// `tree` with `blob` at `parts` (`git mktree`, recursively; no index involved).
fn put(bare: &Bare<'_>, tree: Option<&str>, parts: &[&str], blob: &str) -> Result<String, String> {
    let (name, rest) = parts.split_first().ok_or("an empty path")?;
    let listed = match tree {
        Some(t) => bare.ok(&["ls-tree", "-z", t])?,
        None => String::new(),
    };
    let mut entries: Vec<String> = Vec::new();
    let mut sub = None;
    for entry in listed.split('\0').filter(|e| !e.is_empty()) {
        let (meta, entry_name) = entry.split_once('\t').ok_or("ls-tree output")?;
        if entry_name == *name {
            let mut fields = meta.split(' ');
            if fields.nth(1) == Some("tree") {
                sub = fields.next().map(str::to_string);
            }
        } else {
            entries.push(entry.to_string());
        }
    }
    if rest.is_empty() {
        entries.push(format!("100644 blob {blob}\t{name}"));
    } else {
        let new_sub = put(bare, sub.as_deref(), rest, blob)?;
        entries.push(format!("040000 tree {new_sub}\t{name}"));
    }
    let mut input = entries.join("\0");
    input.push('\0');
    bare.ok_input(&["mktree", "-z"], Some(input.as_bytes()))
}
