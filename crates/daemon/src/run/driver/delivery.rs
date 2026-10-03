//! Milestone 9.2 (task M9.2.12), I/O: a run's delivery mode, resolved once at `run start`
//! (decision 3), preflight in `pr` mode (decision 17) with the remote's seal (the
//! controller's ruling), and `run deliver` / `run watch` (decision 25). `run start
//! --plan` resolves and preflights in `build_plan`, after M8a's git preflight and before
//! the run is built; `run start --goal` before triage, so a refusal costs no decider
//! call, and hands the result on. Every host call runs on `spawn_blocking` with a bound,
//! never under the engine or the manager lock. A refusal leaves nothing behind: no run
//! exists yet. In `local` mode no host is called.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proto::run_wire::request;
use proto::{DeliveryMode, DeliveryProfile, RunReply};

use super::RunService;
use crate::host::{CodeHost, HostRepo, PREFLIGHT_BOUND, PreflightReq};
use crate::run::engine::EventKind;
use crate::run::engine::delivery::DeliveryRequest;
use crate::run::model::Run;
use crate::run::plan::Preflight;

/// What `run start` freezes into `RunDelivery` (decision 3): the mode, preflight's
/// repository and the remote's seal; `local` has neither.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Frozen {
    pub mode: DeliveryMode,
    pub repo: Option<HostRepo>,
    pub seal: Option<String>,
}

impl Frozen {
    /// Onto a run `build_run` made: in `pr` mode, watching from the start at the
    /// configured interval (decisions 11, 25).
    pub(crate) fn apply(self, run: &mut Run) {
        let pr = self.mode == DeliveryMode::Pr;
        let d = &mut run.delivery;
        d.mode = self.mode;
        d.repo = self.repo;
        d.remote_seal = self.seal;
        d.watching = pr;
        d.poll_base_secs = if pr { d.limits.poll_secs } else { 0 };
    }
}

/// How `build_plan` gets a run's delivery: resolved and preflighted there (`run start
/// --plan`), or already done before triage (`run start --goal`).
pub(super) enum DeliveryStart {
    Resolve(Option<DeliveryMode>),
    Done(Frozen),
}

/// Decision 3: `--delivery` over the profile's `[delivery] mode`, else `local`; the
/// profile's remote, else `origin`.
pub(crate) fn resolve(
    asked: Option<DeliveryMode>,
    profile: Option<&DeliveryProfile>,
) -> (DeliveryMode, String) {
    let mode = asked
        .or(profile.map(|p| p.mode))
        .unwrap_or(DeliveryMode::Local);
    let remote = profile.map_or_else(|| "origin".to_string(), |p| p.remote.clone());
    (mode, remote)
}

/// Decision 17's 8 random hexadecimal characters.
fn nonce() -> String {
    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    hasher.write_u128(nanos);
    format!("{:08x}", hasher.finish() as u32)
}

/// Decision 17: `host`'s preflight, then the remote's seal, on a blocking thread within
/// `bound`. The refusal is the failing check's exact text.
pub(crate) async fn preflight(
    host: Arc<dyn CodeHost>,
    req: PreflightReq,
    bound: Duration,
) -> Result<Frozen, String> {
    let work = tokio::task::spawn_blocking(move || {
        let repo = host.preflight(&req).map_err(|e| e.text().to_string())?;
        let seal = host
            .remote_seal(&repo.root, &repo.remote)
            .map_err(|e| e.text().to_string())?;
        Ok(Frozen {
            mode: DeliveryMode::Pr,
            repo: Some(repo),
            seal: Some(seal),
        })
    });
    match tokio::time::timeout(bound, work).await {
        Ok(Ok(result)) => result,
        // A panicking host is a bug: the start is refused, never retried silently.
        Ok(Err(error)) => Err(format!("preflight did not finish: {error}")),
        Err(_) => Err(format!(
            "preflight: gh did not answer within {} s",
            bound.as_millis().div_ceil(1000)
        )),
    }
}

impl RunService {
    /// The daemon's code host (decision 15), shared with the profile service.
    pub fn host(&self) -> Arc<dyn CodeHost> {
        self.ctx.host.clone()
    }

    /// Decision 3 then decision 17, once per `run start`: the mode `asked` or the
    /// profile gives, and in `pr` mode preflight against the repository `pre` found.
    pub(super) async fn freeze_delivery(
        &self,
        pre: &Preflight,
        asked: Option<DeliveryMode>,
        profile: Option<&DeliveryProfile>,
    ) -> Result<Frozen, String> {
        let (mode, remote) = resolve(asked, profile);
        if mode == DeliveryMode::Local {
            return Ok(Frozen::default());
        }
        let req = PreflightReq {
            root: pre.root.clone(),
            remote,
            base_branch: pre.base_branch.clone(),
            base_sha: pre.base_sha.clone(),
            nonce: nonce(),
        };
        let bound = self
            .ctx
            .host_cap
            .map_or(PREFLIGHT_BOUND, |cap| cap.min(PREFLIGHT_BOUND));
        preflight(self.host(), req, bound).await
    }

    /// Decision 25: `run deliver` and `run watch`, answered by the engine.
    pub(super) async fn delivery_request(&self, req: DeliveryRequestOf) -> RunReply {
        let (label, result) = match req {
            DeliveryRequestOf::Deliver { run_id, stage } => (
                request::DELIVER,
                self.ask(|reply| {
                    EventKind::Delivery(DeliveryRequest::Deliver {
                        reply,
                        run_id,
                        stage,
                    })
                })
                .await,
            ),
            DeliveryRequestOf::Watch { run_id, on } => (
                request::WATCH,
                self.ask(|reply| EventKind::Delivery(DeliveryRequest::Watch { reply, run_id, on }))
                    .await,
            ),
        };
        match result {
            Ok(message) => RunReply::done(label, message),
            Err(message) => RunReply::refused(label, message),
        }
    }
}

/// A `RunRequest::Deliver` or `Watch`, before its reply id exists.
pub(super) enum DeliveryRequestOf {
    Deliver { run_id: String, stage: u16 },
    Watch { run_id: String, on: bool },
}

/// The tests' entry to `run start`'s build, with the delivery the repository profile
/// gives (production calls [`RunService::build_delivered`]).
#[cfg(test)]
mod build_plan {
    use std::path::PathBuf;

    use super::super::adapt::BuildError;
    use super::super::build::Shape;
    use super::{DeliveryStart, RunService};
    use crate::run::model::Run;

    impl RunService {
        pub(in crate::run::driver) async fn build_plan(
            &self,
            plan: proto::Plan,
            dir: PathBuf,
            yes: bool,
            trust_project: bool,
            unconfined_checks: bool,
            shape: Shape,
        ) -> Result<Run, BuildError> {
            let flags = (yes, trust_project, unconfined_checks);
            let delivery = DeliveryStart::Resolve(None);
            self.build_delivered(plan, dir, flags, shape, delivery)
                .await
        }
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
pub(in crate::run::driver) mod tests;
