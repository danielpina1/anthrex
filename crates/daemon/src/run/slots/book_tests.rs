//! Milestone 9.1 task M9.1.8: the pure slot book (decisions 24, 25 and 27).

use super::*;

fn req(priority: Priority, want: Want) -> SlotRequest {
    SlotRequest {
        priority,
        critical: false,
        want,
        exclusive: false,
        label: format!("{priority:?}"),
    }
}

fn exclusive() -> SlotRequest {
    SlotRequest {
        exclusive: true,
        ..req(Priority::Gate, Want::All)
    }
}

#[test]
fn book_grants_by_priority_then_age() {
    let mut book = SlotBook::new(4);
    let idle = book.ask(req(Priority::FullIdle, Want::All));
    let candidate = book.ask(req(Priority::Candidate, Want::All));
    assert_eq!(book.grant(), [(candidate, 4)], "the candidate first");
    assert_eq!(book.grant(), [], "the idle job waits for a free slot");
    book.release(candidate);
    assert_eq!(book.grant(), [(idle, 4)]);
    book.release(idle);

    // Equal classes: in ask order.
    let first = book.ask(req(Priority::Gate, Want::All));
    let second = book.ask(req(Priority::Gate, Want::All));
    assert_eq!(book.grant(), [(first, 4)]);
    book.release(first);
    assert_eq!(book.grant(), [(second, 4)]);
    book.release(second);

    // Among equal classes a critical-path request goes first.
    let plain = book.ask(req(Priority::Gate, Want::All));
    let critical = book.ask(SlotRequest {
        critical: true,
        ..req(Priority::Gate, Want::All)
    });
    assert_eq!(book.grant(), [(critical, 4)]);
    book.release(critical);
    assert_eq!(book.grant(), [(plain, 4)]);
}

#[test]
fn a_request_gets_what_is_free_and_the_next_waits_its_turn() {
    let mut book = SlotBook::new(4);
    let one = book.ask(req(Priority::Gate, Want::One));
    assert_eq!(book.grant(), [(one, 1)]);
    // min(asked, free): an all-slots candidate starts at once with the three left.
    let candidate = book.ask(req(Priority::Candidate, Want::All));
    let gate = book.ask(req(Priority::Gate, Want::Half));
    assert_eq!(book.grant(), [(candidate, 3)]);
    book.release(one);
    assert_eq!(book.grant(), [(gate, 1)]);
    assert_eq!(book.held(), 4);
}

#[test]
fn exclusive_step_waits_for_every_slot_and_blocks_later_ones() {
    let mut book = SlotBook::new(4);
    let a = book.ask(req(Priority::Gate, Want::One));
    let b = book.ask(req(Priority::Gate, Want::One));
    assert_eq!(book.grant(), [(a, 1), (b, 1)]);
    let timing = book.ask(exclusive());
    assert_eq!(book.grant(), [], "two slots are held");
    let later = book.ask(req(Priority::Gate, Want::One));
    assert_eq!(
        book.grant(),
        [],
        "strict order: nothing passes the exclusive step"
    );
    book.release(a);
    assert_eq!(book.grant(), []);
    book.release(b);
    assert_eq!(book.grant(), [(timing, 4)], "every slot, once none is held");
    assert_eq!(book.grant(), []);
    book.release(timing);
    assert_eq!(book.grant(), [(later, 1)]);
}

#[test]
fn half_and_one_round_as_decided() {
    for (slots, half) in [(1, 1), (3, 2), (4, 2), (7, 4)] {
        assert_eq!(Want::Half.of(slots), half, "half of {slots}");
        assert_eq!(Want::One.of(slots), 1, "one of {slots}");
        assert_eq!(Want::All.of(slots), slots, "all of {slots}");
    }
}

#[test]
fn a_released_wait_gives_up_its_place() {
    let mut book = SlotBook::new(2);
    let held = book.ask(req(Priority::Gate, Want::All));
    assert_eq!(book.grant(), [(held, 2)]);
    let gone = book.ask(exclusive());
    let next = book.ask(req(Priority::Gate, Want::One));
    book.release(gone);
    book.release(held);
    assert_eq!(book.grant(), [(next, 1)]);
}

#[test]
fn load_is_held_over_slots_to_one_decimal() {
    let mut book = SlotBook::new(4);
    assert_eq!(book.load(), "0.0");
    let half = book.ask(req(Priority::Gate, Want::Half));
    book.grant();
    assert_eq!(book.load(), "0.5");
    book.ask(req(Priority::Candidate, Want::All));
    book.grant();
    assert_eq!(book.load(), "1.0");
    book.release(half);
    assert_eq!(book.load(), "0.5");

    let mut three = SlotBook::new(3);
    three.ask(req(Priority::Gate, Want::One));
    three.grant();
    assert_eq!(three.load(), "0.3");
    three.ask(req(Priority::Gate, Want::One));
    three.grant();
    assert_eq!(three.load(), "0.7");
}

#[test]
fn worker_caps_are_test_slots_over_writers_at_least_one() {
    let names = |caps: &[(String, String)]| -> Vec<String> {
        caps.iter().map(|(k, _)| k.clone()).collect()
    };
    let caps = worker_caps(8, 3);
    assert_eq!(names(&caps), SLOT_VARS);
    assert!(caps.iter().all(|(_, v)| v == "2"), "{caps:?}");
    assert!(worker_caps(2, 4).iter().all(|(_, v)| v == "1"));
    assert!(worker_caps(8, 0).iter().all(|(_, v)| v == "8"));
}
