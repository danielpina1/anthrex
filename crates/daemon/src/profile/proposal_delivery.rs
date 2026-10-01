//! Milestone 9.2 decision 3: `anthrex profile edit delivery.mode=<pr|local>` and
//! `delivery.remote=<name>`, and the validation of a stored `[delivery]` table. Pure.

use proto::{DeliveryMode, DeliveryProfile, RepoProfile};

/// The two `delivery.*` keys `profile edit` takes.
pub(super) const DELIVERY_KEYS: [&str; 2] = ["delivery.mode", "delivery.remote"];

/// `stored` with `delivery.<sub>` set to `value` (`None`: `--unset`). Unsetting the mode
/// drops the table (absent means `local`); unsetting the remote restores `origin`; a
/// remote set on a profile without the table makes a `local` one.
pub(super) fn edit(
    stored: &RepoProfile,
    key: &str,
    value: Option<&str>,
) -> Result<RepoProfile, String> {
    let mut edited = stored.clone();
    let current = stored.delivery.clone();
    let table = |mode, remote: String| Some(DeliveryProfile { mode, remote });
    let origin = || "origin".to_string();
    edited.delivery = match (key, value) {
        ("delivery.mode", None) => None,
        ("delivery.mode", Some(text)) => {
            let mode = match text {
                "pr" => DeliveryMode::Pr,
                "local" => DeliveryMode::Local,
                _ => return Err("delivery.mode: must be pr or local".to_string()),
            };
            table(mode, current.map_or_else(origin, |d| d.remote))
        }
        ("delivery.remote", None) => current.map(|d| DeliveryProfile {
            remote: origin(),
            ..d
        }),
        ("delivery.remote", Some(name)) => {
            if let Some(problem) = remote_problem(name) {
                return Err(problem);
            }
            let mode = current.map_or(DeliveryMode::Local, |d| d.mode);
            table(mode, name.to_string())
        }
        _ => {
            return Err(format!(
                "unknown key {key}; one of {}",
                DELIVERY_KEYS.join(", ")
            ));
        }
    };
    Ok(edited)
}

/// A stored profile's `[delivery]` problems, in `validate`'s `<key>: <problem>` form.
pub(super) fn problems(profile: &RepoProfile) -> Vec<String> {
    profile
        .delivery
        .as_ref()
        .and_then(|d| remote_problem(&d.remote))
        .into_iter()
        .collect()
}

/// `git check-ref-format --branch`'s rule for a remote's name, applied without
/// running git (checked against git 2.50.1): no empty name or component, no leading `-`
/// or `.` and no trailing `.`, not `HEAD`, no `..`, `@{`, `//`, trailing `/` or `.lock`
/// component, and none of ASCII control characters, space, `~ ^ : ? * [ \`. `@` alone
/// is accepted, as git accepts it.
fn remote_problem(name: &str) -> Option<String> {
    let bad_char = |c: char| c.is_ascii_control() || " ~^:?*[\\".contains(c);
    let bad_part = |part: &str| part.is_empty() || part.starts_with('.') || part.ends_with(".lock");
    let refused = name.is_empty()
        || name == "HEAD"
        || name.starts_with('-')
        || name.ends_with('.')
        || name.contains("..")
        || name.contains("@{")
        || name.chars().any(bad_char)
        || name.split('/').any(bad_part);
    refused.then(|| {
        let shown = redacted(name);
        format!("delivery.remote: {shown} is not a valid remote name")
    })
}

/// `name` as an error may show it (M9.2.3's review fix 5): a URL's or an scp-style
/// address's userinfo (`user:token@`) becomes `***@`, and control characters are
/// escaped, so a pasted token never reaches the terminal or a log.
fn redacted(name: &str) -> String {
    let (scheme, rest) = match name.find("://") {
        Some(at) => name.split_at(at + 3),
        None => ("", name),
    };
    let authority = &rest[..rest.find('/').unwrap_or(rest.len())];
    let rest = match authority.rfind('@') {
        Some(at) if !scheme.is_empty() || authority[..at].contains(':') => {
            format!("***{}", &rest[at..])
        }
        _ => rest.to_string(),
    };
    format!("{scheme}{rest}")
        .chars()
        .map(|c| {
            if c.is_control() {
                c.escape_debug().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}
