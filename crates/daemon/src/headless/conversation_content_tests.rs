//! Ruling T7-N1: with Claude's hooks firing, a turn is the daemon's or Claude Code's own
//! by its prompt's text, observed from the `UserPromptSubmit` hook, not by when its
//! `Init` arrives. Every case runs through a real M6.5 `ConversationSet`.

use super::super::*;
use crate::conversation::{Caps, ConversationSet};
use crate::headless::claude_stream::ClaudeStream;
use proto::{Block, HookSource, Role};
use serde_json::json;
use std::time::Instant;

const SESSION: &str = "00000000-0000-4000-8000-000000000003";
const BG: &str = "<task-notification>agent done</task-notification>";

/// One step of a Claude session, in the order the daemon sees it.
enum Step<'a> {
    /// The daemon writes a turn with this text.
    Sent(&'a str),
    /// A real `UserPromptSubmit` hook with this prompt.
    Prompt(&'a str),
    Init,
    Text(&'a str),
    Result,
    /// A real `Stop` hook.
    Stop,
}
use Step::*;

struct Session {
    set: ConversationSet,
    cursor: StreamCursor,
    parser: ClaudeStream,
    /// Whether the manager feeds real hooks to `observe_hook` (M8a.17's wiring).
    feed: bool,
}

impl Session {
    fn new(feed: bool) -> Self {
        Session {
            set: ConversationSet::new(3, Runtime::Claude),
            cursor: StreamCursor::default(),
            parser: ClaudeStream::default(),
            feed,
        }
    }

    fn apply(&mut self, input: ConversationInput) {
        assert!(input.hooks.is_empty(), "hooks fire: nothing is synthesised");
        self.set.enrich(&input.records, Caps::default());
    }

    fn hook(&mut self, payload: serde_json::Value) {
        let hook = crate::hooks::parse(HookSource::Claude, &payload).unwrap();
        self.set.on_hook(
            Runtime::Claude,
            &hook,
            None,
            0,
            Instant::now(),
            Caps::default(),
        );
        if self.feed {
            let input = observe_hook(Runtime::Claude, &hook, &mut self.cursor);
            self.apply(input);
        }
    }

    fn line(&mut self, line: &str) {
        for event in self.parser.parse_line(line) {
            let input = map(Runtime::Claude, true, &event, &mut self.cursor);
            self.apply(input);
        }
    }

    fn run(mut self, steps: &[Step<'_>]) -> proto::Conversation {
        // Every recorded session starts with this hook (`claude-2.1.278-hooks.jsonl`), and
        // M6.5 holds no records for a window with no conversation yet.
        self.hook(
            json!({"hook_event_name": "SessionStart", "session_id": SESSION, "source": "startup"}),
        );
        for step in steps {
            match step {
                Sent(text) => {
                    let input = sent_turn(Runtime::Claude, true, text, &mut self.cursor);
                    self.apply(input);
                }
                Prompt(text) => self.hook(json!({
                    "hook_event_name": "UserPromptSubmit", "session_id": SESSION, "prompt": text
                })),
                Stop => self.hook(json!({"hook_event_name": "Stop", "session_id": SESSION})),
                Init => self.line(&format!(
                    r#"{{"type":"system","subtype":"init","session_id":"{SESSION}","model":"m"}}"#
                )),
                Text(text) => self.line(
                    &json!({"type": "assistant", "parent_tool_use_id": null,
                            "message": {"content": [{"type": "text", "text": text}]}})
                    .to_string(),
                ),
                Result => self.line(r#"{"type":"result","subtype":"success","is_error":false}"#),
            }
        }
        self.set.snapshot(None).unwrap()
    }
}

/// Each `User` turn's prompt with the prose of the reply after it.
fn pairs(conversation: &proto::Conversation) -> Vec<(String, String)> {
    let text = |blocks: &[Block]| {
        blocks
            .iter()
            .find_map(|b| match b {
                Block::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    };
    conversation
        .turns
        .chunks(2)
        .map(|pair| {
            assert_eq!(pair[0].role, Role::User);
            assert_eq!(pair[1].role, Role::Assistant);
            (text(&pair[0].blocks), text(&pair[1].blocks))
        })
        .collect()
}

fn expect(conversation: &proto::Conversation, want: &[(&str, &str)]) {
    assert_eq!(conversation.degraded, None);
    let want: Vec<(String, String)> = want
        .iter()
        .map(|(p, r)| (p.to_string(), r.to_string()))
        .collect();
    assert_eq!(pairs(conversation), want);
}

/// A daemon turn, the way M8a.1 recorded one: sent, its hook, then its stream.
fn turn<'a>(prompt: &'a str, reply: &'a str, sent: bool) -> Vec<Step<'a>> {
    let mut steps = Vec::new();
    if sent {
        steps.push(Sent(prompt));
    }
    steps.extend([Prompt(prompt), Init, Text(reply), Result, Stop]);
    steps
}

#[test]
fn the_three_turn_case_places_every_reply() {
    let steps: Vec<Step<'_>> = [
        turn("first", "reply one", true),
        turn(BG, "bg reply", false),
        turn("second", "reply two", true),
    ]
    .into_iter()
    .flatten()
    .collect();
    let conversation = Session::new(true).run(&steps);
    expect(
        &conversation,
        &[
            ("first", "reply one"),
            (BG, "bg reply"),
            ("second", "reply two"),
        ],
    );
}

#[test]
fn a_background_turn_that_runs_before_a_delivered_turn_keeps_both_aligned() {
    // Review N1: the engine delivers "second" at the first turn's end, but Claude runs a
    // queued background notification first. Timing alone took the background turn for
    // "second" and "second" for an unprompted one.
    let conversation = Session::new(true).run(&[
        Sent("first"),
        Prompt("first"),
        Init,
        Text("reply one"),
        Result,
        Stop,
        Sent("second"),
        Prompt(BG),
        Init,
        Text("bg reply"),
        Result,
        Stop,
        Prompt("second"),
        Init,
        Text("reply two"),
        Result,
        Stop,
        Sent("third"),
        Prompt("third"),
        Init,
        Text("reply three"),
        Result,
        Stop,
    ]);
    expect(
        &conversation,
        &[
            ("first", "reply one"),
            (BG, "bg reply"),
            ("second", "reply two"),
            ("third", "reply three"),
        ],
    );
}

#[test]
fn an_unprompted_turn_in_the_middle_of_a_sent_turn_keeps_both_aligned() {
    // Review case D: the background turn's prompt arrives with no `result` in between.
    let conversation = Session::new(true).run(&[
        Sent("first"),
        Prompt("first"),
        Init,
        Text("reply one"),
        Prompt(BG),
        Init,
        Text("bg reply"),
        Result,
        Stop,
        Sent("second"),
        Prompt("second"),
        Init,
        Text("reply two"),
        Result,
        Stop,
    ]);
    expect(
        &conversation,
        &[
            ("first", "reply one"),
            (BG, "bg reply"),
            ("second", "reply two"),
        ],
    );
}

#[test]
fn an_unprompted_turn_first_in_a_session_keeps_later_turns_aligned() {
    // Review case E: a fresh cursor meets a session whose first turn is Claude's own.
    let steps: Vec<Step<'_>> = [
        turn(BG, "bg reply", false),
        turn("first", "reply one", true),
    ]
    .into_iter()
    .flatten()
    .collect();
    let conversation = Session::new(true).run(&steps);
    expect(&conversation, &[(BG, "bg reply"), ("first", "reply one")]);
}

#[test]
fn a_prompt_applied_before_its_sent_turn_is_still_aligned() {
    // Review case F: the `Init` and even the hook reach the daemon before it records the
    // turn it sent. Content decides, so the order no longer matters.
    let conversation = Session::new(true).run(&[
        Prompt("first"),
        Init,
        Sent("first"),
        Text("reply one"),
        Result,
        Stop,
        Sent("second"),
        Prompt("second"),
        Init,
        Text("reply two"),
        Result,
        Stop,
    ]);
    expect(
        &conversation,
        &[("first", "reply one"), ("second", "reply two")],
    );
}

#[test]
fn prose_of_a_turn_whose_prompt_hook_was_lost_is_dropped_not_misplaced() {
    // `anthrex hook` gives up after a second, so a hook can be lost; M6.5 then builds no
    // turn, and the prose has nowhere to go. It must not land on the previous reply, and
    // the turns after it stay aligned.
    let conversation = Session::new(true).run(&[
        Sent("first"),
        Prompt("first"),
        Init,
        Text("reply one"),
        Result,
        Stop,
        Sent("lost"),
        Init,
        Text("orphan reply"),
        Result,
        Sent("second"),
        Prompt("second"),
        Init,
        Text("reply two"),
        Result,
        Stop,
    ]);
    expect(
        &conversation,
        &[("first", "reply one"), ("second", "reply two")],
    );
}

#[test]
fn a_sent_prompt_is_human_and_an_unprompted_one_is_not() {
    let mut cursor = StreamCursor::default();
    let hook = |prompt: &str| {
        crate::hooks::parse(
            HookSource::Claude,
            &json!({"hook_event_name": "UserPromptSubmit", "session_id": SESSION, "prompt": prompt}),
        )
        .unwrap()
    };
    sent_turn(Runtime::Claude, true, "  go \n", &mut cursor);
    let records = |input: ConversationInput| input.records;
    assert_eq!(
        records(observe_hook(Runtime::Claude, &hook(BG), &mut cursor)),
        [Record::UserText {
            session_id: Some(SESSION.into()),
            ordinal: 0,
            text: BG.into(),
            human: false,
        }]
    );
    // Equal up to surrounding whitespace, as M6.5's alignment has it.
    assert_eq!(
        records(observe_hook(Runtime::Claude, &hook("go"), &mut cursor)),
        [Record::UserText {
            session_id: Some(SESSION.into()),
            ordinal: 1,
            text: "go".into(),
            human: true,
        }]
    );
    // A matched text is used once.
    assert!(matches!(
        records(observe_hook(Runtime::Claude, &hook("go"), &mut cursor)).as_slice(),
        [Record::UserText {
            ordinal: 2,
            human: false,
            ..
        }]
    ));
    // A text sent earlier whose prompt hook was lost is dropped when a later sent text
    // matches, so the same words later are not taken for the daemon's.
    sent_turn(Runtime::Claude, true, "lost", &mut cursor);
    sent_turn(Runtime::Claude, true, "kept", &mut cursor);
    assert!(matches!(
        records(observe_hook(Runtime::Claude, &hook("kept"), &mut cursor)).as_slice(),
        [Record::UserText {
            ordinal: 3,
            human: true,
            ..
        }]
    ));
    assert!(matches!(
        records(observe_hook(Runtime::Claude, &hook("lost"), &mut cursor)).as_slice(),
        [Record::UserText {
            ordinal: 4,
            human: false,
            ..
        }]
    ));
    // A prompt observed before its `sent_turn` is recorded then: the text is not left
    // waiting to claim a later prompt with the same words.
    observe_hook(Runtime::Claude, &hook("early"), &mut cursor);
    sent_turn(Runtime::Claude, true, "early", &mut cursor);
    assert!(matches!(
        records(observe_hook(Runtime::Claude, &hook("early"), &mut cursor)).as_slice(),
        [Record::UserText {
            ordinal: 6,
            human: false,
            ..
        }]
    ));
    // Every other hook kind is ignored.
    let stop = crate::hooks::parse(
        HookSource::Claude,
        &json!({"hook_event_name": "Stop", "session_id": SESSION}),
    )
    .unwrap();
    assert_eq!(
        observe_hook(Runtime::Claude, &stop, &mut cursor),
        ConversationInput::default()
    );
}

#[test]
fn without_a_hook_feed_the_timing_rule_still_applies() {
    // The fallback when no prompt text is observable: M8a.7 fix round 1's rule.
    let steps: Vec<Step<'_>> = [
        turn("first", "reply one", true),
        turn(BG, "bg reply", false),
        turn("second", "reply two", true),
    ]
    .into_iter()
    .flatten()
    .collect();
    let conversation = Session::new(false).run(&steps);
    expect(
        &conversation,
        &[("first", "reply one"), (BG, ""), ("second", "reply two")],
    );
}
