//! The repository file (MR §3.3): `<repo_dir>/models.toml`, a `[models]` table whose
//! rows beat the global table's for runs in that repository. The only file of
//! `config::models` with I/O; it blocks, so callers run it on `spawn_blocking`.

use std::path::{Path, PathBuf};

use proto::models::ModelTable;

use super::parse::{parse_text, render};
use crate::settings::{file_mode, temp_path, write_temp};

/// `None` and no warning for a missing file; `None` and one warning for an unreadable
/// or unparseable one (MR §7); the table and one warning per bad row otherwise. Every
/// warning starts with the path.
pub fn load_repo(path: &Path) -> (Option<ModelTable>, Vec<String>) {
    let shown = path.display();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, Vec::new()),
        Err(e) => return (None, vec![format!("{shown}: could not read: {e}")]),
    };
    match parse_text(&text) {
        Ok((table, problems)) => (
            Some(table),
            problems.iter().map(|p| format!("{shown}: {p}")).collect(),
        ),
        Err(e) => (None, vec![format!("{shown}: not valid TOML: {e}")]),
    }
}

/// Writes `table` to `path` atomically (a temporary file beside it, then a rename, then
/// the directory fsynced); an empty table removes the file. A missing directory is
/// created. As `config::settings::save` (fix round 1, M9): a symlinked file stays a
/// link (its target is replaced) and keeps its mode.
pub fn save_repo(path: &Path, table: &ModelTable) -> Result<(), String> {
    if table.is_empty() {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("could not remove {}: {e}", path.display())),
        };
    }
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path = resolved.as_path();
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let temp = temp_path(&dir, path);
    let written = write_temp(&temp, &render(table), file_mode(path)).and_then(|()| {
        std::fs::rename(&temp, path)
            .map_err(|e| format!("could not replace {}: {e}", path.display()))
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    } else if let Ok(d) = std::fs::File::open(&dir) {
        let _ = d.sync_all();
    }
    written
}
