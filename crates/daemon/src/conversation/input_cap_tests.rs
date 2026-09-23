//! Task M6.5.10 fix round 1 (F1): a tool `input` whose encoding could exceed
//! `max_result_bytes` is stored as `None`, from a hook and from a transcript alike. The
//! summary, taken from the full input first, still says what the call did.

use super::tests::{call, pre, prompt};
use crate::conversation::{Caps, ConversationSet};
use proto::Block;
use serde_json::json;
use std::time::Instant;

fn caps() -> Caps {
    Caps {
        max_result_bytes: 4096,
        ..Caps::default()
    }
}

fn the_call(set: &ConversationSet) -> (Option<serde_json::Value>, String) {
    let root = set.snapshot(None).unwrap();
    root.turns
        .iter()
        .flat_map(|t| &t.blocks)
        .find_map(|b| match b {
            Block::ToolCall { input, summary, .. } => Some((input.clone(), summary.clone())),
            _ => None,
        })
        .expect("one tool call")
}

fn feed(set: &mut ConversationSet, input: serde_json::Value) {
    for h in [prompt("write it"), pre("tu-1", "Write", input)] {
        set.on_hook(proto::Runtime::Claude, &h, None, 0, Instant::now(), caps());
    }
}

/// Just under and just over the bound, measured the way `byte_size` measures it.
fn write_input(content_len: usize) -> serde_json::Value {
    json!({"file_path": "/repo/big.rs", "content": "x".repeat(content_len)})
}

#[test]
fn a_hook_input_over_the_cap_is_not_stored() {
    let fits = write_input(4000);
    assert!(proto::conversation::value_byte_size(&fits) <= caps().max_result_bytes);
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    feed(&mut set, fits.clone());
    assert_eq!(the_call(&set).0, Some(fits));

    let over = write_input(6 * 1024 * 1024);
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    feed(&mut set, over);
    let (input, summary) = the_call(&set);
    assert_eq!(input, None);
    assert!(summary.contains("big.rs"), "{summary}");
}

#[test]
fn a_float_array_input_is_measured_by_its_encoding() {
    // 500 floats: about 2 KB of JSON, about 4.5 KB of MessagePack.
    let floats = json!({"v": vec![0.5; 500]});
    assert!(serde_json::to_string(&floats).unwrap().len() < caps().max_result_bytes);
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    feed(&mut set, floats);
    assert_eq!(the_call(&set).0, None);
}

#[test]
fn a_transcript_input_over_the_cap_is_not_applied() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    let small = json!({"file_path": "/repo/a.rs"});
    feed(&mut set, small.clone());
    set.enrich(&[call("tu-1", write_input(6 * 1024 * 1024))], caps());
    assert_eq!(the_call(&set).0, Some(small), "the hook's input stands");
}
