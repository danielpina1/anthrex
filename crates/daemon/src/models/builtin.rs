//! The built-in lists, served when a CLI is missing or its probe failed with no cache
//! to fall back on. Only today's three effort names: a built-in list is a guess and
//! validates nothing (decision 21).

use proto::{CatalogModel, CatalogSource, ModelCatalog, Runtime};

fn entry(id: &str, label: &str, efforts: bool, is_default: bool) -> CatalogModel {
    CatalogModel {
        id: id.to_string(),
        label: label.to_string(),
        description: String::new(),
        efforts: if efforts {
            vec!["low".into(), "medium".into(), "high".into()]
        } else {
            Vec::new()
        },
        default_effort: None,
        is_default,
    }
}

pub fn models(runtime: Runtime) -> Vec<CatalogModel> {
    match runtime {
        Runtime::Claude => vec![
            entry("claude-haiku-4-5", "Haiku 4.5", false, false),
            entry("claude-sonnet-5", "Sonnet 5", true, false),
            entry("claude-opus-5-5", "Opus 5.5", true, false),
        ],
        Runtime::Codex => vec![entry("default", "Codex default", true, true)],
        Runtime::Shell => Vec::new(),
    }
}

/// `runtime`'s built-in catalog, saying why it is served.
pub fn catalog(
    runtime: Runtime,
    cli_version: String,
    fetched_at: u64,
    problem: String,
) -> ModelCatalog {
    ModelCatalog {
        runtime,
        cli_version,
        fetched_at,
        source: CatalogSource::Builtin,
        models: models(runtime),
        problem: Some(problem),
    }
}
