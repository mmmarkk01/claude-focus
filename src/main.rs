mod config;
mod dbus;
mod notify;
mod process_tree;

use config::Mode;
use serde::Deserialize;
use std::io::Read;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HookInput {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    hook_event_name: Option<String>,
    #[serde(default)]
    notification_type: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

fn main() {
    // Always exit 0 — never block Claude Code
    if let Err(e) = run() {
        eprintln!("claude-focus: {e}");
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;

    let hook_input: HookInput = serde_json::from_str(&input)?;
    let config = config::load_config();

    let notification_type = hook_input.notification_type.as_deref().unwrap_or("");
    let message = hook_input.message.as_deref().unwrap_or("");

    // Check if this notification type is configured to trigger action
    if !notification_type.is_empty()
        && !config.notify_types.iter().any(|t| t == notification_type)
    {
        return Ok(());
    }

    let should_focus = config.mode == Mode::Both || config.mode == Mode::FocusOnly;
    let should_notify = config.mode == Mode::Both || config.mode == Mode::NotifyOnly;

    if should_focus {
        if let Some(pid) = process_tree::find_terminal_pid() {
            dbus::highlight_window(pid, config.notification_timeout_ms);
        }
    }

    if should_notify {
        notify::send_notification(notification_type, message, &config);
    }

    Ok(())
}
