//! Milestone 9.8 (MR §5.1, §5.2): the `models` section's keys and the picker's, split
//! from `settings_keys.rs` by responsibility (`AGENTS.md` hard rule 8). `j`/`k` move,
//! `←`/`→` switch the scope, `space` opens `helpers ▸`, `⏎` and `f` open the picker,
//! `e` cycles the effort, `x` removes the scope's row, `w` saves the scope. While the
//! picker is open it has every key. Pure: what needs the daemon is returned as an
//! intent for the app to send.

use super::model_picker::{Catalogs, Picked};
use super::models_table::{RowKey, Scope};
use super::settings_screen::SettingsScreen;
use crossterm::event::{KeyCode, KeyEvent};

/// What a `models` key asks of the app beyond the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelsIntent {
    Stay,
    /// `w` in `everywhere`: the screen's `Settings(Put)`.
    Save,
    /// `w` in `this repo`: the scope's `PutRepoModels`.
    SaveRepo,
    /// `this repo` entered with its table not read yet: `RepoModels`.
    AskRepo,
    /// `r` in the picker: `ListModels { runtime: None, refresh: true }`.
    Refresh,
    /// Preflight F17: `e` changed brainstorm's effort, which its row does not draw; the
    /// status line says it.
    BrainstormEffort,
}

/// A bare key on the `models` section (`esc` and `tab` are the screen's, unless the
/// picker is open).
pub(crate) fn models_key(
    s: &mut SettingsScreen,
    catalogs: &Catalogs,
    key: KeyEvent,
) -> ModelsIntent {
    let m = &mut s.models;
    if let Some(picker) = &mut m.picker {
        match picker.on_key(key) {
            None => {}
            Some(Picked::Refresh) => return ModelsIntent::Refresh,
            Some(picked) => {
                if m.on_picked(picked, catalogs) {
                    s.touched();
                }
            }
        }
        return ModelsIntent::Stay;
    }
    let changed = match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            m.move_by(1);
            false
        }
        KeyCode::Char('k') | KeyCode::Up => {
            m.move_by(-1);
            false
        }
        KeyCode::Left => {
            m.set_scope(Scope::Everywhere);
            false
        }
        KeyCode::Right => {
            if m.set_scope(Scope::Repo) {
                return ModelsIntent::AskRepo;
            }
            false
        }
        KeyCode::Char(' ') => {
            m.toggle_helpers();
            false
        }
        KeyCode::Enter => {
            m.open_picker(catalogs, false);
            false
        }
        KeyCode::Char('f') => {
            m.open_picker(catalogs, true);
            false
        }
        KeyCode::Char('e') if m.selected_key() == RowKey::Brainstorm => {
            if m.cycle_effort(catalogs) {
                s.touched();
                return ModelsIntent::BrainstormEffort;
            }
            false
        }
        KeyCode::Char('e') => m.cycle_effort(catalogs),
        KeyCode::Char('x') => m.reset(),
        KeyCode::Char('w') => {
            return match m.scope {
                Scope::Everywhere => ModelsIntent::Save,
                Scope::Repo => ModelsIntent::SaveRepo,
            };
        }
        _ => false,
    };
    if changed {
        s.touched();
    }
    ModelsIntent::Stay
}
