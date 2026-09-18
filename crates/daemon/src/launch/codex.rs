/// Returns the source path used to trust Codex lifecycle hooks.
///
/// M3.9 will replace this fallback if runtime verification establishes a trusted
/// source path. Until then, Codex launches without lifecycle hooks.
pub fn default_hook_source() -> Option<String> {
    None
}
