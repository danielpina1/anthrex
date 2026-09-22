//! `load` and `load_with`, and the corrupt/unsupported-file recovery (`mark_aside_with`,
//! `aside_path_with`) they share — the read half of this module, split from `persist`
//! (the write half) and from `mod.rs` (the data model both sides share). A submodule of
//! `state` (AGENTS.md rule 8), pulling `StateFile`, `WindowRecord`, `Problem`, `Severity`,
//! `STATE_VERSION` and `empty_state` in via `use super::*`.

use super::*;

/// Reads and validates the state file at `path`, returning the state to run with and any
/// [`Problem`]s the caller should log. See the module doc for the "never panics, never
/// errors, never silently destroys recoverable bytes" contract.
///
/// Decision 12's steps, in order: a missing file starts empty with no problems; any other
/// read error starts empty too but leaves the file exactly where it was, reported as
/// [`Severity::Error`]; invalid JSON, a non-object, a missing/non-integer `version` or
/// `next_id`, or a present-but-wrong-typed `windows` or `runs` counts as corrupt and gets
/// renamed to `<path>.corrupt-<unix-seconds>` (with a `-1`, `-2`, ... suffix added until
/// the name is free); a `version` newer than [`STATE_VERSION`] is renamed the same way to
/// `<path>.unsupported-<unix-seconds>`, protecting a newer daemon's file from an older
/// one; otherwise each entry of `windows` is deserialized on its own, so one bad record —
/// an unknown runtime, or one that repeats an earlier entry's id or name — is skipped and
/// reported instead of failing the whole load. Every case past "missing file" is
/// [`Severity::Warn`]: the module already recovered on its own.
pub fn load(path: &Path) -> (StateFile, Vec<Problem>) {
    load_with(path, SystemTime::now())
}

/// Injectable twin of [`load`]: `now` is the clock a corrupt/unsupported file's
/// `mark_aside` rename timestamps itself with, following the same pattern as
/// `project::detect_roots_with` — a real caller always uses [`load`], and tests drive
/// `now` directly so a same-second collision (or its absence) is a property of the
/// input instead of a race against the wall clock.
pub fn load_with(path: &Path, now: SystemTime) -> (StateFile, Vec<Problem>) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (empty_state(), Vec::new()),
        Err(e) => {
            return (
                empty_state(),
                vec![Problem {
                    severity: Severity::Error,
                    key: "<state file>".to_string(),
                    message: format!(
                        "could not read {}: {e}; starting empty and leaving the file alone",
                        path.display()
                    ),
                }],
            );
        }
    };

    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(e) => {
            return mark_aside_with(
                path,
                "corrupt",
                &format!("state file is not valid JSON: {e}"),
                now,
            );
        }
    };

    let Some(object) = value.as_object() else {
        return mark_aside_with(path, "corrupt", "state file is not a JSON object", now);
    };

    let Some(version) = object.get("version").and_then(serde_json::Value::as_u64) else {
        return mark_aside_with(
            path,
            "corrupt",
            "state file is missing an integer \"version\"",
            now,
        );
    };
    let Some(saved_next_id) = object.get("next_id").and_then(serde_json::Value::as_u64) else {
        return mark_aside_with(
            path,
            "corrupt",
            "state file is missing an integer \"next_id\"",
            now,
        );
    };
    let mut problems = Vec::new();

    // A saved value past `u32::MAX` cannot be represented by `StateFile::next_id`
    // (`u32`, per decision 8) any more faithfully than by saturating: there is no
    // narrower valid id to fall back to, and silently truncating is exactly the bug
    // this module must not repeat (see the `windows` loop below). Decision 12: the
    // module already recovered on its own, but the loaded state still differs from what
    // the file said, silently, unless this is reported — see Minor #3.
    let saved_next_id = match u32::try_from(saved_next_id) {
        Ok(v) => v,
        Err(_) => {
            problems.push(Problem {
                severity: Severity::Warn,
                key: "next_id".to_string(),
                message: format!(
                    "saved next_id {saved_next_id} does not fit in a u32; using {}",
                    u32::MAX
                ),
            });
            u32::MAX
        }
    };

    if version > u64::from(STATE_VERSION) {
        return mark_aside_with(
            path,
            "unsupported",
            &format!("state file version {version} is newer than {STATE_VERSION}"),
            now,
        );
    }

    // `windows` missing entirely is legitimate (the field is `#[serde(default)]`, e.g. a
    // hand-written file that only sets `version`/`next_id`) and must load silently. A
    // `windows` key that *is* present but is not a JSON array is structurally invalid,
    // not defaultable — treated as corrupt like every other malformed part of this file,
    // not silently swallowed into an empty list with zero warnings.
    let raw_windows: &[serde_json::Value] = match object.get("windows") {
        None => &[],
        Some(serde_json::Value::Array(items)) => items,
        Some(_) => {
            return mark_aside_with(
                path,
                "corrupt",
                "state file's \"windows\" field is not an array",
                now,
            );
        }
    };

    // Minor #6: `runs` gets exactly the treatment `windows` above just got, for the same
    // reason — a present-but-wrong-typed field is structurally invalid, not defaultable,
    // and must not be silently swallowed into an empty list with zero warnings. `runs` is
    // always `[]` this milestone and its element type is opaque `serde_json::Value` (see
    // the `StateFile` doc), so any JSON *array* is already valid content — there is
    // nothing per-element left to validate the way the `windows` loop below validates
    // each `WindowRecord`.
    let runs: Vec<serde_json::Value> = match object.get("runs") {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(_) => {
            return mark_aside_with(
                path,
                "corrupt",
                "state file's \"runs\" field is not an array",
                now,
            );
        }
    };

    let mut windows = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_names = HashSet::new();
    for (index, entry) in raw_windows.iter().enumerate() {
        match serde_json::from_value::<WindowRecord>(entry.clone()) {
            Ok(record) => {
                // Both uniqueness checks run *before* either `HashSet` is touched: a
                // record that is ultimately rejected must never claim the id or the
                // name it arrived with, or a later, legitimate record that reuses that
                // id/name would be wrongly rejected as a duplicate even though nothing
                // surviving holds it.
                if seen_ids.contains(&record.id) {
                    problems.push(Problem {
                        severity: Severity::Warn,
                        key: format!("windows[{index}]"),
                        message: format!("id {} repeats an earlier entry's id; skipped", record.id),
                    });
                    continue;
                }
                if seen_names.contains(&record.name) {
                    problems.push(Problem {
                        severity: Severity::Warn,
                        key: format!("windows[{index}]"),
                        message: format!(
                            "name {:?} repeats an earlier entry's name; skipped",
                            record.name
                        ),
                    });
                    continue;
                }
                seen_ids.insert(record.id);
                seen_names.insert(record.name.clone());
                windows.push(record);
            }
            Err(e) => {
                problems.push(Problem {
                    severity: Severity::Warn,
                    key: format!("windows[{index}]"),
                    message: format!("invalid ({e}); skipped"),
                });
            }
        }
    }

    // `saturating_add` rather than `+ 1`: a loaded record already holding `u32::MAX`
    // has no valid successor id, and wrapping to 0 (as an unchecked cast from wider
    // arithmetic would) would immediately collide with the lowest live id. Saturating at
    // `u32::MAX` instead means `next_id` ends up equal to that already-loaded id -
    // deliberately, since there is no other representable choice. Nothing in this module
    // creates windows, so it cannot refuse the collision itself — that refusal is
    // `manager::create`'s `admit` (`crates/daemon/src/manager/create.rs`), which bails
    // outright the moment `next_id` has nowhere left to advance, before it is ever
    // handed out as a new window's id. Loading the file is silent about the collision
    // itself, though, unless this reports it — see Minor #3.
    let next_id = match windows.iter().map(|w| w.id).max() {
        Some(max_id) => {
            let candidate = max_id.saturating_add(1);
            if candidate == max_id {
                problems.push(Problem {
                    severity: Severity::Warn,
                    key: "next_id".to_string(),
                    message: format!(
                        "the highest loaded window id {max_id} has no valid successor; \
                         next_id is also {max_id}, so the next window created will \
                         collide with it until the daemon restarts"
                    ),
                });
            }
            saved_next_id.max(candidate)
        }
        None => saved_next_id,
    };

    (
        StateFile {
            version: STATE_VERSION,
            next_id,
            windows,
            runs,
        },
        problems,
    )
}

/// Renames `path` aside (decision 12) and returns an empty state plus one [`Severity::Warn`]
/// problem naming the new path. The rename itself failing (permissions, a concurrent
/// deletion) is reported instead of panicking, and still leaves the caller with an empty
/// state rather than a corrupt one it would have to guard against everywhere else.
fn mark_aside_with(
    path: &Path,
    tag: &str,
    why: &str,
    now: SystemTime,
) -> (StateFile, Vec<Problem>) {
    let aside = aside_path_with(path, tag, now);
    let message = match std::fs::rename(path, &aside) {
        Ok(()) => format!("{why}; moved it aside to {}", aside.display()),
        Err(e) => format!(
            "{why}; could not move it aside to {} ({e}); starting empty and leaving it in place",
            aside.display()
        ),
    };
    (
        empty_state(),
        vec![Problem {
            severity: Severity::Warn,
            key: "<state file>".to_string(),
            message,
        }],
    )
}

/// Picks `<path>.<tag>-<unix-seconds>`, or `-1`, `-2`, ... appended to that if it is
/// already taken. The suffix search — not the timestamp alone — is what makes two corrupt
/// loads inside the same wall-clock second land on different names.
///
/// `now` is injectable (following `project::detect_roots_with`'s convention) so a test
/// can drive two, three, or more loads through the identical timestamp deterministically,
/// instead of depending on the wall clock not ticking over between two real `load` calls —
/// see `corrupt_file_is_moved_aside`'s history for why that assumption doesn't hold.
pub(super) fn aside_path_with(path: &Path, tag: &str, now: SystemTime) -> PathBuf {
    let ts = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let base = format!("{}.{tag}-{ts}", path.display());
    let candidate = PathBuf::from(&base);
    if !candidate.exists() {
        return candidate;
    }
    let mut n = 1u32;
    loop {
        let candidate = PathBuf::from(format!("{base}-{n}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}
