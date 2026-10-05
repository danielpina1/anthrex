//! Task M9.6.15's pure round state: what a round's caps count, the amendment's base,
//! what its plan owes, and ruling T4-4's merge of an approved amendment.

use proto::{DocAuthor, DocKind, RoundDesign};

use super::*;
use crate::run::design::state::{DesignState, DocVersion};

fn req(id: &str, text: &str) -> Requirement {
    Requirement {
        id: id.into(),
        text: text.into(),
    }
}

fn version(kind: DocKind, n: u32) -> DocVersion {
    DocVersion {
        kind,
        n,
        author: DocAuthor::Orchestrator,
        reason: "submitted".into(),
        bytes: 1,
        sha256: String::new(),
        at: 0,
        requirements: Vec::new(),
        disputed: Vec::new(),
        not_reviewed: None,
        changes: Vec::new(),
        same_runtime: false,
        report: None,
        draft_review: None,
    }
}

/// Round 1 approved R1 and R2 (spec v1, plan v1, one rethink); round 2 starts.
fn in_round_two(mode: RoundDesign) -> DesignState {
    let mut design = DesignState {
        requirements: vec![req("R1", "one"), req("R2", "two")],
        approved_spec: Some(1),
        rethinks: 1,
        versions: vec![version(DocKind::Spec, 1), version(DocKind::Plan, 1)],
        ..DesignState::default()
    };
    design.round = Some(DesignRound::starting(&design, 2, mode));
    design
}

#[test]
fn a_rounds_caps_count_its_own_versions_and_rethinks() {
    let mut design = in_round_two(RoundDesign::Amend);
    assert_eq!(design.round_versions(DocKind::Spec), 0);
    assert_eq!(design.round_rethinks(), 0);
    design.versions.push(version(DocKind::Spec, 2));
    design.rethinks = 2;
    assert_eq!(design.round_versions(DocKind::Spec), 1);
    assert_eq!(design.gate_versions(DocKind::Spec), 2);
    assert_eq!(design.round_rethinks(), 1);
}

#[test]
fn round_one_counts_everything_and_owes_every_requirement() {
    let design = DesignState {
        requirements: vec![req("R1", "one")],
        versions: vec![version(DocKind::Plan, 1)],
        ..DesignState::default()
    };
    assert_eq!(design.round_versions(DocKind::Plan), 1);
    assert_eq!(design.owed(), design.requirements);
    assert_eq!(design.amend_base(), &design.requirements[..]);
    assert!(!design.amending() && !design.round_off());
}

/// Ruling T4-4: a changed requirement replaces its text and is marked once; a new one
/// is appended; the plan owes those two only, and the base is kept.
#[test]
fn an_approved_amendment_merges_into_its_base() {
    let mut design = in_round_two(RoundDesign::Amend);
    design.requirements.clear();
    design.store_requirements(vec![req("R2", "two, again"), req("R3", "three")]);
    assert_eq!(
        design.requirements,
        vec![
            req("R1", "one"),
            req("R2", "two, again (changed in round 2)"),
            req("R3", "three"),
        ]
    );
    let owed: Vec<String> = design.owed().into_iter().map(|r| r.id).collect();
    assert_eq!(owed, ["R2", "R3"]);
    assert_eq!(design.amend_base().len(), 2);
    design.store_requirements(vec![req("R2", "two (changed in round 2)")]);
    assert_eq!(design.requirements[1].text, "two (changed in round 2)");
}

/// Ruling T8-6: a full round's first pack follows every earlier brainstorm round.
#[test]
fn a_full_rounds_pack_follows_the_earlier_ones() {
    let mut design = DesignState {
        rethinks: 1,
        pack: Some(crate::run::design::pack::FrozenPack {
            round: 2,
            ..Default::default()
        }),
        ..DesignState::default()
    };
    assert_eq!(design.brainstorm_round(), 2);
    design.round = Some(DesignRound::starting(&design, 2, RoundDesign::Full));
    assert_eq!(design.brainstorm_round(), 3);
    design.rethinks = 2;
    assert_eq!(design.brainstorm_round(), 4);
    assert!(in_round_two(RoundDesign::Off).round_off());
}
