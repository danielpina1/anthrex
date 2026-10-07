//! Codex before permission profiles: `-s` (or `-c sandbox_mode=` on a resume whose CLI
//! rejects `-s`), the network and tmp pins, and the extra writable roots. It cannot deny
//! a path inside a writable root.

use super::{Mode, SandboxPlan, Unsupported};
use crate::headless::argv::{CODEX_SANDBOX_PINS, toml_array};
use crate::launch::codex::toml_string;

pub(super) fn render(
    plan: &SandboxPlan,
    resuming: bool,
    resume_takes_sandbox: bool,
) -> Result<Vec<String>, Unsupported> {
    if !plan.read_only.is_empty() {
        return Err(Unsupported(
            "legacy Codex sandbox flags cannot make a path read-only",
        ));
    }
    let mode = match plan.mode {
        Mode::ReadOnly => "read-only",
        Mode::Confined => "workspace-write",
        Mode::FullAccess => "danger-full-access",
    };
    let mut args = Vec::new();
    if resuming && !resume_takes_sandbox {
        args.extend(["-c".into(), format!("sandbox_mode={}", toml_string(mode))]);
    } else {
        args.extend(["-s".into(), mode.to_string()]);
    }
    for pin in CODEX_SANDBOX_PINS {
        args.extend(["-c".into(), pin.to_string()]);
    }
    if !plan.write.is_empty() {
        let roots: Vec<String> = plan.write.iter().map(|p| p.display().to_string()).collect();
        args.extend([
            "-c".into(),
            format!(
                "sandbox_workspace_write.writable_roots={}",
                toml_array(&roots)
            ),
        ]);
    }
    Ok(args)
}
