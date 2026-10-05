//! Task M9.6.9 fix round 2 (ruling T9-2): a report's own `## Appendix: the drafts`
//! heading, told from the engine's appendix by its shape; the wake after a read-back
//! that a refused report waited for; and a `Done` brainstormer with no stored draft.

use proto::{DocGateAction, DocGateKind, DocKind};

use super::design_agents::*;
use super::design_fixture::*;
use super::design_report::{NOT_IN, both_drafts, codex_draft, writes};
use super::fixture::*;
use crate::run::design::report::{APPENDIX, attach};
use crate::run::engine::design::report::{CUT_REFUSAL, CUT_WARNING};
use crate::run::engine::{DocChecked, EventKind};

/// The wake note after a read-back a refused report waited for.
const READ_BACK: &str = "the brainstorm drafts are read back; submit the report again";

fn warnings(fx: &Fixture) -> usize {
    log_lines(fx).iter().filter(|l| *l == CUT_WARNING).count()
}

fn changes(fx: &mut Fixture) {
    let changes = DocGateAction::Changes {
        note: "Again.".into(),
        review: false,
    };
    act(fx, DocGateKind::Brainstorm, changes).unwrap();
}

/// T9-2(a): an own appendix heading that cuts off a required section is refused with
/// the cause; a section missing for any other reason keeps the template's refusal.
#[test]
fn an_own_appendix_heading_that_cuts_a_section_is_refused_with_the_cause() {
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    let questions = "## Questions for you";
    let cut = REPORT.replace(questions, &format!("{APPENDIX}\nmine\n\n{questions}"));
    assert_eq!(refused(&submit(&mut fx, "brainstorm", &cut)), CUT_REFUSAL);
    assert_eq!(
        CUT_REFUSAL,
        "the appendix is the engine's; leave \"## Appendix: the drafts\" out"
    );
    let missing = REPORT.replace("## Questions for you\n", "");
    assert_eq!(
        refused(&submit(&mut fx, "brainstorm", &missing)),
        "the brainstorm is missing the section \"## Questions for you\""
    );
    assert_eq!(gate(&fx), None);
}

/// T9-2(b): the cut part is the engine's appendix by its shape, every `### ` heading in
/// it a brainstormer's label: a stale one sent back (another round's drafts) is cut
/// silently; text of the report's own after the heading warns.
#[test]
fn the_engines_appendix_is_known_by_its_shape() {
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    submitted(&mut fx, "brainstorm", REPORT);
    changes(&mut fx);
    let stale = attach(
        REPORT,
        &[
            ("claude".into(), Ok("## Understanding\nround one\n".into())),
            ("codex".into(), Err("it crashed".into())),
        ],
    );
    submitted(&mut fx, "brainstorm", &stale);
    assert_eq!(warnings(&fx), 0, "{:?}", log_lines(&fx));
    changes(&mut fx);
    let foreign = format!("{REPORT}\n{APPENDIX}\n\n### claude\nok\n### Extra notes\nmine\n");
    submitted(&mut fx, "brainstorm", &foreign);
    assert_eq!(warnings(&fx), 1);
    changes(&mut fx);
    let text = format!("{REPORT}\n{APPENDIX}\nmy own notes\n");
    submitted(&mut fx, "brainstorm", &text);
    assert_eq!(warnings(&fx), 2);
}

/// T9-2(c): a report refused while the restore's read-back was pending is followed,
/// once the read-back lands, by one wake note; with no refused report, none.
#[test]
fn a_report_refused_during_the_read_back_is_woken_once_it_lands() {
    let restarted = |refuse: bool| {
        let mut fx = brainstorming();
        both_drafts(&mut fx);
        submitted(&mut fx, "brainstorm", REPORT);
        changes(&mut fx);
        fx.run_mut().orch.design.as_mut().unwrap().texts.clear();
        if refuse {
            assert_eq!(refused(&submit(&mut fx, "brainstorm", REPORT)), NOT_IN);
        }
        let read = |n: u32, text: String| DocChecked {
            kind: DocKind::BrainstormDraft,
            n,
            read: Ok(Some(text)),
        };
        for _ in 0..2 {
            fx.next(EventKind::DesignChecked {
                run_id: RUN_ID.into(),
                checked: vec![read(1, DRAFT.into()), read(2, codex_draft())],
            });
        }
        notes(&fx).iter().filter(|n| *n == READ_BACK).count()
    };
    assert_eq!(restarted(true), 1);
    assert_eq!(restarted(false), 0);
}

/// Ruling T9-2's nit: a `Done` brainstormer with no stored draft (no writer makes one
/// today) is attached as unread rather than refusing the report forever.
#[test]
fn a_done_brainstormer_without_a_stored_draft_is_attached_as_not_stored() {
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    let design = fx.run_mut().orch.design.as_mut().unwrap();
    design
        .versions
        .retain(|v| !(v.kind == DocKind::BrainstormDraft && v.label() == Some("codex")));
    let effects = submit(&mut fx, "brainstorm", REPORT);
    let drafts = vec![
        ("claude".to_string(), Ok(DRAFT.to_string())),
        (
            "codex".to_string(),
            Err("its draft was not stored".to_string()),
        ),
    ];
    assert_eq!(writes(&effects)[0].1, attach(REPORT, &drafts));
}

/// The final fix wave's FW-8 (carry L194): a rethink clears a wake a refused report was
/// owed, as it clears `drafts_settled`. The flag is set by hand at the open gate (a
/// refusal sets it only in brainstorming or at a revising gate, where no rethink is
/// taken), then a rethink, the new drafts, and a spec approval (whose read-back runs
/// `read_back`): no stale "drafts are read back" note.
#[test]
fn a_rethink_clears_a_read_back_wake_owed() {
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    submitted(&mut fx, "brainstorm", REPORT);
    fx.run_mut().orch.design.as_mut().unwrap().read_back_owed = true;
    let rethink = DocGateAction::Rethink {
        note: "Again.".into(),
    };
    act(&mut fx, DocGateKind::Brainstorm, rethink).unwrap();
    redrafts_in(&mut fx);
    submitted(&mut fx, "brainstorm", REPORT);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::APPROVE).unwrap();
    submitted(&mut fx, "spec", SPEC);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    super::design_plan_fixture::read_back(&mut fx, 1, SPEC);
    let stale = notes(&fx).iter().filter(|n| *n == READ_BACK).count();
    assert_eq!(stale, 0, "{:?}", notes(&fx));
}
