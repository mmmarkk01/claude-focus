use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::config::Config;

/// notify-send urgency. `critical` cuts through and persists (GNOME ignores the
/// expire-time for it), used for permission prompts; everything else is `normal`.
fn urgency_for(notification_type: &str) -> &'static str {
    match notification_type {
        "permission_prompt" => "critical",
        _ => "normal",
    }
}

/// Final path component of `cwd` (the project dir), or None when empty / root.
fn project_basename(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
}

/// Append the project name to the title so concurrent sessions are
/// distinguishable, e.g. "Claude Code — Permission Required · claude-focus".
/// A None / empty / root cwd yields the base title unchanged.
fn title_with_project(base: &str, cwd: Option<&str>) -> String {
    match cwd.and_then(project_basename) {
        Some(name) => format!("{base} · {name}"),
        None => base.to_string(),
    }
}

pub fn send_notification(
    notification_type: &str,
    message: &str,
    cwd: Option<&str>,
    config: &Config,
) {
    let base_title = match notification_type {
        "permission_prompt" => "Claude Code — Permission Required",
        "idle_prompt" => "Claude Code — Ready for Input",
        "elicitation_dialog" => "Claude Code — Question",
        "auth_success" => "Claude Code — Authenticated",
        _ => "Claude Code",
    };
    let title = title_with_project(base_title, cwd);

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

    let urgency = urgency_for(notification_type);
    let mut args = vec![
        "--urgency", urgency,
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

    args.push(&title);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_is_critical() {
        assert_eq!(urgency_for("permission_prompt"), "critical");
    }

    #[test]
    fn other_types_are_normal() {
        assert_eq!(urgency_for("idle_prompt"), "normal");
        assert_eq!(urgency_for("elicitation_dialog"), "normal");
        assert_eq!(urgency_for(""), "normal");
    }

    #[test]
    fn title_includes_project_basename() {
        assert_eq!(
            title_with_project(
                "Claude Code — Permission Required",
                Some("/home/u/git_repos/claude-focus")
            ),
            "Claude Code — Permission Required · claude-focus"
        );
    }

    #[test]
    fn title_unchanged_without_cwd() {
        assert_eq!(title_with_project("Claude Code", None), "Claude Code");
        assert_eq!(title_with_project("Claude Code", Some("")), "Claude Code");
        assert_eq!(title_with_project("Claude Code", Some("/")), "Claude Code");
    }
}
