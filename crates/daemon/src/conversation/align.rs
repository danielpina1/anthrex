//! Whether a transcript prompt is the prompt a hook-built `User` turn holds (task M6.5.8's
//! alignment check, widened for the prompts a runtime injects itself). Pure.

/// A transcript prompt, as `user_text` judges it.
pub(crate) struct Prompt<'a> {
    pub(crate) session_id: Option<&'a str>,
    pub(crate) text: &'a str,
    /// Whether a person typed it, rather than the runtime injecting it.
    pub(crate) human: bool,
}

/// How a transcript prompt lines up with a hook-built `User` turn's prompt.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Alignment {
    /// Equal up to surrounding whitespace.
    Exact,
    /// An injected prompt that wraps the hook's prompt in the runtime's own text.
    Contained,
    Misaligned,
}

/// `build.rs` stores `hook.prompt` verbatim, and nothing upstream truncates it, so a
/// typed prompt must equal it up to surrounding whitespace. A prefix is not accepted:
/// with no truncation to allow for, "yes" against "yes please" is a different prompt.
/// An injected prompt may instead contain it: Claude 2.1.278's peer hand-back wraps the
/// `<agent-message>` block its hook reports in harness text. An empty hook prompt is
/// contained in anything, so it never aligns that way.
pub(crate) fn alignment(hook: &str, prompt: &Prompt<'_>) -> Alignment {
    let hook = hook.trim();
    if hook == prompt.text.trim() {
        Alignment::Exact
    } else if !prompt.human && !hook.is_empty() && prompt.text.contains(hook) {
        Alignment::Contained
    } else {
        Alignment::Misaligned
    }
}
