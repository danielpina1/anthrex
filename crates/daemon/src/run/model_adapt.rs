//! Milestone 8b's additions to the run model. Pure (M8b decision 1): no file,
//! process, thread, async-runtime or clock access. Declared by `model.rs`.

use super::Run;

impl Run {
    /// M8b decision 7's attention line, while the stored profile is stale: a stale
    /// profile is still used, and the user is told which files changed.
    pub fn stale_profile_line(&self) -> Option<String> {
        (!self.stale_profile.is_empty()).then(|| {
            format!(
                "the repository profile may be stale: {} changed since it was confirmed; run anthrex profile detect",
                self.stale_profile.join(", ")
            )
        })
    }
}
