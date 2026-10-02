//! Decision 6: a GitHub remote's URL, read raw from `remote.<name>.url` (never `git remote
//! get-url`, which applies `insteadOf`). Pure.
//!
//! Accepted: `https://<host>[:port]/<owner>/<repo>[.git]`, `<user>@<host>:<owner>/<repo>
//! [.git]` (scp-like) and `ssh://<user>@<host>[:port]/<owner>/<repo>[.git]`, each with or
//! without a trailing `/`. Any host parses; preflight then refuses a host other than
//! `github.com` that `gh auth status --hostname <host>` does not know (GitHub Enterprise
//! is one it knows). `ssh.github.com` (GitHub's SSH-over-443 endpoint) is `github.com`.

/// A remote's GitHub coordinates: `host` lowercased, `owner` and `name` as written (minus
/// a `.git` suffix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRemote {
    pub host: String,
    pub owner: String,
    pub name: String,
}

pub const GITHUB_HOST: &str = "github.com";

pub fn parse(url: &str) -> Option<GithubRemote> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.strip_prefix("https://") {
        split_authority(rest)?
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        split_authority(rest)?
    } else if !url.contains("://") {
        // scp-like: `<user>@<host>:<path>`; a user is required, so a local path that
        // happens to hold a colon is never read as one.
        let (user_host, path) = url.split_once(':')?;
        let (user, host) = user_host.split_once('@')?;
        if !user_ok(user) || path.starts_with('/') {
            return None;
        }
        (host, path)
    } else {
        return None;
    };
    let host = host.to_ascii_lowercase();
    if !host_ok(&host) {
        return None;
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    if !owner_ok(owner) || !name_ok(name) {
        return None;
    }
    let host = if host == "ssh.github.com" {
        GITHUB_HOST.to_string()
    } else {
        host
    };
    Some(GithubRemote {
        host,
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// `[<userinfo>@]<host>[:<port>]/<path>` → (`host`, `path`).
fn split_authority(rest: &str) -> Option<(&str, &str)> {
    let (authority, path) = rest.split_once('/')?;
    let host_port = match authority.rsplit_once('@') {
        Some((_, host_port)) => host_port,
        None => authority,
    };
    let host = match host_port.split_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        Some(_) => return None,
        None => host_port,
    };
    Some((host, path))
}

/// A URL fit for a message: a scheme URL's user information becomes `***` (it can hold
/// a token), and control characters are escaped.
pub fn redact(url: &str) -> String {
    let shown = match url.split_once("://") {
        Some((scheme, rest)) => {
            let authority_end = rest.find('/').unwrap_or(rest.len());
            match rest[..authority_end].rfind('@') {
                Some(at) => format!("{scheme}://***{}", &rest[at..]),
                None => url.to_string(),
            }
        }
        None => url.to_string(),
    };
    shown.chars().flat_map(char::escape_debug).collect()
}

pub(crate) fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['-', '.'])
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

/// A GitHub user or organisation (Enterprise managed users carry an `_`).
pub(crate) fn owner_ok(owner: &str) -> bool {
    (1..=39).contains(&owner.len())
        && !owner.starts_with('-')
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(crate) fn name_ok(name: &str) -> bool {
    (1..=100).contains(&name.len())
        && name != "."
        && name != ".."
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn user_ok(user: &str) -> bool {
    !user.is_empty()
        && user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}
