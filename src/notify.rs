use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::config::Config;

pub fn send_notification(
    notification_type: &str,
    message: &str,
    config: &Config,
) {
    let title = match notification_type {
        "permission_prompt" => "Claude Code — Permission Required",
        "idle_prompt" => "Claude Code — Ready for Input",
        "elicitation_dialog" => "Claude Code — Question",
        "auth_success" => "Claude Code — Authenticated",
        _ => "Claude Code",
    };

    let body = if message.is_empty() {
        match notification_type {
            "permission_prompt" => "Claude Code needs your permission to continue.",
            "idle_prompt" => "Claude Code is waiting for your input.",
            "elicitation_dialog" => "Claude Code has a question for you.",
            "auth_success" => "Authentication successful.",
            _ => "Claude Code needs attention.",
        }
    } else {
        message
    };

    let timeout_ms = config.notification_timeout_ms.to_string();

    let mut args = vec![
        "--urgency", "normal",
        "--expire-time", &timeout_ms,
        "--app-name", "Claude Code",
    ];

    let sound_hint;
    if config.play_sound {
        if let Some(ref sound_file) = config.sound_file {
            sound_hint = format!("string:sound-file:{sound_file}");
            args.extend_from_slice(&["--hint", &sound_hint]);
        }
    }

    args.push(title);
    args.push(body);

    let _ = Command::new("notify-send")
        .args(&args)
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    if config.play_sound {
        if let Some(ref sound_file) = config.sound_file {
            let _ = Command::new("pw-play")
                .arg(sound_file)
                .process_group(0)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}
