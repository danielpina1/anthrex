//! Sub-agent descent, the trail, closing, and what keys may send.

use super::*;

#[test]
fn enter_on_a_spawn_descends_and_esc_returns() {
    let mut view = open_with(main_conversation());
    assert!(press(&mut view, KeyCode::Char('G')).is_empty());
    assert_eq!(
        selected(&view).1,
        Row::Spawn {
            turn_id: 12,
            block: 3,
            agent_id: "agent-b".into()
        }
    );

    assert_eq!(
        press(&mut view, KeyCode::Enter),
        vec![subscribe(Some("agent-b"))]
    );
    assert_eq!(
        view.trail(),
        [Crumb {
            agent_id: "agent-b".into(),
            label: "find every call site".into()
        }]
    );
    assert_eq!(view.key(), Some((WINDOW, Some("agent-b".into()))));
    assert!(
        view.rows().is_empty(),
        "the sub-agent's snapshot has not arrived"
    );

    assert_eq!(
        press(&mut view, KeyCode::Esc),
        vec![unsubscribe(Some("agent-b"))]
    );
    assert!(view.trail().is_empty());
    assert_eq!(view.key(), Some((WINDOW, None)));
    assert_eq!(view.rows(), open_with(main_conversation()).rows());

    assert_eq!(press(&mut view, KeyCode::Esc), vec![unsubscribe(None)]);
    assert!(!view.is_open());
    assert_eq!(view.key(), None);
}

#[test]
fn the_parent_stays_live_while_a_sub_agent_is_shown() {
    let mut view = open_with(main_conversation());
    press(&mut view, KeyCode::Char('G'));
    press(&mut view, KeyCode::Enter);
    // A delta for the root, which is still subscribed, lands while agent-b is shown.
    let new_turn = turn(13, Role::User, 1_120, vec![text("now run the tests")]);
    assert!(delta(&mut view, 5, vec![TurnPatch::Upsert(new_turn)]).is_empty());
    press(&mut view, KeyCode::Esc);
    assert_eq!(view.rev(), Some(6));
    assert!(view.rows().contains(&Row::Text {
        turn_id: 13,
        block: 0,
        line: 0,
        text: "now run the tests".into()
    }));
}

#[test]
fn closing_unsubscribes_every_key_on_the_trail() {
    let mut view = open_with(main_conversation());
    press(&mut view, KeyCode::Char('G'));
    assert_eq!(
        press(&mut view, KeyCode::Enter),
        vec![subscribe(Some("agent-b"))]
    );
    view.on_snapshot(
        WINDOW,
        Some("agent-b".into()),
        sub_agent("agent-b", Some(("agent-c", "review the split"))),
    );
    press(&mut view, KeyCode::Char('G'));
    assert_eq!(
        press(&mut view, KeyCode::Enter),
        vec![subscribe(Some("agent-c"))]
    );
    assert_eq!(view.trail().len(), 2);

    assert_eq!(
        press(&mut view, KeyCode::Char('q')),
        vec![
            unsubscribe(Some("agent-c")),
            unsubscribe(Some("agent-b")),
            unsubscribe(None),
        ]
    );
    assert!(!view.is_open());
    assert!(view.trail().is_empty());
}

#[test]
fn the_view_emits_nothing_but_subscriptions() {
    let mut view = open_with(main_conversation());
    let keys = [
        key(KeyCode::Char('j')),
        key(KeyCode::Down),
        key(KeyCode::Char('k')),
        key(KeyCode::Up),
        key(KeyCode::Char('g')),
        key(KeyCode::Char('G')),
        key(KeyCode::Enter),
        key(KeyCode::Char('o')),
        key(KeyCode::Esc),
        key(KeyCode::Char('/')),
        key(KeyCode::Char('x')),
        key(KeyCode::Backspace),
        key(KeyCode::Char('e')),
        key(KeyCode::Enter),
        key(KeyCode::Char('n')),
        key(KeyCode::Char('N')),
        key(KeyCode::Tab),
        key(KeyCode::Char('i')),
        key(KeyCode::Char('y')),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        key(KeyCode::Left),
        key(KeyCode::PageDown),
        key(KeyCode::Char('q')),
    ];
    let mut effects = Vec::new();
    for round in 0..keys.len() {
        for i in 0..keys.len() {
            // Walk every key from a different starting point so each is pressed with the
            // cursor on a text, tool, spawn and header row at some point.
            let k = keys[(i + round) % keys.len()];
            if !view.is_open() {
                effects.extend(view.open(WINDOW));
                view.on_snapshot(WINDOW, None, main_conversation());
            }
            if let Some((_, Some(agent))) = view.key()
                && view.conversation().is_none()
            {
                view.on_snapshot(WINDOW, Some(agent.clone()), sub_agent(&agent, None));
            }
            effects.extend(view.on_key(k));
        }
    }
    assert!(
        effects.len() > 3,
        "the walk should have descended, returned and closed"
    );
    for effect in effects {
        assert!(
            matches!(
                effect,
                Effect::Send(ClientMsg::SubscribeConversation { .. })
                    | Effect::Send(ClientMsg::UnsubscribeConversation { .. })
            ),
            "the view sent {effect:?}"
        );
    }
}

#[test]
fn gone_for_a_sub_agent_pops_and_keeps_the_reason() {
    let mut view = open_with(main_conversation());
    press(&mut view, KeyCode::Char('G'));
    press(&mut view, KeyCode::Enter);
    let effects = view.on_gone(
        WINDOW,
        Some("agent-b".into()),
        proto::conversation::GONE_TOO_LARGE.into(),
    );
    assert!(
        effects.is_empty(),
        "the daemon already ended it: {effects:?}"
    );
    assert!(view.trail().is_empty());
    assert!(view.is_open());
    assert_eq!(view.gone_reason(), Some("conversation too large to send"));

    let effects = view.on_gone(
        WINDOW,
        None,
        proto::conversation::GONE_WINDOW_REMOVED.into(),
    );
    assert!(effects.is_empty());
    assert!(!view.is_open());
    assert_eq!(view.gone_reason(), Some("window removed"));
}

#[test]
fn j_and_k_walk_rows_and_clamp() {
    let mut view = open_with(main_conversation());
    press(&mut view, KeyCode::Char('g'));
    assert_eq!(selected(&view).0, 0);
    press(&mut view, KeyCode::Char('k'));
    assert_eq!(selected(&view).0, 0, "k clamps at the top");
    press(&mut view, KeyCode::Char('j'));
    press(&mut view, KeyCode::Down);
    assert_eq!(
        selected(&view).1,
        Row::TurnHeader {
            turn_id: 12,
            role: Role::Assistant,
            at_unix_secs: 1_060
        }
    );
    press(&mut view, KeyCode::Up);
    assert_eq!(selected(&view).0, 1);
    press(&mut view, KeyCode::Char('G'));
    press(&mut view, KeyCode::Char('j'));
    assert_eq!(selected(&view).0, 6, "j clamps at the bottom");
    // Enter on a text row does nothing.
    press(&mut view, KeyCode::Char('g'));
    press(&mut view, KeyCode::Char('j'));
    assert!(press(&mut view, KeyCode::Enter).is_empty());
    assert_eq!(view.rows().len(), 7);
}
