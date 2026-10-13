//! The pool's test seam: a stage where the pool stops once and waits (tests only).

use std::sync::Arc;

use tokio::sync::Mutex as AsyncMutex;

use super::TargetPool;
use crate::lock;

/// A test seam: the pool stops at `stage` once, says so on `reached`, and waits for
/// `go`.
pub(crate) struct Seam {
    pub stage: &'static str,
    pub reached: tokio::sync::mpsc::UnboundedSender<()>,
    pub go: Arc<AsyncMutex<tokio::sync::mpsc::UnboundedReceiver<()>>>,
}

impl TargetPool {
    pub(crate) fn set_seam(&self, seam: Seam) {
        *lock(&self.seam) = Some(seam);
    }

    /// Stops here once when the seam names `stage`.
    pub(super) async fn pause(&self, stage: &'static str) {
        let seam = {
            let mut seam = lock(&self.seam);
            match seam.as_ref() {
                Some(s) if s.stage == stage => seam.take(),
                _ => None,
            }
        };
        if let Some(seam) = seam {
            let _ = seam.reached.send(());
            let _ = seam.go.lock().await.recv().await;
        }
    }
}
