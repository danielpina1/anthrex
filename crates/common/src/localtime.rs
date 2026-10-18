//! The local zone, read (milestone 9.19, decision 3, rulings R2 and R4). The one file in
//! the workspace that calls `localtime_r` or reads `tm_gmtoff` (the `one_local_clock`
//! source rule): reading the zone is I/O (`/etc/localtime`, `TZ`), so the pure code is
//! handed the number instead. The TUI's event loop calls [`offset_secs`] every tick, the
//! CLI once per command, and `fake-agent` takes [`parts`] for its rollout directories.
//! The daemon never calls this module: its text is UTC.

/// A Unix time's civil date and time in the local zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalParts {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
}

/// The local zone's offset from UTC at `at` (Unix seconds), in seconds east of UTC. 0
/// (UTC) when the zone cannot be read. `tm_gmtoff` is a C `long`, converted without
/// truncation: a value outside `i32` (none is real) also reads as 0.
pub fn offset_secs(at: u64) -> i32 {
    local_tm(at).map_or(0, |tm| i32::try_from(tm.tm_gmtoff).unwrap_or(0))
}

/// `at`'s civil date and time in the local zone; in UTC when the zone cannot be read.
pub fn parts(at: u64) -> LocalParts {
    local_tm(at).map_or_else(|| utc_parts(at), |tm| from_tm(&tm))
}

/// `localtime_r` of `at`; `None` when it fails or `at` is past `time_t`.
fn local_tm(at: u64) -> Option<libc::tm> {
    let secs = libc::time_t::try_from(at).ok()?;
    // SAFETY: `localtime_r` reads `secs` and writes only into `tm`, a plain C struct
    // zeroed here that we own; it returns null on failure, which is checked.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return None;
        }
        Some(tm)
    }
}

/// The fields of a `struct tm` (years since 1900, months from 0) as civil values.
fn from_tm(tm: &libc::tm) -> LocalParts {
    let field = |v: libc::c_int| u32::try_from(v).unwrap_or(0);
    LocalParts {
        year: tm.tm_year.saturating_add(1900),
        month: field(tm.tm_mon).saturating_add(1),
        day: field(tm.tm_mday),
        hour: field(tm.tm_hour),
        min: field(tm.tm_min),
        sec: field(tm.tm_sec),
    }
}

/// `at` in UTC, by the same civil arithmetic as `numbers`.
fn utc_parts(at: u64) -> LocalParts {
    let (y, month, day) = crate::numbers::civil_from_days((at / 86_400) as i64);
    let secs = (at % 86_400) as u32;
    LocalParts {
        year: i32::try_from(y).unwrap_or(i32::MAX),
        month,
        day,
        hour: secs / 3600,
        min: secs % 3600 / 60,
        sec: secs % 60,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every real zone is within UTC-12 and UTC+14, and on a whole quarter hour. The one
    /// test that reads the machine's zone (ruling R23): it pins no label, since CI runs
    /// in UTC and a developer's machine need not.
    #[test]
    fn the_local_offset_is_a_real_zone_offset() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let offset = offset_secs(now);
        assert!((-12 * 3600..=14 * 3600).contains(&offset), "{offset}");
        assert_eq!(offset % 900, 0, "{offset}");
    }

    /// A `struct tm` counts years from 1900 and months from 0.
    #[test]
    fn a_struct_tm_reads_as_civil_parts() {
        // SAFETY: an all-zero `tm` is a valid value of a plain C struct.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = 126;
        tm.tm_mon = 9;
        tm.tm_mday = 17;
        tm.tm_hour = 16;
        tm.tm_min = 30;
        tm.tm_sec = 5;
        assert_eq!(
            from_tm(&tm),
            LocalParts {
                year: 2026,
                month: 10,
                day: 17,
                hour: 16,
                min: 30,
                sec: 5
            }
        );
    }

    /// The fallback when the zone cannot be read is UTC.
    #[test]
    fn the_fallback_is_utc() {
        // 2026-10-17 14:30:05 UTC.
        assert_eq!(
            utc_parts(1_792_247_405),
            LocalParts {
                year: 2026,
                month: 10,
                day: 17,
                hour: 14,
                min: 30,
                sec: 5
            }
        );
        assert_eq!(utc_parts(0).year, 1970);
    }
}
