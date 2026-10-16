//! Milestone 9.16: the quota steps, in the shapes of M9.16.1's captures.
//!
//! `rate_limit` is a Claude `rate_limit_event` (claude 2.1.280,
//! `crates/daemon/tests/fixtures/headless/claude-2.1.280-rate-limit.jsonl`) whose
//! `unifiedWindows` carry the step's `utilization` fractions; Codex's `exec --json` has no
//! such line, so on Codex it writes nothing. `rollout` appends a `token_count` line
//! (codex-cli 0.160.1, `crates/daemon/tests/fixtures/transcripts/codex-0.160.1-rollout-limits.jsonl`)
//! with the step's `rate_limits` windows to the session's rollout under
//! `$CODEX_HOME/sessions`; Claude has no rollout, so on Claude it writes nothing. A
//! rollout is never written without `CODEX_HOME`: never under a developer's `~/.codex`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::script::Usage;

/// `{"rate_limit": {"five_hour": <fraction>, "seven_day": <fraction>}}`, each optional.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimit {
    #[serde(default)]
    pub five_hour: Option<f64>,
    #[serde(default)]
    pub seven_day: Option<f64>,
}

/// `{"rollout": {"primary": <window>, "secondary": <window>}}`, each optional.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rollout {
    #[serde(default)]
    pub primary: Option<RolloutWindow>,
    #[serde(default)]
    pub secondary: Option<RolloutWindow>,
}

/// One `rate_limits` slot: a **percentage**, and the window's length.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RolloutWindow {
    pub used_percent: f64,
    pub window_minutes: u64,
}

/// The window 0.160.1 reports (M9.16.1 finding 3).
const MODEL_CONTEXT_WINDOW: u64 = 475_000;
const FIVE_HOURS: u64 = 5 * 3600;
const SEVEN_DAYS: u64 = 7 * 86_400;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The `rate_limit_info` of a `rate_limit_event` at `now` (Unix seconds): `allowed`, its
/// `rateLimitType` the first window given, each window resetting a window from now.
pub fn rate_limit_info(limit: &RateLimit, now: u64) -> Value {
    let windows = [
        ("five_hour", limit.five_hour, now + FIVE_HOURS),
        ("seven_day", limit.seven_day, now + SEVEN_DAYS),
    ];
    let mut unified = serde_json::Map::new();
    let mut top = None;
    for (key, used, resets) in windows {
        if let Some(used) = used {
            top.get_or_insert((key, resets));
            unified.insert(key.into(), json!({"utilization": used, "resetsAt": resets}));
        }
    }
    let (kind, resets) = top.unwrap_or(("five_hour", now + FIVE_HOURS));
    json!({
        "status": "allowed",
        "resetsAt": resets,
        "rateLimitType": kind,
        "overageStatus": "rejected",
        "overageDisabledReason": "org_level_disabled",
        "isUsingOverage": false,
        "unifiedWindows": unified,
    })
}

/// The `token_count` line at `now` for `usage` and the step's windows.
pub fn token_count(rollout: &Rollout, usage: Usage, now: u64) -> Value {
    let slot = |w: Option<RolloutWindow>| match w {
        Some(w) => json!({
            "used_percent": w.used_percent,
            "window_minutes": w.window_minutes,
            "resets_at": now + w.window_minutes * 60,
        }),
        None => Value::Null,
    };
    let tokens = json!({
        "input_tokens": usage.input + usage.cache_read,
        "cached_input_tokens": usage.cache_read,
        "cache_write_input_tokens": usage.cache_write,
        "output_tokens": usage.output,
        "reasoning_output_tokens": 0,
        "total_tokens": usage.input + usage.cache_read + usage.output,
    });
    json!({
        "timestamp": crate::headless::timestamp(),
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "total_token_usage": tokens,
                "last_token_usage": tokens,
                "model_context_window": MODEL_CONTEXT_WINDOW,
            },
            "rate_limits": {
                "limit_id": "codex",
                "limit_name": null,
                "primary": slot(rollout.primary),
                "secondary": slot(rollout.secondary),
                "credits": null,
                "individual_limit": null,
                "spend_control_reached": null,
                "plan_type": null,
                "rate_limit_reached_type": null,
            },
        },
    })
}

/// `$CODEX_HOME`, which the daemon's environment gives its sessions; an error without it.
pub fn codex_home() -> Result<PathBuf> {
    match std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        Some(home) => Ok(PathBuf::from(home)),
        None => bail!("rollout: CODEX_HOME is unset; fake-agent never writes under ~/.codex"),
    }
}

/// Appends `line` to thread `thread`'s rollout under `<home>/sessions`, made at `now` if
/// it has none: `<YYYY>/<MM>/<DD>/rollout-<YYYY-MM-DD>T<HH-MM-SS>-<thread>.jsonl` in
/// **local time**, as codex-cli names it (M9.16.1 finding 3, ruling C27), starting with
/// its `session_meta` line. The file's path.
pub fn append_rollout(home: &Path, thread: &str, line: &Value, now: u64) -> Result<PathBuf> {
    let t = local(now);
    let dir = home
        .join("sessions")
        .join(format!("{:04}", t.tm_year + 1900))
        .join(format!("{:02}", t.tm_mon + 1))
        .join(format!("{:02}", t.tm_mday));
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let suffix = format!("-{thread}.jsonl");
    let existing = fs::read_dir(&dir)?.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix))
    });
    let (path, fresh) = match existing {
        Some(path) => (path, false),
        None => {
            let name = format!(
                "rollout-{:04}-{:02}-{:02}T{:02}-{:02}-{:02}{suffix}",
                t.tm_year + 1900,
                t.tm_mon + 1,
                t.tm_mday,
                t.tm_hour,
                t.tm_min,
                t.tm_sec
            );
            (dir.join(name), true)
        }
    };
    let mut text = String::new();
    if fresh {
        let meta = json!({"timestamp": crate::headless::timestamp(), "type": "session_meta",
            "payload": {"id": thread, "session_id": thread, "source": "exec"}});
        text.push_str(&format!("{meta}\n"));
    }
    text.push_str(&format!("{line}\n"));
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    Ok(path)
}

/// `now` in the local zone.
fn local(now: u64) -> libc::tm {
    let secs = libc::time_t::try_from(now).unwrap_or(0);
    // SAFETY: `localtime_r` writes only into `tm`, a plain C struct zeroed here.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daemon::headless::quota::claude_rate_limit;
    use daemon::transcript::fill::codex_limits;
    use daemon::transcript::rollout::{TRANSCRIPT_TAIL_BYTES, find_rollout, read_tail};
    use proto::brakes::{Permille, QuotaWindow};

    /// The daemon's own parser reads the step's fraction as per mille, never as a
    /// percent (review focus 1).
    #[test]
    fn rate_limit_info_is_read_as_a_fraction() {
        let limit = RateLimit {
            five_hour: Some(0.2),
            seven_day: Some(0.97),
        };
        let sample = claude_rate_limit(&rate_limit_info(&limit, 1_000)).unwrap();
        assert_eq!(
            sample.windows,
            vec![
                (
                    QuotaWindow::FiveHour,
                    Permille(200),
                    Some(1_000 + FIVE_HOURS)
                ),
                (
                    QuotaWindow::SevenDay,
                    Permille(970),
                    Some(1_000 + SEVEN_DAYS)
                ),
            ]
        );
    }

    /// The rollout lands where the daemon's `find_rollout` looks for the thread started
    /// at `now`, and `codex_limits` reads its window by length, as a percentage.
    #[test]
    fn the_rollout_is_found_and_read_by_the_daemon() {
        let home = tempfile::tempdir().unwrap();
        let now = now();
        let step = Rollout {
            primary: None,
            secondary: Some(RolloutWindow {
                used_percent: 99.0,
                window_minutes: 10080,
            }),
        };
        let line = token_count(&step, Usage::default(), now);
        let path = append_rollout(home.path(), "thread-1", &line, now).unwrap();
        // A second line goes to the same file.
        assert_eq!(
            append_rollout(home.path(), "thread-1", &line, now).unwrap(),
            path
        );
        let sessions = home.path().join("sessions");
        assert_eq!(find_rollout(&sessions, "thread-1", now), Some(path.clone()));
        let tail = read_tail(&path, TRANSCRIPT_TAIL_BYTES).unwrap();
        assert_eq!(tail.lines().count(), 3);
        let sample = codex_limits(&tail).unwrap();
        assert_eq!(
            sample.windows,
            vec![(QuotaWindow::SevenDay, Permille(990), Some(now + 10080 * 60))]
        );
    }
}
