//! Decisions 5 and 6: a failure's model-free signature, the same for the same failure
//! in any checkout, run or minute, and the signatures a task record carries. Pure.

use std::sync::LazyLock;

use proto::safe_text::{multi_line, one_line};
use proto::{SigGate, SigRef};
use regex::{Captures, Regex};
use sha2::{Digest, Sha256};

use crate::run::model::Task;

/// Decision 5: the decisive lines a signature keeps.
pub const SIG_LINES: usize = 3;
/// Decision 5: a headline's characters at most.
pub const HEADLINE_MAX: usize = 160;
/// Decision 6: a task record's signatures at most.
pub const SIGS_PER_TASK: usize = 8;

/// One failure's signature: the fingerprint of its kept lines and the first of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sig {
    pub fp: String,
    pub headline: String,
}

/// The fingerprint's hex characters.
const FP_CHARS: usize = 16;

/// Decision 5 step 2: lines that carry no failure of their own.
const SKIPPED: [&str; 16] = [
    "Compiling",
    "Checking",
    "Running",
    "Finished",
    "Downloaded",
    "Downloading",
    "Blocking",
    "Locking",
    "Updating",
    "Fresh",
    "Documenting",
    "running ",
    "test result:",
    "note:",
    "= note:",
    "= help:",
];

/// Decision 5 step 3, lowercased (`FAILED` is `failed`'s case).
const DECISIVE: [&str; 10] = [
    "error",
    "failed",
    "panicked",
    "assert",
    "expected",
    "cannot",
    "not found",
    "undefined",
    "exception",
    "traceback",
];

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid signature pattern")
}

/// Ruling C10 (a): CSI and OSC sequences, and any other two-character escape.
static ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?|\x1b[@-Z\\-_]?")
});
/// `…/runs/<run id>/<checkout>/` (`model_paths.rs`'s `<wt_dir>/runs/<id>/<task>`).
static CHECKOUT: LazyLock<Regex> = LazyLock::new(|| re(r"(?:/[^/\s]+)*?/runs/[^/\s]+/[^/\s]+/"));
/// Ruling C10 (b): `…/target-pool/<dir>/`, 9.14's pool directories.
static POOL: LazyLock<Regex> = LazyLock::new(|| re(r"(?:/[^/\s]+)*?/target-pool/[^/\s]+/"));
/// The system temporary directory's entries, up to the next separator.
static TMP: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?:/private)?/var/folders/(?:[^/\s]+/)*?T/[^/\s]*|(?:/private)?/tmp/[^/\s]*")
});
static RUN_ID: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(runs/|anthrex/)[a-z0-9-]+-[0-9a-f]{4}\b"));
static TASK_ID: LazyLock<Regex> = LazyLock::new(|| re(r"\bt\d+[a-z]?\b"));
/// Review I2 (task M9.15.2), an amendment to decision 5: an ISO-8601 timestamp, with
/// or without fractional seconds and a `Z` or `±hh:mm` offset; before [`LINE_COL`].
static TIME: LazyLock<Regex> = LazyLock::new(|| {
    re(r"\b\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?")
});
/// Review I1 (task M9.15.2), an amendment to decision 5: a panic line's thread id
/// (`thread '<name>' (<tid>) panicked at`).
static TID: LazyLock<Regex> = LazyLock::new(|| re(r"\(\d+\) panicked"));
static LINE_COL: LazyLock<Regex> = LazyLock::new(|| re(r":\d+:\d+"));
/// `:<line>` after a file name (one with an extension).
static LINE: LazyLock<Regex> = LazyLock::new(|| re(r"(\.[A-Za-z0-9]+):\d+\b"));
static DURATION: LazyLock<Regex> = LazyLock::new(|| re(r"\b\d+(?:\.\d+)?(?:ms|s)\b"));
/// Every maximal hex run; [`normalise`] rewrites those of 7 to 40 characters.
static HEX: LazyLock<Regex> = LazyLock::new(|| re(r"[0-9a-fA-F]+"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r" {2,}"));

/// Decision 5 step 1, in its order.
fn normalise(line: &str) -> String {
    let line = CHECKOUT.replace_all(line, "<worktree>/");
    let line = POOL.replace_all(&line, "<target>/");
    let line = TMP.replace_all(&line, "<tmp>");
    let line = RUN_ID.replace_all(&line, "${1}<id>");
    let line = TASK_ID.replace_all(&line, "<id>");
    let line = TIME.replace_all(&line, "<time>");
    let line = TID.replace_all(&line, "(<tid>) panicked");
    let line = LINE_COL.replace_all(&line, ":<l>:<c>");
    let line = LINE.replace_all(&line, "${1}:<l>");
    let line = DURATION.replace_all(&line, "<dur>");
    let line = HEX.replace_all(&line, |c: &Captures| match c[0].len() {
        7..=40 => "<sha>".to_string(),
        _ => c[0].to_string(),
    });
    let line = SPACES.replace_all(&line, " ");
    line.trim().to_string()
}

fn skipped(line: &str) -> bool {
    line.is_empty() || SKIPPED.iter().any(|p| line.starts_with(p))
}

fn decisive(line: &str) -> bool {
    let lower = line.to_lowercase();
    DECISIVE.iter().any(|word| lower.contains(word))
}

/// Decision 5: `output`'s signature, or `None` when no line is left to keep.
pub fn of(output: &str) -> Option<Sig> {
    let text = multi_line(&ESCAPE.replace_all(output, ""));
    let lines: Vec<String> = (text.lines().map(normalise))
        .filter(|l| !skipped(l))
        .collect();
    let mut kept: Vec<&String> = lines
        .iter()
        .filter(|l| decisive(l))
        .take(SIG_LINES)
        .collect();
    if kept.is_empty() {
        kept = lines
            .iter()
            .skip(lines.len().saturating_sub(SIG_LINES))
            .collect();
    }
    let first = kept.first()?;
    let joined = kept
        .iter()
        .map(|l| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let digest = Sha256::digest(joined.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Some(Sig {
        fp: hex[..FP_CHARS].to_string(),
        headline: one_line(first).chars().take(HEADLINE_MAX).collect(),
    })
}

/// Decision 6 with ruling C21: the signatures of `task`'s failed checks (in order),
/// then of its failed proofs, deduplicated by `fp` (the first kept), at most
/// [`SIGS_PER_TASK`].
pub fn task_signatures(task: &Task) -> Vec<SigRef> {
    let checks = (task.checks.iter().filter(|c| !c.ok)).map(|c| {
        let gate = match c.tier.as_ref().map(|t| t.tier) {
            Some(1) => SigGate::Tier1,
            Some(2) => SigGate::Tier2,
            _ => SigGate::Check,
        };
        (gate, c.tail.as_str())
    });
    // The history tally's failed proof (`history::gates`): a red-only check is none.
    let proofs = (task.proofs.iter())
        .filter(|p| !p.red_only && !(p.red_failed && p.head_passed && p.matched))
        .map(|p| {
            let tail = if p.head_passed {
                &p.red_tail
            } else {
                &p.head_tail
            };
            (SigGate::Proof, tail.as_str())
        });
    let mut out: Vec<SigRef> = Vec::new();
    for (gate, tail) in checks.chain(proofs) {
        if out.len() == SIGS_PER_TASK {
            break;
        }
        let Some(sig) = of(tail) else { continue };
        if out.iter().all(|s| s.fp != sig.fp) {
            out.push(SigRef {
                gate,
                fp: sig.fp,
                headline: sig.headline,
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "signature_tests.rs"]
mod tests;
