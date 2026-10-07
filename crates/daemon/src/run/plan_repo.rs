//! `for_repo`, moved out of `plan.rs` (milestone 9.8, task M9.8.7a) to keep that file
//! under the 600-line rule; a pure move. It canonicalises paths, as it did there.

/// The value of a table keyed by repository root (the user's `[orchestrator.cache_dirs]`,
/// F1c round 3, N3; `[orchestrator.confined_network]`, F1d) for the repository at
/// `root`. The key is matched both as written and canonicalised, so a config that names
/// the repository by a path with a symlink or a trailing slash still applies.
pub(crate) fn for_repo<'a, T>(
    table: &'a std::collections::BTreeMap<String, T>,
    root: &std::path::Path,
) -> Option<&'a T> {
    let canonical = root.canonicalize().ok();
    let same = |a: Option<&std::path::Path>, b: Option<&std::path::Path>| match (a, b) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    table.iter().find_map(|(key, value)| {
        let key_path = std::path::Path::new(key);
        let key_canonical = key_path.canonicalize().ok();
        let matches = key_path == root
            || same(canonical.as_deref(), Some(key_path))
            || same(key_canonical.as_deref(), canonical.as_deref())
            || same(key_canonical.as_deref(), Some(root));
        matches.then_some(value)
    })
}
