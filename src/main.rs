mod config;
mod dbus;
mod doctor;
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
    // Always exit 0 — never block Claude Code.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match parse_args(&args) {
        Command::Hook => run(),
        Command::Version => {
            println!("claude-focus {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Test(t) => run_test(t.as_deref()),
        Command::Doctor => run_doctor(),
        Command::Help => {
            print_help();
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("claude-focus: {e}");
    }
}

#[derive(Debug)]
enum Command {
    Hook,
    Version,
    Test(Option<String>),
    Doctor,
    Help,
}

/// Parse CLI args (already stripped of argv[0]). No args (or `hook`) preserves
/// the stdin-hook contract Claude Code relies on; anything unknown shows help
/// rather than blocking on stdin.
fn parse_args(args: &[String]) -> Command {
    match args.first().map(|s| s.as_str()) {
        None | Some("hook") => Command::Hook,
        Some("--version") | Some("-V") => Command::Version,
        Some("doctor") => Command::Doctor,
        Some("test") => Command::Test(args.get(1).cloned()),
        Some("--help") | Some("-h") => Command::Help,
        Some(_) => Command::Help,
    }
}

fn print_help() {
    eprintln!(
        "claude-focus {}\n\n\
         Usage:\n  \
         claude-focus            Run as a Claude Code Notification hook (reads JSON on stdin)\n  \
         claude-focus test [t]   Fire synthetic notification(s) through the real focus+notify path\n  \
         claude-focus doctor     Check deps, config, extension, and hook registration\n  \
         claude-focus --version  Print version",
        env!("CARGO_PKG_VERSION")
    );
}

const ALL_TYPES: [&str; 4] = [
    "permission_prompt",
    "idle_prompt",
    "elicitation_dialog",
    "auth_success",
];

/// Build the (type, in_allowlist) list `test` will fire. Pure, for testing.
fn test_plan(which: Option<&str>, notify_types: &[String]) -> Vec<(String, bool)> {
    let types: Vec<&str> = match which {
        Some(t) => vec![t],
        None => ALL_TYPES.to_vec(),
    };
    types
        .into_iter()
        .map(|t| (t.to_string(), notify_types.iter().any(|x| x == t)))
        .collect()
}

/// Diagnostic: fire synthetic notification(s) through the REAL focus+notify
/// code, intentionally BYPASSING both the notify_types allowlist AND the config
/// `mode` gate (`force = true`) — `test` always exercises focus AND notify so
/// you can confirm each leg. Highlights the invoking terminal (the /proc walk
/// starts from this process).
fn run_test(which: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let config = config::load_config();
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let plan = test_plan(which, &config.notify_types);
    for (ty, in_allowlist) in plan {
        let note = if in_allowlist {
            ""
        } else {
            "  (not in your notify_types — forcing anyway)"
        };
        println!("→ firing {ty}{note}");
        dispatch(&ty, "", cwd.as_deref(), &config, true);
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
    Ok(())
}

fn run_doctor() -> Result<(), Box<dyn std::error::Error>> {
    doctor::run()
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

    dispatch(
        notification_type,
        message,
        hook_input.cwd.as_deref(),
        &config,
        false,
    );

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
fn dispatch(
    notification_type: &str,
    message: &str,
    cwd: Option<&str>,
    config: &config::Config,
    force: bool,
) {
    let should_focus = force || config.mode == Mode::Both || config.mode == Mode::FocusOnly;
    let should_notify = force || config.mode == Mode::Both || config.mode == Mode::NotifyOnly;

    if should_focus {
        if let Some(pid) = process_tree::find_terminal_pid() {
            dbus::highlight_window(pid, config.notification_timeout_ms);
        }
    }
    if should_notify {
        notify::send_notification(notification_type, message, cwd, config, force);
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

    #[test]
    fn no_args_is_hook() {
        assert!(matches!(parse_args(&[]), Command::Hook));
    }

    #[test]
    fn version_flag_parses() {
        assert!(matches!(
            parse_args(&["--version".to_string()]),
            Command::Version
        ));
        assert!(matches!(parse_args(&["-V".to_string()]), Command::Version));
    }

    #[test]
    fn doctor_parses() {
        assert!(matches!(
            parse_args(&["doctor".to_string()]),
            Command::Doctor
        ));
    }

    #[test]
    fn test_with_type_parses() {
        match parse_args(&["test".to_string(), "idle_prompt".to_string()]) {
            Command::Test(Some(t)) => assert_eq!(t, "idle_prompt"),
            other => panic!("expected Test(Some), got {other:?}"),
        }
    }

    #[test]
    fn test_without_type_parses() {
        assert!(matches!(
            parse_args(&["test".to_string()]),
            Command::Test(None)
        ));
    }

    #[test]
    fn test_plan_no_arg_covers_all_four_with_allowlist_flags() {
        let notify = vec!["permission_prompt".to_string(), "idle_prompt".to_string()];
        let plan = test_plan(None, &notify);
        assert_eq!(plan.len(), 4);
        // auth_success is absent from the allowlist -> flagged false (forced anyway).
        let auth = plan.iter().find(|(t, _)| t == "auth_success").unwrap();
        assert!(!auth.1);
        let perm = plan.iter().find(|(t, _)| t == "permission_prompt").unwrap();
        assert!(perm.1);
    }

    #[test]
    fn test_plan_single_type() {
        let plan = test_plan(Some("idle_prompt"), &[]);
        assert_eq!(plan, vec![("idle_prompt".to_string(), false)]);
    }
}
