//! Codex permission profiles (≥ 0.160): one profile extending `:read-only`, so nothing
//! is writable but what the plan names, and a `read` entry inside a writable path keeps
//! that sub-path read-only. Identical on a first turn and a resume.

use super::{Mode, PROFILE_NAME, SandboxPlan};
use crate::launch::codex::toml_string;

pub(super) fn render(plan: &SandboxPlan) -> Vec<String> {
    let c = |v: String| ["-c".to_string(), v];
    match plan.mode {
        Mode::ReadOnly => c(format!("default_permissions={}", toml_string(":read-only"))).to_vec(),
        Mode::FullAccess => c(format!(
            "default_permissions={}",
            toml_string(":danger-full-access")
        ))
        .to_vec(),
        Mode::Confined => {
            let entries: Vec<String> = std::iter::once(&plan.cwd)
                .chain(&plan.write)
                .map(|p| {
                    format!(
                        "{}={}",
                        toml_string(&p.display().to_string()),
                        toml_string("write")
                    )
                })
                .chain(plan.read_only.iter().map(|p| {
                    format!(
                        "{}={}",
                        toml_string(&p.display().to_string()),
                        toml_string("read")
                    )
                }))
                .collect();
            [
                c(format!("default_permissions={}", toml_string(PROFILE_NAME))),
                c(format!(
                    "permissions.{PROFILE_NAME}.extends={}",
                    toml_string(":read-only")
                )),
                c(format!("permissions.{PROFILE_NAME}.network.enabled=false")),
                c(format!(
                    "permissions.{PROFILE_NAME}.filesystem={{{}}}",
                    entries.join(",")
                )),
            ]
            .concat()
        }
    }
}
