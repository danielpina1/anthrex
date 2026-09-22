//! `WindowManager::restore` and `WindowManager::state_snapshot`: the two directions of
//! milestone 6's persistence (decisions 9 and 14). `restore` turns a loaded
//! [`state::StateFile`] into dormant entries at startup; `state_snapshot` turns the live
//! entry table back into one, for [`crate::state::spawn_persister`] and the final flush
//! in [`crate::lifecycle::run`] to write.
//!
//! Both run entirely under the manager lock and do no I/O of any kind — no filesystem,
//! no git, no process spawn. `state_snapshot` in particular is decision 9's "a clone
//! taken under the manager lock with no I/O under it": every field it reads is already
//! in memory, so the lock is held for exactly as long as cloning a handful of small
//! values takes, never for as long as serializing or writing them would.

use super::entry::{Entry, Inner, Process};
use super::{DAEMON_RESTARTED, WindowManager, sanitize_name};
use crate::agent_state::AgentState;
use crate::state::{self, StateFile, WindowRecord, WorktreeRecord};
use crate::worktree::ManagedWorktree;
use proto::{ExitInfo, Status, WindowSpec};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::sync::broadcast;
use unicode_segmentation::UnicodeSegmentation;

/// A restored window has no saved terminal size (the state file does not record one), so
/// it starts at this and is resized to the client's real size on the first `Subscribe`,
/// exactly as `server::handle_client` already does for a live window before `attach`.
const RESTORED_SIZE: (u16, u16) = (80, 24);

impl WindowManager {
    /// Rebuilds the window table from a loaded state file (decision 14), called once at
    /// startup, before the socket is bound, so the first client's `Welcome` already lists
    /// every restored window.
    ///
    /// Each record becomes one dormant [`Entry`]: no process, status `Exited`, `exit` set
    /// to [`DAEMON_RESTARTED`]. `next_id` is derived independently of the state file's own
    /// `next_id` field — recomputed here from the loaded records' own ids, the same
    /// saturating max-plus-one `state::load` already applies — rather than trusted
    /// verbatim. `state::load`'s own `next_id` is already correct when this state came
    /// from it, but `restore` is a public entry point in its own right (this milestone's
    /// tests call it directly with a hand-built `StateFile`), and a caller that got
    /// `next_id` wrong must not be able to hand a later `create` a colliding id — the same
    /// class of bug the previous task's review found twice in `state.rs`. Recomputing
    /// here means a restored set with gaps (ids 1 and 9, not 1 and 2) still leaves
    /// `next_id` past every one of them, and a record already holding `u32::MAX` still
    /// saturates instead of wrapping.
    ///
    /// Publishes exactly once, after every record is inserted, rather than once per
    /// record: a watcher (in particular the persister this same startup sequence spawns
    /// moments later) must never see a partially-restored table.
    ///
    /// A record whose id or name is already present in `inner.entries` — a live window,
    /// or one this same call already restored — is skipped and logged instead of
    /// overwriting it (fix wave 4, item 3). `state::load` already rejects a record that
    /// repeats an earlier one's id or name within a single file, but `restore` is a
    /// public entry point in its own right (this milestone's tests call it directly with
    /// a hand-built `StateFile`, and nothing stops a second `restore` call against a
    /// manager that already has entries) — the same "do not trust the caller" reasoning
    /// this function's own `next_id` handling already applies to the counter has to apply
    /// to the table it inserts into, or a colliding id silently drops a live window's
    /// `Window` (and orphans its PTY) with no kill and no `start_cleanup`.
    pub fn restore(&self, state: StateFile) {
        let mut inner = crate::lock(&self.inner);
        let now = Instant::now();
        let mut next_id = inner.next_id;

        // Fix wave 8, Major 1: `unique_name` below disambiguates a sanitized name only
        // against `inner.entries` — records already inserted by this same loop, or a live
        // window from before this call. It has no way to see a record still waiting later
        // in `state.windows` whose name is already valid and needs no repair at all. Left
        // alone, a record that *does* need repair can sanitize onto that later record's
        // exact name, take it first, and the later record is then dropped by the ordinary
        // name-collision check a few lines down — a regression this fix closes by making
        // every name that will NOT be repaired (its `sanitize_name` output equals its own
        // input) reserved up front, before any record is processed, so a repaired name can
        // never steal one. This is computed once, from the file as loaded, not
        // incrementally as records are inserted: a record already present in `inner`
        // (skipped below by the id check, never reaching this loop's insert at all) must
        // not reserve anything, since it will never claim its name here.
        let windows = state.windows;
        let protected_names: std::collections::HashSet<String> = windows
            .iter()
            .filter(|record| {
                !inner.entries.contains_key(&record.id)
                    && sanitize_name(record.id, &record.name) == record.name
            })
            .map(|record| record.name.clone())
            .collect();

        for record in windows {
            let WindowRecord {
                id,
                name,
                runtime,
                cwd,
                project,
                worktree,
                model,
                initial_prompt,
                session_id,
                created_at,
                status: _saved_status,
                run,
            } = record;

            if inner.entries.contains_key(&id) {
                tracing::warn!(
                    id,
                    name = %name,
                    "restore: skipping a record whose id is already present"
                );
                continue;
            }

            // Fix wave 6, Major finding: `state.json` is a user-editable input, not a
            // private serialization format, and `state::load` never applies
            // `validate_name`'s rules to a loaded record — only this module's own
            // id/name-collision checks. Ruling: sanitize, do not reject, so a bad name
            // never costs the user their window. A name that was already valid
            // (`sanitize_name` is idempotent on anything `validate_name` would accept, so
            // equality here means nothing needed repairing) falls straight through to the
            // untouched collision check below, unchanged from before this fix. A name that
            // needed repair is also made unique against every name already on the table —
            // live windows, any earlier record this same call already restored, and (fix
            // wave 8, Major 1) every not-yet-processed record whose own name needs no
            // repair at all, via `protected_names` above — since dropping a *sanitized*
            // name for colliding would undo the "keep the window" ruling for the exact
            // record it exists to protect, and letting it steal a later, valid record's
            // name would undo that same ruling for the window it displaces instead.
            let sanitized = sanitize_name(id, &name);
            let name = if sanitized == name {
                name
            } else {
                let unique = unique_name(&inner, &sanitized, &protected_names);
                tracing::warn!(
                    id,
                    original = ?name,
                    sanitized = %unique,
                    "restore: window name failed validation; sanitized to keep the window"
                );
                unique
            };

            if inner.entries.values().any(|entry| entry.name == name) {
                tracing::warn!(
                    id,
                    name = %name,
                    "restore: skipping a record whose name is already present"
                );
                continue;
            }

            let managed = worktree.map(
                |WorktreeRecord {
                     repo_root,
                     path,
                     branch,
                 }| {
                    ManagedWorktree {
                        repo_root,
                        path,
                        branch,
                    }
                },
            );
            // Decision 14 leaves `Entry.managed` and a worktree window's `cwd` untouched
            // across a restore, so a future restart runs in the same checkout without
            // ever calling `worktree::create` again.
            //
            // `Entry.worktree` — the *watched* root, milestone 4.5's field — is only
            // recoverable here for a window this daemon made the worktree for; a window
            // merely standing inside someone else's existing worktree has no saved record
            // of that at all (decision 8 only captures `managed`), so it comes back
            // without one.
            //
            // The root that *is* recovered here is put back under the watcher by
            // `server::register_restored_roots`, at startup. This comment used to end "so
            // it comes back unwatched until it is restarted", which was false in both
            // halves: nothing registered a restored root at startup, and `requests::restart`
            // never took the registry either, so a restart did not repair it. See
            // `crates/daemon/tests/server_restore_git.rs`.
            let watched_worktree = managed.as_ref().map(|m| m.path.clone());
            let spec = WindowSpec {
                name: Some(name.clone()),
                runtime,
                cwd: cwd.clone(),
                worktree_branch: managed.as_ref().map(|m| m.branch.clone()),
                model,
                initial_prompt,
            };
            // Ruling (fix wave 4, item 7): a null saved `project` is kept `None`, not
            // rewritten to `cwd`. A real re-detection would need `project::detect_roots`,
            // a blocking git subprocess, which cannot run here — this function is
            // synchronous and holds the manager lock the whole time (AGENTS.md hard rules
            // 2 and 10) — and the seam that *could* run it before this call, outside the
            // lock, sits in `lifecycle::run` immediately before the socket bind, which is
            // exactly the class of bug fix wave 4, item 1 exists to prevent: a per-window
            // git probe there is that bug again, multiplied by the window count.
            //
            // The earlier fallback to `cwd` looked harmless — "no worse than a plain
            // window" — but `state_snapshot` below writes `Entry.project` back verbatim,
            // so the first save after a restore would have baked the fallback into the
            // file permanently: `null` never survives to be re-detected by a later boot
            // that might do better. Never persisting a value that was derived rather than
            // detected is what keeps that door open; a display value, where one is
            // wanted, is derived at the point of display instead (`Entry::info`).
            let (output, _unused) = broadcast::channel(1);

            let entry = Entry {
                id,
                name,
                spec,
                project,
                worktree: watched_worktree,
                managed,
                removing: false,
                restarting: false,
                status: Status::Exited,
                state: AgentState {
                    session_id,
                    ..AgentState::default()
                },
                viewers: 0,
                since: now,
                last_output: now,
                created_at: UNIX_EPOCH
                    .checked_add(Duration::from_secs(created_at))
                    .unwrap_or(UNIX_EPOCH),
                exit: Some(ExitInfo {
                    code: None,
                    reason: DAEMON_RESTARTED.to_string(),
                }),
                child_alive: false,
                process: Process::Dormant {
                    output,
                    cols: RESTORED_SIZE.0,
                    rows: RESTORED_SIZE.1,
                },
                // Whole-branch-review Major 2: carried verbatim, not discarded — see
                // `Entry.run`'s own doc comment.
                run,
            };

            next_id = next_id.max(id.saturating_add(1));
            inner.entries.insert(id, entry);
        }

        inner.next_id = next_id.max(state.next_id);
        // Whole-branch-review Major 2: see `Inner.runs`'s own doc comment. Overwrites
        // rather than extends — `restore` runs once at startup in production, and a
        // test calling it more than once is building up a fresh scenario each time, not
        // accumulating history this module has no way to deduplicate anyway (the
        // elements are opaque `serde_json::Value`s with no id this module understands).
        inner.runs = state.runs;
        self.publish(&inner);
    }

    /// A clone of the live window table as a [`StateFile`] (decision 9), taken under the
    /// manager lock with no I/O of any kind — every field below is already resident in
    /// `Entry`, `Entry.spec`, `Entry.state` or `Inner` itself, so this is a handful of
    /// clones, not a filesystem call. This milestone never gives `runs` or `run` any
    /// meaning of its own (decision 8) — neither is read anywhere, and `create` always
    /// leaves `Entry.run` `None` — but whole-branch-review Major 2 established that
    /// "never given meaning" must not mean "discarded": both are written back exactly as
    /// `restore` last saw them (`Inner.runs`, `Entry.run`), which is what makes
    /// `state.rs`'s own forward-compatibility rationale for typing them as opaque JSON
    /// actually true.
    pub fn state_snapshot(&self) -> StateFile {
        let inner = crate::lock(&self.inner);
        let windows = inner
            .entries
            .values()
            .map(|entry| WindowRecord {
                id: entry.id,
                name: entry.name.clone(),
                runtime: entry.spec.runtime,
                cwd: entry.spec.cwd.clone(),
                // `entry.project` is already `Option<PathBuf>` and is written back
                // exactly as it came in — never `Some`-wrapped unconditionally — so a
                // restored record's `null` survives any number of restore-then-save
                // cycles instead of being baked into the file as a derived guess (fix
                // wave 4, ruling 7).
                project: entry.project.clone(),
                worktree: entry.managed.as_ref().map(|m| WorktreeRecord {
                    repo_root: m.repo_root.clone(),
                    path: m.path.clone(),
                    branch: m.branch.clone(),
                }),
                model: entry.spec.model.clone(),
                initial_prompt: entry.spec.initial_prompt.clone(),
                session_id: entry.state.session_id.clone(),
                created_at: entry
                    .created_at
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                status: entry.status,
                run: entry.run.clone(),
            })
            .collect();
        StateFile {
            version: state::STATE_VERSION,
            next_id: inner.next_id,
            windows,
            runs: inner.runs.clone(),
        }
    }
}

/// Fix wave 6: makes `base` (already [`sanitize_name`]'s output — valid and within
/// decision 22's length limit) unique against every name already in `inner.entries`, live
/// windows and any earlier record this same `restore` call already inserted alike, by
/// appending `-2`, `-3`, ... until one is free.
///
/// Fix wave 8, Major 1: also avoids every name in `protected`, the set `restore` builds
/// once up front from every record in the file that does **not** need repair — names a
/// later iteration of `restore`'s loop is going to claim outright, with no call to this
/// function at all. Without this, a base that only collides with a not-yet-processed
/// valid record would sail through unchanged (`inner.entries` cannot see a record that
/// has not been inserted yet), take that record's name, and the valid record would then
/// be dropped by the plain collision check as if two saved windows had genuinely fought
/// over one name — when in fact only one of them ever chose it.
///
/// This is deliberately narrower than the ordinary duplicate-name refusal a few lines
/// above it: a genuine collision between two otherwise-*valid* saved names is still
/// refused and the later record dropped, unchanged from before this fix (see
/// `restore_refuses_a_record_whose_name_collides_with_an_existing_entry`). A *sanitized*
/// name is different — it is a repair the manager made up on the window's behalf, not
/// something the user chose, so making it merely unique is what "sanitize, do not reject"
/// (fix wave 6's ruling) requires: the window must survive, and it cannot survive under a
/// name that collides with another live entry or displaces a valid one.
fn unique_name(inner: &Inner, base: &str, protected: &std::collections::HashSet<String>) -> String {
    let taken =
        |name: &str| inner.entries.values().any(|e| e.name == name) || protected.contains(name);
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = suffixed(base, n);
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// `base` with `-<n>` appended, re-truncated (in grapheme clusters, the same unit
/// `validate_name` and `sanitize_name` count by) so the result still fits decision 22's
/// 64-character limit — `sanitize_name` already left `base` at or under that limit on its
/// own, but appending a suffix could push it back over.
fn suffixed(base: &str, n: u32) -> String {
    let suffix = format!("-{n}");
    let budget = 64usize
        .saturating_sub(suffix.graphemes(true).count())
        .max(1);
    let shortened: String = base.graphemes(true).take(budget).collect();
    format!("{shortened}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::ManagerConfig;
    use proto::Runtime;
    use std::path::PathBuf;

    fn manager() -> std::sync::Arc<WindowManager> {
        let (m, _events) = WindowManager::new(ManagerConfig::new(
            "/tmp/unused-restore-test.sock".into(),
            "/bin/sh".into(),
        ));
        m
    }

    /// Minor 3, M6.5 review: of the eight same-typed transpositions the review applied to
    /// this file, `Entry.spec.name` (`name: Some(name.clone())` here vs
    /// `session_id.clone()`) was the one that survived — nothing under `crates/*/src`
    /// reads `Entry.spec.name` again after `restore` builds it (decision 17's restart,
    /// task M6.7, is the first thing that will), so no test anywhere could catch a
    /// transposition there through a public accessor, because there isn't one yet.
    ///
    /// `Entry` is private to `manager` and this module (`manager::restore`) is one of its
    /// descendants, so — exactly like `restore`'s own `crate::lock(&self.inner)` above —
    /// this reaches the field directly rather than waiting for a public path to exist.
    #[test]
    fn restore_sets_entrys_spec_name_to_the_records_own_name() {
        let m = manager();
        m.restore(StateFile {
            version: state::STATE_VERSION,
            next_id: 2,
            windows: vec![WindowRecord {
                id: 1,
                name: "record-name".into(),
                runtime: Runtime::Shell,
                cwd: PathBuf::from("/tmp"),
                project: None,
                worktree: None,
                model: None,
                initial_prompt: None,
                session_id: Some("session-id-value".into()),
                created_at: 1,
                status: Status::Exited,
                run: None,
            }],
            runs: Vec::new(),
        });

        let inner = crate::lock(&m.inner);
        let entry = inner.entries.get(&1).expect("restored entry present");
        assert_eq!(
            entry.spec.name.as_deref(),
            Some("record-name"),
            "Entry.spec.name must be the record's own name, not its session_id"
        );
    }

    /// A minimal record with the given id and (unsanitized) name; the fields the
    /// Major-1 tests below don't care about are filled with harmless defaults.
    fn wr(id: u32, name: &str) -> WindowRecord {
        WindowRecord {
            id,
            name: name.to_string(),
            runtime: Runtime::Shell,
            cwd: PathBuf::from("/tmp"),
            project: None,
            worktree: None,
            model: None,
            initial_prompt: None,
            session_id: None,
            created_at: 1,
            status: Status::Exited,
            run: None,
        }
    }

    /// Restores `windows` into a fresh manager and returns the set of names that survived.
    fn restored_names(windows: Vec<WindowRecord>) -> std::collections::BTreeSet<String> {
        let m = manager();
        let next_id = windows.iter().map(|w| w.id).max().unwrap_or(0) + 1;
        m.restore(StateFile {
            version: state::STATE_VERSION,
            next_id,
            windows,
            runs: Vec::new(),
        });
        let inner = crate::lock(&m.inner);
        inner.entries.values().map(|e| e.name.clone()).collect()
    }

    /// Fix wave 8, Major 1 (re-review of fix wave 6's "sanitize, do not reject" ruling):
    /// `unique_name` used to disambiguate a sanitized name only against records already
    /// inserted, never against records still to come. A record whose name *sanitizes*
    /// onto a name a later, perfectly valid record already owns took that name first, and
    /// the later, untouched record was then dropped by the plain name-collision check a
    /// few lines below `unique_name`'s call site — silently and permanently, since the
    /// next `state_snapshot` -> `state::save` never writes the dropped window back.
    ///
    /// Purely id-order dependent before this fix: reversing which id holds the bad name
    /// changed which window survived. The three inputs here are the re-review's own
    /// (`waves-6-7-re-review.md`, Major 1): an embedded escape character, a hand-edited
    /// stray space, and a 70-character name against its own 64-character truncation. Each
    /// is driven through **both** id orders — the order-dependence was the bug's own
    /// signature, so a fix that only worked one way would not be a fix.
    ///
    /// Deliberately checks "both survive, one under a disambiguated name" rather than
    /// which id ends up with which exact name: *that* part is still legitimately
    /// order-dependent (whichever record is processed first claims the plain name), and
    /// pinning it would test an implementation detail this fix does not claim to remove.
    #[test]
    fn restore_does_not_let_a_repaired_name_steal_a_later_valid_windows_name() {
        let cases: [(String, String); 3] = [
            ("a\u{1b}b".to_string(), "a_b".to_string()),
            (" api ".to_string(), "api".to_string()),
            ("x".repeat(70), "x".repeat(64)),
        ];
        for (bad, good) in cases {
            for windows in [
                vec![wr(1, &bad), wr(2, &good)],
                vec![wr(1, &good), wr(2, &bad)],
            ] {
                let names = restored_names(windows.clone());
                assert_eq!(
                    names.len(),
                    2,
                    "both records must survive for bad={bad:?} good={good:?}, \
                     windows={windows:?}, survivors={names:?}"
                );
                assert!(
                    names.contains(&good),
                    "the already-valid name must survive unchanged: bad={bad:?} \
                     good={good:?}, survivors={names:?}"
                );
                assert!(
                    names.iter().any(|n| n != &good),
                    "the repaired record must still be present, under a disambiguated \
                     name: bad={bad:?} good={good:?}, survivors={names:?}"
                );
            }
        }
    }
}
