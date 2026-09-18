use crate::launch::hook_command;
use proto::HookSource;
use serde_json::{Value, json};
use std::path::Path;

pub const HOOK_EVENTS: [&str; 10] = [
    "Notification",
    "PermissionRequest",
    "PostToolUse",
    "PreToolUse",
    "SessionEnd",
    "SessionStart",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "UserPromptSubmit",
];

pub fn settings(exe: &Path, window_id: u32) -> Value {
    let command = hook_command(exe, window_id, HookSource::Claude);
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        hooks.insert(
            event.to_string(),
            json!([{
                "hooks": [{"command": command, "type": "command"}],
                "matcher": ""
            }]),
        );
    }
    json!({
        "hooks": hooks,
        "preferredNotifChannel": "terminal_bell"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_settings_json_is_exact() {
        let command = "'/opt/anthrex/bin/anthrex' hook --window 4 --source claude";
        let hook = json!([{
            "hooks": [{"command": command, "type": "command"}],
            "matcher": ""
        }]);
        let expected = json!({
            "hooks": {
                "Notification": hook,
                "PermissionRequest": hook,
                "PostToolUse": hook,
                "PreToolUse": hook,
                "SessionEnd": hook,
                "SessionStart": hook,
                "Stop": hook,
                "SubagentStart": hook,
                "SubagentStop": hook,
                "UserPromptSubmit": hook
            },
            "preferredNotifChannel": "terminal_bell"
        });

        let actual = settings(Path::new("/opt/anthrex/bin/anthrex"), 4);
        assert_eq!(actual, expected);
        assert_eq!(
            serde_json::to_string(&actual).unwrap(),
            serde_json::to_string(&expected).unwrap()
        );
    }
}
