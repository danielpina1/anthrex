//! M9.2 review ruling 8, pinning: the edits milestone 9 adds to the wire but implements
//! in a later task (M9.13a's `message` and `refresh`) refuse the whole batch they are
//! in, and the run is left as it was. M9.4 implemented `amend_task` deps
//! (`edits_tests_orch.rs`), so it left this list.

use proto::{MessageKind, MessageTarget};

use super::*;

const NOT_YET: &str = "op: message and refresh are not available yet";

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

#[test]
fn placeholder_edits_refuse_their_whole_batch() {
    let run = flat();
    let before = run.clone();
    for batch in [
        vec![message()],
        vec![refresh()],
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
