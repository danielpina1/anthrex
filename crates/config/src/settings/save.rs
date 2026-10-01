//! Decision 28 step 3: the save. `save` validates, edits the text, proves the edit reads
//! back, writes a temporary file beside the original, fsyncs it, renames it over the
//! original and fsyncs the directory. A refused or cancelled save leaves the file
//! byte-identical and no temporary file behind.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use proto::{Origin, SettingsDoc};

use super::origin::{lookup, origin_of};
use crate::Orchestrator;

/// What a save wrote: the `[orchestrator]` table the new file reads as, and its origin.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub orchestrator: Orchestrator,
    pub origin: BTreeMap<String, Origin>,
}

/// Blocking: [`super::validate`], read, [`super::edit_text`], temporary file, fsync,
/// `cancel` check, rename, fsync the directory. A missing file (and its directory) is
/// created. On any refusal the file is untouched and no temporary file is left.
pub fn save(path: &Path, doc: &SettingsDoc, cancel: &AtomicBool) -> Result<Saved, Vec<String>> {
    let problems = super::validate(doc);
    if !problems.is_empty() {
        return Err(problems);
    }
    let (text, mode) = match std::fs::read_to_string(path) {
        Ok(text) => (text, file_mode(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
        Err(e) => return Err(vec![format!("could not read {}: {e}", path.display())]),
    };
    let out = super::edit_text(&text, doc)?;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| vec![format!("could not create {}: {e}", dir.display())])?;
    let temp = temp_path(&dir, path);
    let written = write_temp(&temp, &out, mode).and_then(|()| {
        if cancel.load(Ordering::SeqCst) {
            return Err("the save was cancelled; nothing changed".to_string());
        }
        std::fs::rename(&temp, path)
            .map_err(|e| format!("could not replace {}: {e}", path.display()))
    });
    if let Err(problem) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(vec![problem]);
    }
    // The rename is durable once the directory entry is; a failure here does not undo it.
    if let Ok(d) = std::fs::File::open(&dir) {
        let _ = d.sync_all();
    }
    let table: toml::Table = out.parse().unwrap_or_default();
    Ok(Saved {
        orchestrator: crate::parse(&out).0.orchestrator,
        origin: origin_of(&table),
    })
}

/// `<dir>/.<name>.anthrex-<pid>-<nanos>.tmp`, beside the original so the rename stays on
/// one filesystem.
fn temp_path(dir: &Path, path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(|| "config.toml".into(), |n| n.to_string_lossy());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    dir.join(format!(
        ".{name}.anthrex-{}-{nanos}.tmp",
        std::process::id()
    ))
}

fn write_temp(temp: &Path, text: &str, mode: Option<u32>) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode.unwrap_or(0o644));
    }
    let what = |e: std::io::Error| format!("could not write {}: {e}", temp.display());
    let mut file = options.open(temp).map_err(what)?;
    // `create_new`'s mode is masked by the umask; the original's mode is kept exactly.
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(what)?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    file.write_all(text.as_bytes()).map_err(what)?;
    file.sync_all().map_err(what)
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn file_mode(_: &Path) -> Option<u32> {
    None
}

/// Decision 28 step 3's guard: `out` must parse, every owned key must read back as
/// `doc`, and every other key must read back as `before` has it.
pub(super) fn read_back(
    before: &toml::Table,
    out: &str,
    doc: &SettingsDoc,
) -> Result<(), Vec<String>> {
    let after: toml::Table = out
        .parse()
        .map_err(|e| vec![format!("the edited config.toml would not parse: {e}")])?;
    let read = super::doc_of(&crate::parse(out).0.orchestrator);
    if read != *doc {
        return Err(vec![format!(
            "config.toml would not read back as saved ({}); nothing changed",
            differing(&read, doc).join(", ")
        )]);
    }
    if unowned(before.clone()) != unowned(after) {
        return Err(vec![
            "the edit would change keys the settings screen does not own; nothing changed"
                .to_string(),
        ]);
    }
    Ok(())
}

/// The keys of `SETTINGS_KEYS` on which `a` and `b` differ.
fn differing(a: &SettingsDoc, b: &SettingsDoc) -> Vec<&'static str> {
    use proto::settings::key;
    let (x, y) = (&a.limits, &b.limits);
    [
        (key::MODELS, a.models != b.models),
        (
            key::AGENT_RUNTIME,
            a.orchestrator.runtime != b.orchestrator.runtime,
        ),
        (
            key::AGENT_MODEL,
            a.orchestrator.model != b.orchestrator.model,
        ),
        (
            key::BUDGET_S_CALLS,
            x.budget_s.tool_calls != y.budget_s.tool_calls,
        ),
        (
            key::BUDGET_S_MINUTES,
            x.budget_s.minutes != y.budget_s.minutes,
        ),
        (
            key::BUDGET_M_CALLS,
            x.budget_m.tool_calls != y.budget_m.tool_calls,
        ),
        (
            key::BUDGET_M_MINUTES,
            x.budget_m.minutes != y.budget_m.minutes,
        ),
        (
            key::BUDGET_L_CALLS,
            x.budget_l.tool_calls != y.budget_l.tool_calls,
        ),
        (
            key::BUDGET_L_MINUTES,
            x.budget_l.minutes != y.budget_l.minutes,
        ),
        (
            key::STALL_AFTER_SECS,
            x.stall_after_secs != y.stall_after_secs,
        ),
        (key::MAX_WRITERS, x.max_writers != y.max_writers),
        (key::MAX_READERS, x.max_readers != y.max_readers),
        (key::MAX_BOUNCES, x.max_bounces != y.max_bounces),
    ]
    .into_iter()
    .filter_map(|(k, differs)| differs.then_some(k))
    .collect()
}

/// `table` without the owned keys, and without an owned table left empty by their removal.
fn unowned(mut table: toml::Table) -> toml::Table {
    let Some(o) = table.get_mut("orchestrator").and_then(|v| v.as_table_mut()) else {
        return table;
    };
    for k in [
        "builtin_models",
        "max_writers",
        "max_readers",
        "max_bounces",
        "stall_after_secs",
        "models",
    ] {
        o.remove(k);
    }
    if let Some(agent) = o.get_mut("agent").and_then(|v| v.as_table_mut()) {
        agent.remove("runtime");
        agent.remove("model");
    }
    if let Some(budget) = o.get_mut("budget").and_then(|v| v.as_table_mut()) {
        for r in ["s", "m", "l"] {
            if let Some(t) = budget.get_mut(r).and_then(|v| v.as_table_mut()) {
                t.remove("tool_calls");
                t.remove("minutes");
            }
            prune(budget, r);
        }
    }
    prune(o, "budget");
    prune(o, "agent");
    prune(&mut table, "orchestrator");
    debug_assert!(lookup(&table, "orchestrator.max_writers").is_none());
    table
}

fn prune(table: &mut toml::Table, key: &str) {
    if table
        .get(key)
        .and_then(|v| v.as_table())
        .is_some_and(|t| t.is_empty())
    {
        table.remove(key);
    }
}
