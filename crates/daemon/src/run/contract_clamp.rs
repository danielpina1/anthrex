//! The diff clamp (decision 35, ruling Q4): the reviewer's and the hand-over's diffs,
//! cut in the middle on character boundaries. Pure (design decision 2).

/// Decision 35 and ruling Q4: the reviewer's diff, and decision 30's hand-over diff, are
/// clamped to this many bytes.
pub const REVIEW_DIFF_MAX: usize = 16 * 1024;

/// The line [`clamp_diff`] puts where it cut the middle out of a diff.
pub const DIFF_CUT_MARKER: &str = "\n[anthrex: the middle of this diff was cut to fit]\n";

/// A head-and-tail clamp on character boundaries: `text` itself when it is at most
/// `max` bytes, else its first part, [`DIFF_CUT_MARKER`] and its last part, together at
/// most `max` bytes and never more than 3 bytes short of it (the most a UTF-8 cut can
/// cost, since the tail takes whatever the head's cut left over).
pub fn clamp_diff(text: &str, max: usize) -> String {
    clamp_with(text, max, DIFF_CUT_MARKER)
}

/// [`clamp_diff`] with another marker line; `messages::clamp` uses it (decision 29).
pub fn clamp_with(text: &str, max: usize, marker: &str) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    if max <= marker.len() {
        return text[..floor_boundary(text, max)].to_string();
    }
    let budget = max - marker.len();
    let head_end = floor_boundary(text, budget / 2);
    let tail_len = budget - head_end;
    let tail_start = ceil_boundary(text, text.len() - tail_len);
    let mut out = String::with_capacity(max);
    out.push_str(&text[..head_end]);
    out.push_str(marker);
    out.push_str(&text[tail_start..]);
    out
}

pub(crate) fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}
