//! Test only: a [`Runner`] that records every command and answers from a script, in
//! order (task M9.2.4). It starts no process; a call it has no answer for panics.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::HostError;
use super::runner::{Capture, Cut, Program, RunOutput, Runner};

/// One recorded command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    pub program: Program,
    pub dir: PathBuf,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub cap: Capture,
}

#[derive(Default)]
pub(crate) struct ScriptedRunner {
    calls: Mutex<Vec<Call>>,
    answers: Mutex<VecDeque<Result<RunOutput, HostError>>>,
}

impl ScriptedRunner {
    pub(crate) fn new() -> Self {
        ScriptedRunner::default()
    }

    /// The next command exits 0 printing `stdout`.
    pub(crate) fn ok(self, stdout: &str) -> Self {
        self.answer(Ok(RunOutput {
            success: true,
            stdout: stdout.as_bytes().to_vec(),
            ..RunOutput::default()
        }))
    }

    /// The next command exits non-zero printing `stdout` and `stderr`.
    pub(crate) fn fails(self, stdout: &str, stderr: &str) -> Self {
        self.answer(Ok(RunOutput {
            success: false,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.to_string(),
            cut: None,
        }))
    }

    pub(crate) fn answer(self, answer: Result<RunOutput, HostError>) -> Self {
        crate::lock(&self.answers).push_back(answer);
        self
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        crate::lock(&self.calls).clone()
    }

    pub(crate) fn argvs(&self) -> Vec<Vec<String>> {
        self.calls().into_iter().map(|c| c.argv).collect()
    }

    pub(crate) fn unanswered(&self) -> usize {
        crate::lock(&self.answers).len()
    }
}

impl Runner for ScriptedRunner {
    fn run(
        &self,
        program: Program,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        timeout: Duration,
        cap: Capture,
    ) -> Result<RunOutput, HostError> {
        crate::lock(&self.calls).push(Call {
            program,
            dir: dir.to_path_buf(),
            argv: argv.to_vec(),
            env: env.to_vec(),
            timeout,
            cap,
        });
        let answer = crate::lock(&self.answers)
            .pop_front()
            .unwrap_or_else(|| panic!("ScriptedRunner: no answer for {program:?} {argv:?}"));
        // A head-and-tail capture keeps only the two ends, as `subprocess` does.
        match (answer, cap) {
            (Ok(mut out), Capture::HeadTail { head, tail }) if out.stdout.len() > head + tail => {
                let total = out.stdout.len();
                let mut kept = out.stdout[..head].to_vec();
                kept.extend_from_slice(&out.stdout[total - tail..]);
                out.stdout = kept;
                out.cut = Some(Cut {
                    at: head,
                    dropped: (total - head - tail) as u64,
                });
                Ok(out)
            }
            (answer, _) => answer,
        }
    }
}
