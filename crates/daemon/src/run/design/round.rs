//! Milestone 9.6 task M9.6.15 (decisions 28 and 29, DF §8.1): a design run's later
//! rounds. A round from round 2 on records what it does with the documents
//! ([`DesignRound`]): `amend` (specifying, then the round's plan gate), `full`
//! (brainstorming first) or `off` (9.3's round, straight to its plan gate). Its
//! amendment continues from the requirements approved before it ([`DesignRound::base`]),
//! which a back inside the round never clears (task 10's carry), and its plan covers only
//! the requirements new or changed in it ([`DesignState::owed`]). Its caps count the
//! round's own versions and rethinks (brief ruling BD-2). Pure.

use proto::{DocKind, RoundDesign};
use serde::{Deserialize, Serialize};

use super::requirements::Requirement;
use super::state::DesignState;

/// What round `n` (from 2 on) of a design run does with its documents, and what it
/// started from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignRound {
    pub n: u32,
    pub mode: RoundDesign,
    /// The requirements approved before the round: what its amendment continues from.
    #[serde(default)]
    pub base: Vec<Requirement>,
    /// The spec version approved before the round, and its Goal and Interfaces
    /// sections: what a rejected round goes back to.
    #[serde(default)]
    pub spec_before: Option<u32>,
    #[serde(default)]
    pub goal_before: String,
    #[serde(default)]
    pub interfaces_before: String,
    /// The gate versions of the brainstorm, the spec and the plan when it started.
    #[serde(default)]
    pub versions_before: [u32; 3],
    /// The brainstorm's rethinks when it started.
    #[serde(default)]
    pub rethinks_before: u32,
    /// The brainstorm rounds before it, past the run's rethinks: a full round's packs
    /// take the numbers after them (ruling T8-6's `pack-r<k>.md` never repeats).
    #[serde(default)]
    pub packs_before: u32,
    /// The ids new or changed in its approved amendment, in the amendment's order.
    #[serde(default)]
    pub amended: Vec<String>,
    /// The document reviews asked before it: a dropped round's own are marked dropped
    /// (rulings T15-2 and T15-9).
    #[serde(default)]
    pub reviews_before: usize,
}

/// Ruling T15-1: a spec the user approved: the round it was approved in (round 1's, or a
/// later round's amendment), and its spec gate version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedSpec {
    pub round: u32,
    pub version: u32,
}

/// Decision 14 and ruling T4-4: a changed requirement's marker.
pub fn marker(k: u32) -> String {
    format!("(changed in round {k})")
}

/// The index of `kind`'s gate in [`DesignRound::versions_before`].
fn slot(kind: DocKind) -> Option<usize> {
    match kind {
        DocKind::Brainstorm => Some(0),
        DocKind::Spec => Some(1),
        DocKind::Plan => Some(2),
        DocKind::BrainstormDraft => None,
    }
}

impl DesignRound {
    /// Round `n` of `design` starting now in `mode`.
    pub fn starting(design: &DesignState, n: u32, mode: RoundDesign) -> DesignRound {
        let versions = [DocKind::Brainstorm, DocKind::Spec, DocKind::Plan];
        let packs = design.pack.as_ref().map_or(0, |p| p.round);
        DesignRound {
            n,
            mode,
            base: design.requirements.clone(),
            spec_before: design.approved_spec,
            goal_before: design.goal_section.clone(),
            interfaces_before: design.interfaces_section.clone(),
            versions_before: versions.map(|k| design.gate_versions(k)),
            rethinks_before: design.rethinks,
            packs_before: packs.saturating_sub(design.rethinks),
            amended: Vec::new(),
            reviews_before: design.reviews.len(),
        }
    }
}

impl DesignState {
    /// `kind`'s gate versions in the current round (all of them in round 1): what
    /// brief ruling BD-2's cap counts.
    pub fn round_versions(&self, kind: DocKind) -> u32 {
        let before = (self.round.as_ref())
            .and_then(|r| slot(kind).map(|i| r.versions_before[i]))
            .unwrap_or(0);
        self.gate_versions(kind).saturating_sub(before)
    }

    /// The brainstorm's rethinks in the current round.
    pub fn round_rethinks(&self) -> u32 {
        let before = self.round.as_ref().map_or(0, |r| r.rethinks_before);
        self.rethinks.saturating_sub(before)
    }

    /// The number of the brainstorm round a pack is frozen for now (ruling T8-6): one
    /// past the run's rethinks, after every earlier round's brainstorms.
    pub fn brainstorm_round(&self) -> u32 {
        let before = self.round.as_ref().map_or(0, |r| r.packs_before);
        self.rethinks + 1 + before
    }

    /// Ruling T15-1: every spec approved so far, in round order: round 1's, then each
    /// later round's amendment (the current round's once approved).
    pub fn approved_specs(&self) -> Vec<ApprovedSpec> {
        let mut specs = self.approved_before.clone();
        let round = self.round.as_ref().map_or(1, |r| r.n);
        if let Some(version) = self.approved_spec
            && specs.last().is_none_or(|s| s.version != version)
        {
            specs.push(ApprovedSpec { round, version });
        }
        specs
    }

    /// The current round plans as 9.3 does (`iterate --design off`).
    pub fn round_off(&self) -> bool {
        self.round
            .as_ref()
            .is_some_and(|r| r.mode == RoundDesign::Off)
    }

    /// The requirements an amendment (`amend: true`) continues from: the round's base,
    /// or in round 1 the approved ones.
    pub fn amend_base(&self) -> &[Requirement] {
        match &self.round {
            Some(round) => &round.base,
            None => &self.requirements,
        }
    }

    /// Whether the current round amends an approved spec (ruling T15-7, m4: an `off`
    /// round never does).
    pub fn amending(&self) -> bool {
        self.round.as_ref().is_some_and(amends)
    }

    /// Decision 29: the requirements the current plan must cover. In a round that
    /// amends the spec, the ones its amendment adds or changes; otherwise every one.
    pub fn owed(&self) -> Vec<Requirement> {
        let Some(round) = self.round.as_ref().filter(|_| self.amending()) else {
            return self.requirements.clone();
        };
        (self.requirements.iter())
            .filter(|r| round.amended.contains(&r.id))
            .cloned()
            .collect()
    }

    /// The approved spec's requirements `found` in its text (`requirements::scan`),
    /// stored. In a round that amends the spec they are merged into its base: a changed
    /// one replaces its earlier text and carries [`marker`] (ruling T4-4: added when its
    /// text lacks it), a new one is appended, and the round records their ids. One
    /// restated word for word (ruling T15-7, m2: equal after whitespace normalisation)
    /// is unchanged.
    pub fn store_requirements(&mut self, found: Vec<Requirement>) {
        let k = self.round.as_ref().map_or(1, |r| r.n);
        let Some(round) = self.round.as_mut().filter(|r| amends(r)) else {
            self.requirements = found;
            return;
        };
        let mut merged = round.base.clone();
        round.amended.clear();
        for mut r in found {
            match merged.iter_mut().find(|b| b.id == r.id) {
                Some(earlier) if normalised(&earlier.text) == normalised(&r.text) => {}
                Some(earlier) => {
                    if !r.text.contains(&marker(k)) {
                        r.text = format!("{} {}", r.text, marker(k)).trim().to_string();
                    }
                    earlier.text = r.text;
                    round.amended.push(r.id);
                }
                None => {
                    round.amended.push(r.id.clone());
                    merged.push(r);
                }
            }
        }
        self.requirements = merged;
    }
}

/// Whether `round` amends the spec: it has a base, and its design is not `off`.
fn amends(round: &DesignRound) -> bool {
    !round.base.is_empty() && round.mode != RoundDesign::Off
}

/// `text` with every run of whitespace one space, trimmed.
fn normalised(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "round_tests.rs"]
mod tests;
