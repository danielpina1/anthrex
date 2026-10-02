//! Decision 14's last guard: whether a `gh` argv asks to merge, approve or enable
//! auto-merge, in any spelling `gh` accepts. A superset of decision 14's list:
//!
//! - `gh pr merge` (any flags; `--auto` is "enable auto-merge") and `gh pr review` (any
//!   form), with `-R`/`--repo` in any of their forms before the subcommand;
//! - `gh api` with any positional whose path has a segment starting `merge` (`merge`,
//!   `merges`, `merge-upstream`: "merge") or a `reviews` segment ("approve"), with
//!   method `PUT` ("merge"), any write to `git/refs` ("move a branch"), or a body
//!   containing `mutation` sent to `graphql` (`graphql`, `/graphql`, `graphql/`,
//!   `api/graphql` or a full URL ending so). Paths are percent-decoded first, and a
//!   positional's query string is a body. Flags are read as pflag reads them: `-Xv`,
//!   `-X=v`, `-X v`, combined shorthands (`-iXPUT`), `--flag=v`, `--flag v`; `-F k=@f`
//!   and `--input f` bodies are read from `f` relative to the directory `gh` runs in,
//!   and every string in a JSON body is read too;
//! - `gh repo sync` ("sync").

use std::path::Path;

/// `gh api`'s shorthand flags that take a value.
const SHORT_VALUE: &str = "XfFHqtp";
/// `gh api`'s long flags that take a value.
const LONG_VALUE: [&str; 10] = [
    "--method",
    "--raw-field",
    "--field",
    "--header",
    "--input",
    "--jq",
    "--template",
    "--preview",
    "--cache",
    "--hostname",
];

/// The verb of decision 14's panic text, or `None`.
pub(super) fn landing(args: &[&str], dir: &Path) -> Option<&'static str> {
    match args.split_first()? {
        (&"pr", rest) => pr_landing(rest),
        (&"api", rest) => api_landing(rest, dir),
        // The final fix wave (A3): `gh repo sync` moves a branch to its upstream's.
        (&"repo", rest) => {
            (rest.iter().find(|a| !a.starts_with('-')) == Some(&"sync")).then_some("sync")
        }
        _ => None,
    }
}

fn pr_landing(rest: &[&str]) -> Option<&'static str> {
    let mut i = 0;
    while let Some(arg) = rest.get(i) {
        match *arg {
            "-R" | "--repo" => i += 2,
            a if a.starts_with("--repo=") || (a.starts_with("-R") && a.len() > 2) => i += 1,
            _ => break,
        }
    }
    let (sub, after) = rest.get(i..)?.split_first()?;
    match *sub {
        "merge" => Some(
            if after
                .iter()
                .any(|a| *a == "--auto" || a.starts_with("--auto="))
            {
                "enable auto-merge"
            } else {
                "merge"
            },
        ),
        "review" => Some("approve"),
        _ => None,
    }
}

/// Every value-taking flag of a `gh api` argv with its value (as pflag reads them;
/// a long flag by its shorthand, `--input` as `'I'`), and the positionals.
fn flag_values<'a>(rest: &[&'a str]) -> (Vec<(char, String)>, Vec<&'a str>) {
    let mut values = Vec::new();
    let mut positional = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i];
        i += 1;
        if let Some(long) = arg.strip_prefix("--") {
            let (name, glued) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let flag = format!("--{name}");
            if !LONG_VALUE.contains(&flag.as_str()) {
                continue;
            }
            let value = glued.or_else(|| {
                i += 1;
                rest.get(i - 1).map(|v| v.to_string())
            });
            values.push((short_of(&flag), value.unwrap_or_default()));
        } else if let Some(shorts) = arg.strip_prefix('-').filter(|s| !s.is_empty()) {
            for (at, c) in shorts.char_indices() {
                if !SHORT_VALUE.contains(c) {
                    continue;
                }
                let glued = &shorts[at + c.len_utf8()..];
                let value = if glued.is_empty() {
                    i += 1;
                    rest.get(i - 1).map(|v| v.to_string()).unwrap_or_default()
                } else {
                    glued.strip_prefix('=').unwrap_or(glued).to_string()
                };
                values.push((c, value));
                break;
            }
        } else {
            positional.push(arg);
        }
    }
    (values, positional)
}

/// A long value flag's shorthand (`'I'` for `--input`; `'-'` when none matters here).
fn short_of(flag: &str) -> char {
    match flag {
        "--method" => 'X',
        "--raw-field" => 'f',
        "--field" => 'F',
        "--input" => 'I',
        _ => '-',
    }
}

/// A positional as a path, percent-decoded (deferred from task 5): no `scheme://host/`,
/// no leading or trailing `/`, no query.
fn path_of(positional: &str) -> String {
    let rest = match positional.split_once("://") {
        Some((_, after)) => after.split_once('/').map_or("", |(_, p)| p),
        None => positional,
    };
    let path = rest.split(['?', '#']).next().unwrap_or("");
    decode(path).trim_matches('/').to_string()
}

/// A positional's query string, percent-decoded: `gh api` sends it as given, so a
/// `graphql?query=mutation…` is a mutation (deferred from task 5).
fn query_of(positional: &str) -> Option<String> {
    let (_, query) = positional.split_once('?')?;
    Some(decode(query.split('#').next().unwrap_or("")))
}

/// `%XX` decoded (an invalid escape is kept as written).
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A body file's text, and every string inside it when it is JSON, so an escaped
/// `\u006dutation` reads as `mutation` (deferred from task 5).
fn read(dir: &Path, file: &str) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join(file)).unwrap_or_default();
    let mut found = Vec::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        strings_in(&value, &mut found);
    }
    found.push(text);
    found
}

fn strings_in(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| strings_in(v, out)),
        serde_json::Value::Object(map) => map.iter().for_each(|(k, v)| {
            out.push(k.clone());
            strings_in(v, out);
        }),
        _ => {}
    }
}

/// Whether a path's segments name `wanted`. The segment after `collaborators` (or
/// `users`) is a login, never read as a verb: a reviewer may be called `mergebot`.
fn has_segment(path: &str, wanted: impl Fn(&str) -> bool) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    segments.iter().enumerate().any(|(at, s)| {
        let login = at > 0 && matches!(segments[at - 1], "collaborators" | "users");
        !login && wanted(s)
    })
}

fn api_landing(rest: &[&str], dir: &Path) -> Option<&'static str> {
    let (values, positional) = flag_values(rest);
    let paths: Vec<String> = positional.iter().map(|p| path_of(p)).collect();
    let segment = |wanted: &dyn Fn(&str) -> bool| paths.iter().any(|p| has_segment(p, wanted));
    let mut bodies: Vec<String> = positional.iter().filter_map(|p| query_of(p)).collect();
    let mut method = None;
    for (flag, value) in values {
        match flag {
            'X' => method = Some(value.to_ascii_uppercase()),
            'f' => bodies.push(value),
            'F' => match value.split_once("=@") {
                Some((_, file)) => bodies.extend(read(dir, file)),
                None => bodies.push(value),
            },
            'I' => bodies.extend(read(dir, &value)),
            _ => {}
        }
    }
    // `gh api` sends a GET unless fields or an input make it a POST.
    let method = method.unwrap_or_else(|| {
        let body = rest.iter().any(|a| {
            ["-f", "-F", "--raw-field", "--field", "--input"]
                .iter()
                .any(|f| a.starts_with(f))
        });
        (if body { "POST" } else { "GET" }).to_string()
    });
    // Deferred from task 5: a write to a branch's ref is a landing (anthrex pushes its
    // stage refs through git, never through the API).
    let writes = ["POST", "PATCH", "PUT", "DELETE"].contains(&method.as_str());
    if writes && paths.iter().any(|p| p.contains("git/refs")) {
        return Some("move a branch");
    }
    // A3: any segment that starts with `merge` (`merge`, `merges`, `merge-upstream`).
    if segment(&|s| s.starts_with("merge")) {
        return Some("merge");
    }
    if segment(&|s| s == "reviews") {
        return Some("approve");
    }
    if method == "PUT" {
        return Some("merge");
    }
    let graphql = paths.iter().any(|p| p == "graphql" || p == "api/graphql");
    let mutation = bodies.iter().find(|b| b.contains("mutation"));
    let lower = mutation.filter(|_| graphql)?.to_ascii_lowercase();
    Some(if lower.contains("automerge") {
        "enable auto-merge"
    } else if lower.contains("review") {
        "approve"
    } else {
        "merge"
    })
}
