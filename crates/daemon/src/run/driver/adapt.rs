//! Milestone 8b's driver side (M8b decision 1): the executions and requests the
//! adaptation adds. `driver/requests.rs` and `driver/ops.rs` only dispatch here.
//!
//! M8b.4: [`RunService::choose_profile`], decision 6 at `run start`. The stored profile
//! is read, and its staleness checked, on `spawn_blocking`, never under a lock.

use std::path::PathBuf;

use proto::{OutputFilter, Plan, ProfileSource};

use super::RunService;
use crate::profile::resolve::{apply_choice as apply_to_plan, run_profile};
use crate::profile::store::{self, PROFILE_FILE, Stored};
use crate::run::model::{LogEntry, Run};
use crate::run::plan::Preflight;

/// What decision 6 chose, for the run `build_run` makes.
pub(super) struct ProfileChoice {
    source: ProfileSource,
    output_filter: OutputFilter,
    filter_prefixes: Vec<String>,
    repo_dir: PathBuf,
    stale: Vec<String>,
    notes: Vec<String>,
}

/// Decision 6's refusal for a stored profile that does not parse.
fn unparseable(path: &std::path::Path, error: &str) -> String {
    format!(
        "the stored profile at {} does not parse: {error}; fix it with anthrex profile edit or re-detect it with anthrex profile detect",
        path.display()
    )
}

impl RunService {
    /// Decision 6, right after preflight: loads the repository's stored profile and,
    /// when there is one, makes it the plan's whole profile and empties the cloned
    /// config's `profile`, so `resolve_profile` fills no deliberate gap. The config's
    /// confinement tables are left alone: `build_run` takes them for the root. A stored
    /// profile that does not parse refuses the run.
    pub(super) async fn choose_profile(
        &self,
        plan: &mut Plan,
        config: &mut config::Orchestrator,
        pre: &Preflight,
    ) -> Result<ProfileChoice, String> {
        let repo_dir = crate::profile::repo_dir(&self.ctx.data_dir, &pre.project);
        let (dir, project) = (repo_dir.clone(), pre.project.clone());
        let (stored, stale) = tokio::task::spawn_blocking(move || {
            let stored = store::load(&dir);
            let stale = match &stored {
                Stored::Found { meta, .. } => store::stale(&project, meta),
                _ => Vec::new(),
            };
            (stored, stale)
        })
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?;
        let (stored, path) = match stored {
            Stored::Found { profile, path, .. } => (Some(profile), path),
            Stored::Unparseable { path, error } => return Err(unparseable(&path, &error)),
            Stored::Absent => (None, repo_dir.join(PROFILE_FILE)),
        };
        let chosen = run_profile(stored.as_ref(), &path, &plan.profile, &config.profile);
        apply_to_plan(&chosen, &mut plan.profile, &mut config.profile);
        Ok(ProfileChoice {
            source: chosen.source,
            output_filter: chosen.output_filter,
            filter_prefixes: chosen.filter_prefixes,
            repo_dir,
            stale,
            notes: chosen.notes,
        })
    }
}

/// Copies the choice onto the built run: its source, filter settings, repository data
/// directory, stale files (the attention line) and one log line per ignored plan key.
pub(super) fn apply_choice(run: &mut Run, choice: ProfileChoice, now: u64) {
    run.profile_source = Some(choice.source);
    run.output_filter = choice.output_filter;
    run.filter_prefixes = choice.filter_prefixes;
    run.repo_dir = choice.repo_dir;
    run.stale_profile = choice.stale;
    run.log.extend(
        choice
            .notes
            .into_iter()
            .map(|text| LogEntry { at: now, text }),
    );
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use proto::{OutputFilter, ProfileSource};

    use super::{ProfileChoice, apply_choice};
    use crate::run::plan::{BuildContext, Preflight, build_run, parse_plan};

    fn run() -> crate::run::model::Run {
        let plan = parse_plan(
            "goal = \"g\"\n[[task]]\nid = \"t1\"\ntitle = \"One\"\nsize = \"S\"\nowns = [\"a.rs\"]\nbrief = \"b\"\nacceptance = [\"a\"]\n",
        )
        .unwrap();
        let config = config::Orchestrator::default();
        let pre = Preflight {
            root: PathBuf::from("/tmp/ax-adapt-root"),
            project: PathBuf::from("/tmp/ax-adapt-root"),
            git_common_dir: PathBuf::from("/tmp/ax-adapt-root/.git"),
            base_branch: "main".into(),
            base_sha: "b".repeat(40),
            protected_files: Vec::new(),
        };
        let ctx = BuildContext {
            id: "g-0001".into(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: PathBuf::from("/tmp/data/runs/g-0001"),
            config: &config,
            now: 1,
            yes: false,
        };
        build_run(plan, pre, ctx).unwrap_or_else(|e| panic!("{e:?}"))
    }

    #[test]
    fn apply_choice_copies_every_field_and_logs_each_note() {
        let mut run = run();
        assert_eq!(run.profile_source, None);
        assert_eq!(run.repo_dir, PathBuf::new());
        let choice = ProfileChoice {
            source: ProfileSource::Stored,
            output_filter: OutputFilter::Tail,
            filter_prefixes: vec!["cargo test".into()],
            repo_dir: PathBuf::from("/data/repos/r-00000000"),
            stale: vec!["Cargo.toml".into()],
            notes: vec!["note one".into(), "note two".into()],
        };
        apply_choice(&mut run, choice, 42);
        assert_eq!(run.profile_source, Some(ProfileSource::Stored));
        assert_eq!(run.output_filter, OutputFilter::Tail);
        assert_eq!(run.filter_prefixes, vec!["cargo test".to_string()]);
        assert_eq!(run.repo_dir, PathBuf::from("/data/repos/r-00000000"));
        assert_eq!(run.stale_profile, vec!["Cargo.toml".to_string()]);
        let logged: Vec<(u64, &str)> = run.log.iter().map(|e| (e.at, e.text.as_str())).collect();
        assert_eq!(logged, vec![(42, "note one"), (42, "note two")]);
    }
}
