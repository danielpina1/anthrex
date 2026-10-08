//! `Batch::amend_task` and its re-resolution, moved out of `edits.rs` (milestone 9.3,
//! task 3) so the batch's edits stay under the file-size limit. Pure, like its parent.

use proto::{PlanEdit, PlanTask, Size};

use super::{Batch, EditConsequence};
use crate::run::contract::amend_message;
use crate::run::edits_state::{has_live_worker, has_started, is_paused, not_started};
use crate::run::orch::EditSource;
use crate::run::plan::PlanError;
use crate::run::validate::resolve_task_lenient;
use crate::run::validate_patterns;

impl Batch {
    /// `amend_task`: brief, acceptance and priority on any unfinished task; route, test
    /// mode, its reason, size, deps and stage only on a task that has not started (one
    /// refusal per such field; nothing is applied); race and pair only on one never
    /// dispatched (milestone 9.5). An amend naming no field is refused.
    /// When route, test mode, its reason or size changed, the task's derived fields are
    /// re-resolved from the spec (decisions 8–10), but never below the engine's own
    /// changes (fix round 1, F1): a size the engine raised (rung 3) is a floor, even for
    /// an explicit smaller `size`; an escalated route (rung 2) stays unless the amend
    /// names `route`; the engine's notes stay. Otherwise only the spec changes. Either
    /// way the spec is validated as a plan task's is. A new brief or new criteria reach
    /// a live worker as `amend_message`.
    pub(super) fn amend_task(&mut self, edit: &PlanEdit) {
        let PlanEdit::AmendTask {
            task_id,
            brief,
            acceptance,
            route,
            test_mode,
            test_mode_reason,
            priority,
            size,
            deps,
            stage,
            race,
            pair,
        } = edit
        else {
            return;
        };
        let Some(i) = self.find(task_id) else { return };
        // Milestone 9.8 decision 31: the orchestrator's and a sub-planner's route is
        // ignored, so it neither changes the task nor is refused on a started one.
        let ignored = route.is_some() && self.source != EditSource::User;
        let route = match route {
            Some(sent) if ignored => {
                self.route_ignored(sent);
                &None
            }
            route => route,
        };
        let nothing = brief.is_none()
            && acceptance.is_none()
            && route.is_none()
            && test_mode.is_none()
            && test_mode_reason.is_none()
            && priority.is_none()
            && size.is_none()
            && deps.is_none()
            && stage.is_none()
            && race.is_none()
            && pair.is_none();
        // An amend that named only the ignored route is a no-op, not an error.
        if nothing && ignored {
            return;
        }
        if nothing {
            self.errors.push(PlanError::new(
                Some(task_id),
                "amend_task",
                "13",
                "nothing to amend",
            ));
            return;
        }
        let state = self.run.tasks[i].state;
        if state.is_finished() {
            return self.refuse(i, "only unfinished tasks can be amended");
        }
        // Milestone 9.5 ruling RR-8: race and pair change only before dispatch (after
        // 9.3's earlier-round refusal, which `apply_edits` runs first, ruling RR-7),
        // reported beside the restricted fields' refusals (review m1).
        let refused = validate_patterns::amend_refusals(&self.run.tasks[i], *race, *pair);
        let race_or_pair_refused = !refused.is_empty();
        self.errors.extend(refused);
        let restricted = [
            ("route", route.is_some()),
            ("test_mode", test_mode.is_some()),
            ("test_mode_reason", test_mode_reason.is_some()),
            ("size", size.is_some()),
            ("deps", deps.is_some()),
            ("stage", stage.is_some()),
        ];
        let reresolve = restricted[..4].iter().any(|(_, set)| *set);
        if restricted.iter().any(|(_, set)| *set) && !not_started(&self.run.tasks[i]) {
            for (name, _) in restricted.iter().filter(|(_, set)| *set) {
                let why = format!("{name} can be amended only on pending, queued or blocked tasks");
                self.refuse_unstarted(i, &format!("its {name} cannot change"), &why);
            }
            return;
        }
        // Ruling C-14 (d): a blocked task with a worktree has started too.
        if stage.is_some() && has_started(&self.run.tasks[i]) {
            let text = format!("task {task_id} has started: its stage cannot change");
            self.errors
                .push(PlanError::new(Some(task_id), "", "13", text));
            return;
        }
        if race_or_pair_refused {
            return;
        }
        // Controller ruling (fix round 1 of M9.8.7a): a user's route that names a
        // runtime or a strength but no model would be ignored (decision 10: the row
        // fills it), so it is refused.
        if let Some(r) = route.as_ref().filter(|r| r.model.is_none())
            && matches!(self.source, EditSource::User)
            && (r.runtime.is_some() || r.strength.is_some())
        {
            let what = if r.runtime.is_some() {
                "runtime"
            } else {
                "strength"
            };
            let text = format!("choose a model; {what} alone no longer selects one");
            self.errors
                .push(PlanError::new(Some(task_id), "route.model", "route", text));
            return;
        }
        // Preflight ruling F24: a route's effort reaches `--effort` as given (decision 10
        // keeps a user's route), so it must be an effort name (decision 5), or the
        // model's default.
        if let Some(effort) = route.as_ref().and_then(|r| r.effort.as_ref())
            && !(effort.is_default() || proto::models::valid_effort(effort.as_str()))
        {
            let text = format!(
                "{:?} is not an effort name (1 to {} characters of a-z, 0-9, _ and -)",
                effort.as_str(),
                proto::models::EFFORT_MAX_CHARS
            );
            let error = PlanError::new(Some(task_id), "route.effort", "route", text);
            self.errors.push(error);
            return;
        }

        let mut spec = self.run.tasks[i].spec.clone();
        let mut changed = Vec::new();
        if let Some(v) = brief {
            spec.brief = v.clone();
            changed.push("brief");
        }
        if let Some(v) = acceptance {
            spec.acceptance = v.clone();
            changed.push("acceptance");
        }
        if let Some(v) = route {
            spec.route = v.clone();
            changed.push("route");
        }
        if let Some(v) = test_mode {
            spec.test_mode = Some(*v);
            changed.push("test_mode");
        }
        if let Some(v) = test_mode_reason {
            spec.test_mode_reason = Some(v.clone());
            changed.push("test_mode_reason");
        }
        if let Some(v) = priority {
            spec.priority = *v;
            changed.push("priority");
        }
        if let Some(v) = size {
            spec.size = *v;
            changed.push("size");
        }
        if let Some(v) = stage {
            spec.stage = *v;
            changed.push("stage");
        }
        // Milestone 9.5 decisions 17 and 24: validated with the spec.
        if let Some(v) = race {
            spec.race = *v;
            changed.push("race");
        }
        if let Some(v) = pair {
            spec.pair = *v;
            changed.push("pair");
        }

        if !reresolve {
            let resolved = self.resolve(spec);
            self.run.tasks[i].spec = resolved.spec;
        } else {
            self.reresolve(i, spec, route.is_some());
        }
        if let Some(deps) = deps {
            self.amend_deps(i, deps, &mut changed);
        }
        // Milestone 9.5 rulings T17a-3, T17a-4: what decides the race changed before
        // the task started, so its next dispatch decides again. A size re-resolves the
        // route, and with it the second racer (task 17a's second re-review, (d)).
        let decides = race.is_some() || pair.is_some() || route.is_some() || size.is_some();
        let task = &mut self.run.tasks[i];
        if (decides || stage.is_some() || deps.is_some()) && !has_started(task) {
            task.race_decision = None;
            task.race_wait_since = None;
        }
        let task = &self.run.tasks[i];
        // Decision 42c: a new brief or acceptance also releases a paused task.
        let reaches = brief.is_some() || acceptance.is_some();
        if reaches && (has_live_worker(task) || is_paused(task)) {
            self.consequences.push(EditConsequence::Deliver {
                task_id: task.id().to_string(),
                text: amend_message(task),
            });
            self.release_pause(i);
        }
        self.log(i, format!("amended: {}", changed.join(", ")));
    }

    /// Re-resolves task `i` from its amended `spec` without undoing the engine: what the
    /// unamended spec resolves to is the plan's part, and anything the task holds beyond
    /// it (a larger size, a different route, extra notes) is the engine's.
    fn reresolve(&mut self, i: usize, spec: PlanTask, route_named: bool) {
        let run = &self.run;
        let old = &run.tasks[i];
        let (planned, _) = resolve_task_lenient(old.spec.clone(), &run.profile, &run.limits);
        // The recorded rung-3 raise, never a guess from the spec (fix round 2, N1).
        let floor = old.raised_size.unwrap_or(Size::S);
        let escalated = (old.route != planned.route).then(|| old.route.clone());
        let engine_notes: Vec<String> = old
            .notes
            .iter()
            .filter(|n| !planned.notes.contains(n))
            .cloned()
            .collect();

        let mut sized = spec.clone();
        sized.size = sized.size.max(floor);
        let resolved = self.resolve(sized);
        // A route named again replaces an escalated one.
        let route = match escalated {
            Some(route) if !route_named => route,
            _ => resolved.route,
        };
        // Milestone 9.8 decision 27: the reviewer row against the task's route.
        let models = self.run.limits.models();
        let review_route = (resolved.review_level).map(|_| models.reviewer_route(&route).0);
        let task = &mut self.run.tasks[i];
        task.review_route = review_route;
        task.spec = spec;
        task.size = resolved.size;
        task.hub = resolved.hub;
        task.test_mode = resolved.test_mode;
        task.review_level = resolved.review_level;
        task.route = route;
        task.budget = resolved.budget;
        task.notes = resolved.notes;
        for note in engine_notes {
            if !task.notes.contains(&note) {
                task.notes.push(note);
            }
        }
    }
}
