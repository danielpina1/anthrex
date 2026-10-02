//! Decision 14's last guard: whether a `gh` argv asks to merge, approve or enable
//! auto-merge, in any spelling `gh` accepts. A superset of decision 14's list:
//!
//! - `gh pr merge` (any flags; `--auto` is "enable auto-merge") and `gh pr review` (any
//!   form), with `-R`/`--repo` in any of their forms before the subcommand;
//! - `gh api` with any positional whose path has a `merge` or `merges` segment
//!   ("merge") or a `reviews` segment ("approve"), with method `PUT` ("merge"), or a
//!   body containing `mutation` sent to `graphql` (`graphql`, `/graphql`,
//!   `api/graphql` or a full URL ending so). Flags are read as pflag reads them: `-Xv`,
//!   `-X=v`, `-X v`, combined shorthands (`-iXPUT`), `--flag=v`, `--flag v`; `-F k=@f`
//!   and `--input f` bodies are read from `f` relative to the directory `gh` runs in.

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

/// A positional as a path: no `scheme://host/`, no leading `/`, no query.
fn path_of(positional: &str) -> &str {
    let rest = match positional.split_once("://") {
        Some((_, after)) => after.split_once('/').map_or("", |(_, p)| p),
        None => positional,
    };
    let rest = rest.trim_start_matches('/');
    rest.split(['?', '#']).next().unwrap_or("")
}

fn read(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).unwrap_or_default()
}

fn api_landing(rest: &[&str], dir: &Path) -> Option<&'static str> {
    let (values, positional) = flag_values(rest);
    let paths: Vec<&str> = positional.iter().map(|p| path_of(p)).collect();
    let segment = |wanted: &[&str]| {
        paths
            .iter()
            .any(|p| p.split('/').any(|s| wanted.contains(&s)))
    };
    if segment(&["merge", "merges"]) {
        return Some("merge");
    }
    if segment(&["reviews"]) {
        return Some("approve");
    }
    let mut bodies = Vec::new();
    for (flag, value) in values {
        match flag {
            'X' if value.eq_ignore_ascii_case("PUT") => return Some("merge"),
            'f' => bodies.push(value),
            'F' => bodies.push(match value.split_once("=@") {
                Some((_, file)) => read(dir, file),
                None => value,
            }),
            'I' => bodies.push(read(dir, &value)),
            _ => {}
        }
    }
    let graphql = paths.iter().any(|p| *p == "graphql" || *p == "api/graphql");
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
