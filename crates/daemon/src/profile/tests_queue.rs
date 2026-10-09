//! Milestone 9.10 task M9.10.4: the goal queue's pure part (decisions 5, 6 and 10) and
//! its file (decision 4), in a scratch directory.

use std::path::PathBuf;

use proto::{CheckProgress, ProposalState, SetupState};

use super::queue::{
    DROPPED_KEPT, GoalQueue, QUEUE_FILE, QueuedGoal, info, load, next_id, save, setup_state,
};

pub(super) fn goal(project: &std::path::Path, text: &str, yes: bool) -> QueuedGoal {
    QueuedGoal {
        id: next_id(1),
        project: project.to_path_buf(),
        dir: project.to_path_buf(),
        goal: text.to_string(),
        queued_at: 1_790_000_000,
        yes,
        trust_project: false,
        unconfined_checks: false,
        orchestrator: None,
        delivery: None,
        design: None,
    }
}

#[test]
fn setup_state_follows_the_proposal() {
    let progress = Some(CheckProgress { done: 2, total: 4 });
    assert_eq!(setup_state(None, None), SetupState::Reading);
    assert_eq!(
        setup_state(Some(&ProposalState::Preparing), progress),
        SetupState::Reading
    );
    assert_eq!(
        setup_state(Some(&ProposalState::Scouting), None),
        SetupState::Reading
    );
    assert_eq!(
        setup_state(Some(&ProposalState::Verifying), progress),
        SetupState::Checking { progress }
    );
    assert_eq!(
        setup_state(Some(&ProposalState::Verifying), None),
        SetupState::Checking { progress: None }
    );
    assert_eq!(
        setup_state(Some(&ProposalState::Ready), progress),
        SetupState::NeedsReview
    );
    let failed = ProposalState::Failed {
        reason: "the scout gave up".into(),
    };
    assert_eq!(
        setup_state(Some(&failed), None),
        SetupState::Failed {
            reason: "the scout gave up".into()
        }
    );
}

#[test]
fn info_carries_the_goal_and_its_setup() {
    let project = PathBuf::from("/work/app");
    let queued = goal(&project, "add a flag", true);
    let shown = info(&queued, &SetupState::NeedsReview);
    assert_eq!(shown.id, queued.id);
    assert_eq!(shown.project, project);
    assert_eq!(shown.goal, "add a flag");
    assert_eq!(shown.queued_at, 1_790_000_000);
    assert!(shown.yes);
    assert_eq!(shown.setup, SetupState::NeedsReview);
}

#[test]
fn a_queue_survives_save_and_load() {
    let dir = tempfile::tempdir().unwrap();
    let project = PathBuf::from("/work/app");
    // A missing file loads empty.
    assert_eq!(load(dir.path()).unwrap(), GoalQueue::default());

    let mut queue = GoalQueue {
        goals: vec![
            goal(&project, "first", false),
            goal(&project, "second", true),
        ],
        dropped: Vec::new(),
    };
    queue.record_drop("third", "the repository is gone", 7);
    save(dir.path(), &queue).unwrap();
    assert!(dir.path().join(QUEUE_FILE).is_file());
    assert_eq!(load(dir.path()).unwrap(), queue);

    // An empty queue removes the file, and saving it again is no error.
    save(dir.path(), &GoalQueue::default()).unwrap();
    assert!(!dir.path().join(QUEUE_FILE).exists());
    save(dir.path(), &GoalQueue::default()).unwrap();
    assert_eq!(load(dir.path()).unwrap(), GoalQueue::default());
}

#[test]
fn a_queue_file_that_does_not_parse_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(QUEUE_FILE), "not json").unwrap();
    assert!(load(dir.path()).is_err());
}

#[test]
fn record_drop_keeps_the_last_five() {
    let mut queue = GoalQueue::default();
    for n in 0..(DROPPED_KEPT as u64 + 2) {
        queue.record_drop(&format!("goal {n}"), "refused", n);
    }
    assert_eq!(queue.dropped.len(), DROPPED_KEPT);
    let kept: Vec<_> = queue.dropped.iter().map(|d| d.goal.as_str()).collect();
    assert_eq!(kept, ["goal 2", "goal 3", "goal 4", "goal 5", "goal 6"]);
    assert_eq!(queue.dropped[4].reason, "refused");
    assert_eq!(queue.dropped[4].at, 6);
}

#[test]
fn ids_are_unique() {
    let (first, second) = (next_id(42), next_id(42));
    assert_ne!(first, second);
    assert!(first.starts_with("q-42-"), "{first}");
    assert!(second.starts_with("q-42-"), "{second}");
}
