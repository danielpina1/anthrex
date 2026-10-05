//! Milestone 9.6 decisions 23 and 24 (DF §5.3): what the documents commit carries, and
//! where each document lands in the repository. Pure: the engine fills a
//! [`DocsCommitSpec`] from the run's index of versions; the driver reads each stored
//! version back with its recorded size and SHA-256 (`driver/design_commit.rs`) and
//! names it with [`slug`] and [`repo_path`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The folders under `docs_dir` the documents go to (decision 24): the spec's, the
/// plan's and the brainstorm report's.
pub const FOLDERS: [&str; 3] = ["specs", "plans", "brainstorms"];

/// The slug's most characters (decision 24).
pub const SLUG_MAX: usize = 48;

/// `OpKind::CommitDesignDocs`' spec: the run branch at `expected_head`, the documents
/// to add on top of its tree, and decision 24's date and message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocsCommitSpec {
    pub root: PathBuf,
    /// `anthrex/<run>/integration`.
    pub branch: String,
    /// The run head the commit's one parent is, and the branch must be at.
    pub expected_head: String,
    /// The integration worktree, put back on the branch once it moved.
    pub integration: PathBuf,
    pub docs_dir: String,
    /// The run's start date, `YYYY-MM-DD` in UTC.
    pub date: String,
    /// The slug when the spec's title gives none.
    pub run_id: String,
    /// The approved spec first (its title is the slug), then the plan, then the
    /// brainstorm report with `commit_brainstorm`.
    pub files: Vec<DocSource>,
    pub message: String,
    /// Task M9.6.15 (DF §8.1): a later round's commit is its first new stage. This
    /// stage branch is created at the commit, and `branch` (its alias) moves to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_branch: Option<String>,
}

/// One stored, approved version: the folder it goes to under `docs_dir` (`specs`,
/// `plans`, `brainstorms`), its file in the design folder, and what the index recorded
/// of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocSource {
    pub folder: String,
    /// `spec v2`, for a failure's text.
    pub what: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    /// Task M9.6.15: its repository path when it is not named by the slug (a later
    /// round's documents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
    /// Task M9.6.15 (decision 24): appended under this heading to the file already at
    /// `repo_path` in the run head (`## Round <k> amendment`), not written whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append: Option<String>,
}

/// Decision 24: the slug of the spec's `# ` title: lowercase ASCII alphanumerics, every
/// other character a `-`, runs of `-` collapsed and none at either end, at most
/// [`SLUG_MAX`] characters; `run_id` when that leaves nothing.
pub fn slug(spec: &str, run_id: &str) -> String {
    let title = spec.lines().find_map(|line| line.strip_prefix("# "));
    let mut out = String::new();
    for c in title.unwrap_or_default().chars() {
        let c = match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '-',
        };
        if c == '-' && (out.is_empty() || out.ends_with('-')) {
            continue;
        }
        out.push(c);
    }
    out.truncate(SLUG_MAX);
    let out = out.trim_end_matches('-');
    match out.is_empty() {
        true => run_id.to_string(),
        false => out.to_string(),
    }
}

/// Decision 24: `<docs_dir>/<folder>/<date>-<slug>.md`.
pub fn repo_path(docs_dir: &str, folder: &str, date: &str, slug: &str) -> String {
    format!("{docs_dir}/{folder}/{date}-{slug}.md")
}

#[cfg(test)]
#[path = "commit_tests.rs"]
mod tests;
