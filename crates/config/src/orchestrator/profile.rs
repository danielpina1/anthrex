//! `[orchestrator.profile]` and `[orchestrator.profile.env]` (decision 56, and the
//! Interfaces block's `pub profile: proto::ProfileSpec`). Split out of
//! `orchestrator.rs` to keep that file under the 600-line rule.
//!
//! `protected` holds the user's additions only: the built-ins the daemon's
//! `run::plan::BUILTIN_PROTECTED` names are added by `resolve_profile` (M8a.5), which
//! this crate cannot see (`crates/config/Cargo.toml` depends on `proto`, `toml` and
//! `unicode-width` only).

use super::Orchestrator;
use crate::{Problem, not_a_table_problem, unknown_key_problem};

const KNOWN_PROFILE_KEYS: &[&str] = &[
    "modules",
    "hub",
    "source",
    "check",
    "check_timeout_secs",
    "single_test",
    "test_passed",
    "setup",
    "generated",
    "protected",
    "env",
];

pub(super) fn read_profile(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("profile") else {
        return;
    };
    let Some(profile) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.profile"));
        return;
    };

    read_profile_string(profile, "check", &mut o.profile.check, problems);
    read_profile_string(profile, "single_test", &mut o.profile.single_test, problems);
    read_profile_string(profile, "test_passed", &mut o.profile.test_passed, problems);
    read_profile_string(profile, "setup", &mut o.profile.setup, problems);
    read_profile_string_list(profile, "modules", &mut o.profile.modules, problems);
    read_profile_string_list(profile, "hub", &mut o.profile.hub, problems);
    read_profile_string_list(profile, "source", &mut o.profile.source, problems);
    read_profile_string_list(profile, "generated", &mut o.profile.generated, problems);
    read_profile_string_list(profile, "protected", &mut o.profile.protected, problems);

    if let Some(v) = profile.get("check_timeout_secs") {
        match v
            .as_integer()
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n >= 1)
        {
            Some(n) => o.profile.check_timeout_secs = Some(n),
            None => problems.push(Problem {
                key: "orchestrator.profile.check_timeout_secs".to_string(),
                message: "must be at least 1".to_string(),
                default: match o.profile.check_timeout_secs {
                    Some(n) => n.to_string(),
                    None => "unset".to_string(),
                },
            }),
        }
    }

    read_profile_env(profile, o, problems);
}

fn read_profile_string(
    table: &toml::Table,
    key: &str,
    field: &mut Option<String>,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get(key) else {
        return;
    };
    match value.as_str() {
        Some(s) => *field = Some(s.to_string()),
        None => problems.push(Problem {
            key: format!("orchestrator.profile.{key}"),
            message: "expected a string".to_string(),
            default: "unset".to_string(),
        }),
    }
}

fn read_profile_string_list(
    table: &toml::Table,
    key: &str,
    field: &mut Option<Vec<String>>,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get(key) else {
        return;
    };
    let Some(array) = value.as_array() else {
        problems.push(Problem {
            key: format!("orchestrator.profile.{key}"),
            message: "expected an array of strings".to_string(),
            default: "unset".to_string(),
        });
        return;
    };
    let mut out = Vec::new();
    for item in array {
        match item.as_str() {
            Some(s) => out.push(s.to_string()),
            None => {
                problems.push(Problem {
                    key: format!("orchestrator.profile.{key}"),
                    message: "expected an array of strings".to_string(),
                    default: "unset".to_string(),
                });
                return;
            }
        }
    }
    *field = Some(out);
}

fn read_profile_env(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("env") else {
        return;
    };
    let Some(env) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.profile.env"));
        return;
    };
    let mut map = std::collections::BTreeMap::new();
    for (k, v) in env {
        // Final fix batch F2 (C-I4): the keys a launch scrubs or sets on purpose, and
        // those that choose what an agent loads, are refused here as in a plan.
        if let Some(reason) = crate::reserved_env::reserved_env(k) {
            problems.push(Problem {
                key: format!("orchestrator.profile.env.{k}"),
                message: format!("may not be set by a profile: {reason}"),
                default: "unset".to_string(),
            });
            continue;
        }
        match v.as_str() {
            Some(s) => {
                map.insert(k.clone(), s.to_string());
            }
            None => problems.push(Problem {
                key: format!("orchestrator.profile.env.{k}"),
                message: "expected a string".to_string(),
                default: "unset".to_string(),
            }),
        }
    }
    o.profile.env = Some(map);
}

pub(super) fn report_unknown_profile(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for key in table.keys() {
        if !KNOWN_PROFILE_KEYS.contains(&key.as_str()) {
            problems.push(unknown_key_problem(&format!("orchestrator.profile.{key}")));
        }
    }
}

/// `[orchestrator.cache_dirs]` (M8a final fix batch F1c round 3, N3): a table keyed by
/// repository root, each value the directories a confined check, proof or `setup` in
/// that repository may also write. Only the user's own config sets this (never a plan
/// or the repo profile), because a `cache_dirs` entry can name a place whose files run
/// as the user.
pub(super) fn read_cache_dirs(
    table: &toml::Table,
    o: &mut Orchestrator,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("cache_dirs") else {
        return;
    };
    let Some(map) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.cache_dirs"));
        return;
    };
    let mut out = std::collections::BTreeMap::new();
    for (root, dirs) in map {
        let Some(array) = dirs.as_array() else {
            problems.push(Problem {
                key: format!("orchestrator.cache_dirs.{root:?}"),
                message: "expected an array of strings".to_string(),
                default: "unset".to_string(),
            });
            continue;
        };
        let mut list = Vec::new();
        let mut ok = true;
        for item in array {
            match item.as_str() {
                // M8a final fix batch F1d (R3): an entry is absolute, or `~/…`. A relative
                // one used to mean "in the checkout", which the run itself can rewrite.
                Some(s) if !(s.starts_with('/') || s.starts_with("~/")) => {
                    problems.push(Problem {
                        key: format!("orchestrator.cache_dirs.{root:?}"),
                        message: format!(
                            "{s:?} is not an absolute path; each entry must start with / or ~/ \
                             (the checkout itself is already writable)"
                        ),
                        default: "unset".to_string(),
                    });
                    ok = false;
                    break;
                }
                Some(s) => list.push(s.to_string()),
                None => {
                    problems.push(Problem {
                        key: format!("orchestrator.cache_dirs.{root:?}"),
                        message: "expected an array of strings".to_string(),
                        default: "unset".to_string(),
                    });
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            out.insert(root.clone(), list);
        }
    }
    o.cache_dirs = out;
}

/// `[orchestrator.confined_network]` (M8a final fix batch F1d, R4): a table keyed by
/// repository root, each value whether confined checks, proofs and `setup` in that
/// repository may use the network (TCP, UDP and Unix sockets, except the anthrex
/// daemon's). Off unless named. Only the user's own config sets this.
pub(super) fn read_confined_network(
    table: &toml::Table,
    o: &mut Orchestrator,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("confined_network") else {
        return;
    };
    let Some(map) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.confined_network"));
        return;
    };
    let mut out = std::collections::BTreeMap::new();
    for (root, on) in map {
        match on.as_bool() {
            Some(on) => {
                out.insert(root.clone(), on);
            }
            None => problems.push(Problem {
                key: format!("orchestrator.confined_network.{root:?}"),
                message: "expected true or false".to_string(),
                default: "false".to_string(),
            }),
        }
    }
    o.confined_network = out;
}

/// `[orchestrator.confined_unix_sockets]` (M8a final fix batch F1d round 2, S1): a table
/// keyed by repository root, each value the absolute paths of Unix sockets (a local
/// database's, Docker's) confined checks there may connect to. A relative entry is a
/// problem and drops that repository's list. Only the user's own config sets this.
pub(super) fn read_confined_unix_sockets(
    table: &toml::Table,
    o: &mut Orchestrator,
    problems: &mut Vec<Problem>,
) {
    let key = "confined_unix_sockets";
    let Some(map) = keyed_table(table, key, problems) else {
        return;
    };
    for (root, value) in map {
        let items = value.as_array().and_then(|a| {
            a.iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        });
        let problem = match &items {
            None => Some("expected an array of strings".to_string()),
            Some(list) => list
                .iter()
                .find(|s| !(s.starts_with('/') || s.starts_with("~/")))
                .map(|s| {
                    format!("{s:?} is not an absolute path; each entry must start with / or ~/")
                }),
        };
        match (items, problem) {
            (Some(list), None) => {
                o.confined_unix_sockets.insert(root.clone(), list);
            }
            (_, Some(message)) => problems.push(Problem {
                key: format!("orchestrator.{key}.{root:?}"),
                message,
                default: "unset".to_string(),
            }),
            (None, None) => {}
        }
    }
}

/// `[orchestrator.confined_localhost_ports]` (M8a final fix batch F1d round 2, S1/S2): a
/// table keyed by repository root, each value the loopback ports confined checks there
/// may connect to, bind and accept on (a test server, a local database). Only the
/// user's own config sets this.
pub(super) fn read_confined_localhost_ports(
    table: &toml::Table,
    o: &mut Orchestrator,
    problems: &mut Vec<Problem>,
) {
    let key = "confined_localhost_ports";
    let Some(map) = keyed_table(table, key, problems) else {
        return;
    };
    for (root, value) in map {
        let ports = value.as_array().and_then(|a| {
            a.iter()
                .map(|v| {
                    v.as_integer()
                        .and_then(|n| u16::try_from(n).ok())
                        .filter(|p| *p > 0)
                })
                .collect::<Option<Vec<u16>>>()
        });
        match ports {
            Some(ports) => {
                o.confined_localhost_ports.insert(root.clone(), ports);
            }
            None => problems.push(Problem {
                key: format!("orchestrator.{key}.{root:?}"),
                message: "expected an array of port numbers from 1 to 65535".to_string(),
                default: "unset".to_string(),
            }),
        }
    }
}

/// `[orchestrator.<key>]` as a table, or a problem when it is something else.
fn keyed_table<'a>(
    table: &'a toml::Table,
    key: &str,
    problems: &mut Vec<Problem>,
) -> Option<&'a toml::Table> {
    let value = table.get(key)?;
    let map = value.as_table();
    if map.is_none() {
        problems.push(not_a_table_problem(&format!("orchestrator.{key}")));
    }
    map
}
