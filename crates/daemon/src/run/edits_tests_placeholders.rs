//! M9.2 review ruling 8, pinning: the edits milestone 9 adds to the wire but implements
//! in later tasks (M9.4's `amend_task` deps, M9.13a's `message` and `refresh`) refuse
//! the whole batch they are in, and the run is left as it was.

use proto::{MessageKind, MessageTarget};

use super::*;

const NOT_YET: &str = "op: amend deps, message and refresh are not available yet";

fn message() -> PlanEdit {
    PlanEdit::Message {
        to: MessageTarget::Tasks(vec!["t1".into()]),
        text: "the API changed".into(),
        kind: MessageKind::Change,
    }
}

fn refresh() -> PlanEdit {
    PlanEdit::Refresh {
        task_id: "t1".into(),
    }
}

fn amend_deps() -> PlanEdit {
    match amend("t2", Amend::default()) {
        PlanEdit::AmendTask {
            task_id,
            brief,
            acceptance,
            route,
            test_mode,
            test_mode_reason,
            priority,
            size,
            ..
        } => PlanEdit::AmendTask {
            task_id,
            brief,
            acceptance,
            route,
            test_mode,
            test_mode_reason,
            priority,
            size,
            deps: Some(vec!["t1".into()]),
        },
        other => panic!("an amend: {other:?}"),
    }
}

#[test]
fn placeholder_edits_refuse_their_whole_batch() {
    let run = flat();
    let before = run.clone();
    for batch in [
        vec![message()],
        vec![refresh()],
        vec![amend_deps()],
        vec![PlanEdit::Pause, message()],
        vec![PlanEdit::Finish, refresh()],
        // A valid edit in the same batch is not applied either.
        vec![cancel("t3"), message()],
    ] {
        let errors = rejected(&run, batch.clone());
        assert_eq!(errors, [NOT_YET], "{batch:?}");
        assert_eq!(run, before, "{batch:?}");
    }
}
