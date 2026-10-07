//! `<cli> --version`, bounded (decision 23).

use super::ProbeError;
use super::lines::{LineReader, read_deadline, spawn};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

/// `program --version`'s `x.y.z`, or `unknown` when its output has none.
pub fn cli_version(
    program: &str,
    deadline: Instant,
    cancel: &CancellationToken,
) -> Result<String, ProbeError> {
    let mut command = Command::new(program);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let mut owned = spawn(&mut command, program, deadline, cancel)?;
    let stdout = owned.child.stdout.take().expect("stdout was piped");
    let mut reader = LineReader::new(stdout)
        .map_err(|e| ProbeError::Failed(format!("could not read {program}: {e}")))?;
    let until = read_deadline(deadline);
    let mut output = String::new();
    while let Some(line) = reader.next(until, cancel)? {
        output.push_str(&line);
        output.push('\n');
    }
    // EOF: the CLI is done writing; it exits now, or `ProbeChild` kills it.
    loop {
        match owned.child.try_wait() {
            Ok(Some(status)) => {
                owned.reaped = true;
                owned.completed = true;
                if !status.success() {
                    return Err(ProbeError::Failed(format!(
                        "{program} --version failed: {status}"
                    )));
                }
                break;
            }
            Ok(None) if cancel.is_cancelled() => return Err(ProbeError::Cancelled),
            Ok(None) if Instant::now() >= until => return Err(ProbeError::Timeout),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
            Err(e) => {
                return Err(ProbeError::Failed(format!(
                    "could not wait for {program}: {e}"
                )));
            }
        }
    }
    Ok(parse_version(&output))
}

/// Decision 23: `launch::codex::parse_version`'s `x.y.z`, else `unknown`.
pub fn parse_version(output: &str) -> String {
    match crate::launch::codex::parse_version(output) {
        Some((a, b, c)) => format!("{a}.{b}.{c}"),
        None => "unknown".to_string(),
    }
}
