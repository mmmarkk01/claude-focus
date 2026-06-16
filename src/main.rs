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

    if !should_act(notification_type, &config.notify_types) {
        return Ok(());
    }

    dispatch(notification_type, message, &config, false);

    Ok(())
}

/// Whether this notification type should trigger action, given the allowlist.
/// An empty/missing type is treated as non-matching: it must be explicitly
/// listed to act (fixes the old empty-type bypass).
fn should_act(notification_type: &str, notify_types: &[String]) -> bool {
    notify_types.iter().any(|t| t == notification_type)
}

/// Perform the focus and/or notify actions for a notification.
/// Shared by the real hook path (`force = false`, honors `mode`) and the
/// `test` subcommand (`force = true`, fires BOTH legs regardless of `mode`).
fn dispatch(notification_type: &str, message: &str, config: &config::Config, force: bool) {
    let should_focus = force || config.mode == Mode::Both || config.mode == Mode::FocusOnly;
    let should_notify = force || config.mode == Mode::Both || config.mode == Mode::NotifyOnly;

    if should_focus {
        if let Some(pid) = process_tree::find_terminal_pid() {
            dbus::highlight_window(pid, config.notification_timeout_ms);
        }
    }
    if should_notify {
        notify::send_notification(notification_type, message, config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> Vec<String> {
        vec!["permission_prompt".into(), "idle_prompt".into()]
    }

    #[test]
    fn configured_type_acts() {
        assert!(should_act("idle_prompt", &types()));
    }

    #[test]
    fn unconfigured_type_skips() {
        assert!(!should_act("auth_success", &types()));
    }

    #[test]
    fn empty_type_skips_by_default() {
        // The old code let an empty type bypass the allowlist and always fire.
        assert!(!should_act("", &types()));
    }

    #[test]
    fn empty_type_acts_only_if_explicitly_listed() {
        assert!(should_act("", &[String::new()]));
    }
}
