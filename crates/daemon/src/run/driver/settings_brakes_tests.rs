//! Milestone 9.16 decision 10 (task M9.16.7): the loaded config's `[orchestrator.brakes]`
//! is in the engine before its first step, and every Settings save that lands sends the
//! live brakes as `EventKind::BrakesConfig`. On a real `RunService` whose config file is
//! under a `tempfile` directory, as `settings_tests.rs` builds one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{RunReply, SettingsReply, SettingsRequest};

use crate::live_config::LiveSettings;
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::driver::{Msg, RunContext, RunService};
use crate::run::engine::EventKind;
use crate::run::engine::quota_book::BrakeConfig;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn service(path: &Path) -> Arc<RunService> {
    let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let data = path.parent().unwrap().join("data");
    let orchestrator = config::load(path).0.orchestrator;
    let live = LiveSettings::defaults_of(orchestrator.clone());
    let ctx = RunContext::new(data, manager.config(), orchestrator, Arc::new(NoRoots))
        .with_settings(live, path.to_path_buf());
    RunService::new(manager, ctx)
}

/// Every `BrakesConfig` the service queued, in order.
fn brakes_sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Msg>) -> Vec<BrakeConfig> {
    let mut sent = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let Msg::Event(EventKind::BrakesConfig(cfg)) = msg {
            sent.push(cfg);
        }
    }
    sent
}

#[tokio::test]
async fn settings_save_sends_brakes_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[orchestrator.brakes]\nquota_pause_7d_percent = 97\n",
    )
    .unwrap();
    let s = service(&path);
    // Loaded: U11's default-on brake with the file's 7-day threshold, before any step.
    let loaded = BrakeConfig {
        quota_brake: true,
        pause_5h_percent: 90,
        pause_7d_percent: 97,
        fresh_secs: 600,
    };
    assert_eq!(crate::lock(&s.state).brakes.config, loaded);
    let mut rx = crate::lock(&s.rx).take().expect("the loop is not running");
    assert!(brakes_sent(&mut rx).is_empty());

    let mut doc = config::settings::doc_of(&s.ctx.settings.current().orchestrator);
    doc.brakes.quota_pause_5h_percent = 80;
    let request = proto::RunRequest::Settings(SettingsRequest::Put { settings: doc });
    let RunReply::Settings { reply, .. } = s.request(request).await else {
        panic!("not a settings reply")
    };
    assert!(matches!(*reply, SettingsReply::Saved { .. }), "{reply:?}");
    let saved = BrakeConfig {
        pause_5h_percent: 80,
        ..loaded
    };
    assert_eq!(brakes_sent(&mut rx), [saved]);
}
