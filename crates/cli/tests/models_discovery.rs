//! Milestone 9.8 (MR §4): a real daemon discovers both CLIs' models from their
//! handshakes (`fake-agent` as both, pinned by `RunHarness`) and caches them per version.

mod support;

use proto::{CatalogSource, Runtime};
use support::models::list_models;
use support::run_harness::RunHarness;

#[test]
fn list_models_reports_both_clis_from_their_handshakes_and_caches_them() {
    let h = RunHarness::new("");
    let catalogs = list_models(&h.socket(), None, false);
    assert_eq!(catalogs.len(), 2, "{catalogs:?}");
    let claude = catalogs
        .iter()
        .find(|c| c.runtime == Runtime::Claude)
        .unwrap();
    assert_eq!(claude.source, CatalogSource::Live, "{claude:?}");
    // Ruling F8: `fake-agent --version` prints `codex-cli 0.160.1` for both runtimes.
    assert_eq!(claude.cli_version, "0.160.1");
    assert_eq!(
        claude
            .models
            .iter()
            .map(|m| m.label.as_str())
            .collect::<Vec<_>>(),
        [
            "Default (recommended)",
            "Opus (1M context)",
            "Fable",
            "Sonnet",
            "Haiku"
        ]
    );
    // Real-CLI manual check fix: the alias entries carry the model each resolves to.
    let opus = claude.models.iter().find(|m| m.id == "opus[1m]").unwrap();
    assert_eq!(opus.resolved.as_deref(), Some("claude-opus-5-5[1m]"));
    assert_eq!(opus.efforts, ["low", "medium", "high", "xhigh", "max"]);
    let codex = catalogs
        .iter()
        .find(|c| c.runtime == Runtime::Codex)
        .unwrap();
    assert_eq!(codex.models.len(), 7, "both pages");
    assert!(
        codex
            .models
            .iter()
            .any(|m| m.id == "gpt-6.1-sol" && m.is_default && m.efforts.last().unwrap() == "ultra")
    );
    assert_eq!(
        h.io_lines("discovery-claude", "stdin").len(),
        1,
        "one initialize, no prompt"
    );
    let again = list_models(&h.socket(), None, false);
    assert_eq!(again, catalogs, "the same version's cache, unchanged");
    assert_eq!(
        h.io_lines("discovery-claude", "stdin").len(),
        1,
        "no second probe"
    );
    list_models(&h.socket(), Some(Runtime::Claude), true);
    assert_eq!(
        h.io_lines("discovery-claude", "stdin").len(),
        2,
        "r refreshes"
    );
}
