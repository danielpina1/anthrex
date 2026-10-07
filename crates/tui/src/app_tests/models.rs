//! Milestone 9.8.4: `DaemonMsg::Models` lands in `App.catalogs`.

use super::*;
use proto::{CatalogSource, ModelCatalog};

fn catalog(runtime: Runtime, version: &str) -> ModelCatalog {
    ModelCatalog {
        runtime,
        cli_version: version.into(),
        fetched_at: 1,
        source: CatalogSource::Live,
        models: vec![],
        problem: None,
    }
}

#[test]
fn a_models_message_stores_the_catalogs_one_per_runtime() {
    let mut app = app_with(vec![]);
    assert!(app.catalogs.list.is_empty());
    let effects = app.on_daemon(DaemonMsg::Models {
        catalogs: vec![
            catalog(Runtime::Claude, "2.1.290"),
            catalog(Runtime::Codex, "0.160.1"),
        ],
    });
    assert!(effects.is_empty());
    assert_eq!(app.catalogs.list.len(), 2);

    // A later answer for one runtime replaces that runtime's catalog only.
    app.on_daemon(DaemonMsg::Models {
        catalogs: vec![catalog(Runtime::Codex, "0.161.0")],
    });
    let versions: Vec<(Runtime, &str)> = app
        .catalogs
        .list
        .iter()
        .map(|c| (c.runtime, c.cli_version.as_str()))
        .collect();
    assert_eq!(
        versions,
        [(Runtime::Claude, "2.1.290"), (Runtime::Codex, "0.161.0")]
    );
}
