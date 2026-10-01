//! Decisions 20, 21 and 25: the steps a tier job runs, each command substituted.
//! Pure.

use super::command::{Piece, Placeholders, filter_expr, pieces, substitute};
use super::{Affected, Scope, Step, StepKind, TierPlan, TierProfile};

/// Whether `template` has a `{filter:…}` slot.
fn has_filter(template: &str) -> bool {
    pieces(template)
        .iter()
        .any(|piece| matches!(piece, Piece::Filter(_)))
}

/// Builds the steps of one plan.
struct Steps<'a> {
    tiers: &'a TierProfile,
    scope: Scope,
    key: String,
    steps: Vec<Step>,
}

impl Steps<'_> {
    /// A test-running step. A timing step is always exclusive; any other step is
    /// exclusive when it would run the timing tests without a slot to leave them out
    /// (decision 25).
    fn push(&mut self, kind: StepKind, template: &str, values: Placeholders) {
        let timing = kind == StepKind::Timing;
        let exclusive = timing || (self.tiers.timing_tests.is_some() && !has_filter(template));
        let values = Placeholders {
            filter: filter_expr(self.tiers, self.scope, timing),
            ..values
        };
        self.steps.push(Step {
            kind,
            command: substitute(template, &values),
            exclusive,
            affected_key: self.key.clone(),
        });
    }

    fn build(&mut self, build_check: &str) {
        self.steps.push(Step {
            kind: StepKind::Build,
            command: build_check.to_string(),
            exclusive: false,
            affected_key: "-".to_string(),
        });
    }

    /// `check` as one tests step, the whole suite as shard 1 of 1, then its timing step
    /// when it can leave the timing tests out and they are set.
    fn check(&mut self, check: &str) {
        let whole = || Placeholders {
            shard: Some((1, 1)),
            ..Placeholders::default()
        };
        self.push(StepKind::Tests, check, whole());
        self.timing_of(check, whole);
    }

    fn timing_of(&mut self, template: &str, values: impl Fn() -> Placeholders) {
        if self.tiers.timing_tests.is_some() && has_filter(template) {
            self.push(StepKind::Timing, template, values());
        }
    }
}

/// Decision 21: the steps tier `tier` runs for `affected` (tier 3's scope is `full`,
/// every other tier's `gate`), with decision 20's degradations.
pub fn plan(tier: u8, affected: &Affected, tiers: &TierProfile, check: Option<&str>) -> TierPlan {
    let scope = if tier >= 3 { Scope::Full } else { Scope::Gate };
    let mut steps = Steps {
        tiers,
        scope,
        key: affected.key(),
        steps: Vec::new(),
    };
    match (scope, check) {
        (Scope::Full, None) => {}
        (Scope::Full, Some(check)) => full(&mut steps, check),
        (Scope::Gate, _) => gate(&mut steps, affected, check),
    }
    TierPlan {
        scope,
        affected: affected.clone(),
        steps: steps.steps,
    }
}

/// Scope `full`: `check`, as `full_shards` shard steps when above 1, then the timing
/// step.
fn full(steps: &mut Steps<'_>, check: &str) {
    let shards = steps.tiers.full_shards;
    if shards <= 1 {
        steps.check(check);
        return;
    }
    for k in 1..=shards {
        let values = Placeholders {
            shard: Some((k, shards)),
            ..Placeholders::default()
        };
        steps.push(StepKind::Shard { k, of: shards }, check, values);
    }
    steps.timing_of(check, || Placeholders {
        shard: Some((1, 1)),
        ..Placeholders::default()
    });
}

/// Scope `gate` (tiers 1 and 2).
fn gate(steps: &mut Steps<'_>, affected: &Affected, check: Option<&str>) {
    let tiers = steps.tiers;
    let Some(build_check) = tiers.build_check.as_deref() else {
        // No build_check: `check` is the whole tier.
        if let Some(check) = check {
            steps.check(check);
        }
        return;
    };
    let names = match affected {
        // `check` is the full gate by definition: build_check does not run. Without a
        // `check` only build_check is left (see the M9.1.6 implementation notes).
        Affected::Full(_) => {
            match check {
                Some(check) => steps.check(check),
                None => steps.build(build_check),
            }
            return;
        }
        Affected::Modules(names) => names,
    };
    steps.build(build_check);
    if names.is_empty() {
        return;
    }
    let list: Vec<String> = names.iter().cloned().collect();
    if let Some(template) = tiers.module_tests.as_deref() {
        let all = || Placeholders {
            modules: Some(list.clone()),
            ..Placeholders::default()
        };
        steps.push(StepKind::Tests, template, all());
        steps.timing_of(template, all);
    } else if let Some(template) = tiers.module_test.as_deref() {
        let one = |name: &String| Placeholders {
            module: Some(name.clone()),
            ..Placeholders::default()
        };
        for name in &list {
            steps.push(StepKind::Tests, template, one(name));
        }
        for name in &list {
            steps.timing_of(template, || one(name));
        }
    } else if let Some(check) = check {
        // No module command: `check` in place of the tests.
        steps.check(check);
    }
}
