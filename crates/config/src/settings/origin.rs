//! Decision 26: where each settings value came from. A key is `File` when the parsed
//! table holds it; `orchestrator.models` is `File` when the file sets `builtin_models`
//! or has any `[[orchestrator.models]]`. Everything else is `Default`.

use std::collections::BTreeMap;
use std::path::Path;

use proto::Origin;
use proto::settings::{SETTINGS_KEYS, key};

use crate::{Config, Problem};

/// The origin of each of the thirteen `SETTINGS_KEYS` in `table`.
pub fn origin_of(table: &toml::Table) -> BTreeMap<String, Origin> {
    let has = |path: &str| lookup(table, path).is_some();
    SETTINGS_KEYS
        .iter()
        .map(|k| {
            let set = if *k == key::MODELS {
                has("orchestrator.builtin_models") || has(key::MODELS)
            } else {
                has(k)
            };
            let origin = if set { Origin::File } else { Origin::Default };
            (k.to_string(), origin)
        })
        .collect()
}

/// Blocking: [`crate::load`] plus the origin of the text it read. A missing or unreadable
/// file, or text that is not TOML, gives every key `Default`, as `load` gives defaults.
pub fn load_with_origin(path: &Path) -> (Config, Vec<Problem>, BTreeMap<String, Origin>) {
    let all_default = || origin_of(&toml::Table::new());
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (config, problems) = crate::parse(&text);
            let origin = text
                .parse::<toml::Table>()
                .map(|t| origin_of(&t))
                .unwrap_or_else(|_| all_default());
            (config, problems, origin)
        }
        Err(_) => {
            let (config, problems) = crate::load(path);
            (config, problems, all_default())
        }
    }
}

/// The value at a dotted path of plain segments, if every table on the way exists.
pub(super) fn lookup<'a>(table: &'a toml::Table, path: &str) -> Option<&'a toml::Value> {
    let mut segments = path.split('.');
    let mut value = table.get(segments.next()?)?;
    for segment in segments {
        value = value.as_table()?.get(segment)?;
    }
    Some(value)
}
