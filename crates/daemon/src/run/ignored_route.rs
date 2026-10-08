//! Milestone 9.8 decision 31, task M9.8.9's fix round 1 (controller ruling): a route
//! from the orchestrator, a sub-planner or a plan file is ignored, so none of its values
//! may refuse the call or the plan, not even ones `RouteSpec`'s serde shape cannot hold
//! (an unknown runtime; any strength is read and ignored since M9.8.14). Before serde
//! reads them, such a route is replaced by [`NAMED`]'s stand-in when it named anything,
//! and dropped when it named nothing;
//! the edit batch and `build_run` then clear the stand-in and note it
//! (`contract::ROUTE_IGNORED`). The bounds `tools_bounds` checks on the raw value come
//! first and stay. A user's route (`run edit`, the task edit form) never comes here.
//! Pure.

use proto::Plan;
use serde_json::Value;

/// The effort of the stand-in route: any string is an `Effort` to serde, and a route
/// that names one is not the default, so the batch and the build note it. It is
/// cleared before resolution and never reaches a session.
pub const NAMED: &str = "ignored";

/// `edit`'s routes (`task.route`, each `into[].route`, an amend's `route`), from an
/// orchestrator's or a sub-planner's tool call, made readable whatever they held.
pub fn in_edit(edit: &mut Value) {
    let Some(map) = edit.as_object_mut() else {
        return;
    };
    replace_json(map);
    if let Some(task) = map.get_mut("task").and_then(Value::as_object_mut) {
        replace_json(task);
    }
    if let Some(into) = map.get_mut("into").and_then(Value::as_array_mut) {
        for task in into.iter_mut().filter_map(Value::as_object_mut) {
            replace_json(task);
        }
    }
}

fn replace_json(map: &mut serde_json::Map<String, Value>) {
    let named = match map.get("route") {
        None | Some(Value::Null) => return,
        Some(Value::Object(route)) => route.values().any(|v| !v.is_null()),
        Some(_) => true,
    };
    if named {
        map.insert("route".into(), serde_json::json!({ "effort": NAMED }));
    } else {
        map.remove("route");
    }
}

/// A plan file as `run start --plan` reads it: as written when it parses, else with
/// every task's route made readable (named: [`NAMED`]'s stand-in; empty: dropped). A
/// plan that fails either way keeps the first parse's error, which names its line.
pub fn plan_file(text: &str) -> Result<Plan, String> {
    let strict = toml::from_str::<Plan>(text).map_err(|e| e.to_string());
    let Err(error) = strict else { return strict };
    let Ok(mut value) = toml::from_str::<toml::Value>(text) else {
        return Err(error);
    };
    let tasks = value.get_mut("task").and_then(toml::Value::as_array_mut);
    for task in tasks.into_iter().flatten() {
        let Some(task) = task.as_table_mut() else {
            continue;
        };
        let named = match task.get("route") {
            None => continue,
            Some(toml::Value::Table(route)) => !route.is_empty(),
            Some(_) => true,
        };
        if named {
            let mut stand_in = toml::Table::new();
            stand_in.insert("effort".into(), toml::Value::String(NAMED.into()));
            task.insert("route".into(), toml::Value::Table(stand_in));
        } else {
            task.remove("route");
        }
    }
    value.try_into::<Plan>().map_err(|_| error)
}
