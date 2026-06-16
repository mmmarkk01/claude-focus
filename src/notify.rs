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

/// Parse "HH:MM" into minutes-since-midnight (0..=1439). None if malformed.
fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// True if `now_min` (minutes since midnight) is inside the quiet window
/// `"HH:MM-HH:MM"`. Supports windows that wrap past midnight (e.g.
/// "22:00-08:00"). End is exclusive. A malformed/None window is "no quiet
/// hours" (false) — never silently swallow a bad value into "always quiet".
fn in_quiet_hours(window: Option<&str>, now_min: u32) -> bool {
    let Some(window) = window else {
        return false;
    };
    let Some((start, end)) = window.split_once('-') else {
        return false;
    };
    let (Some(start), Some(end)) = (parse_hm(start), parse_hm(end)) else {
        return false;
    };
    if start <= end {
        now_min >= start && now_min < end
    } else {
        now_min >= start || now_min < end
    }
}

/// GNOME "show banners" == false ⇒ Do Not Disturb on. Best-effort: any failure
/// (no gsettings, non-GNOME, parse miss) ⇒ false, i.e. notify normally. Never
/// false-suppress.
fn gnome_dnd_active() -> bool {
    Command::new("gsettings")
        .args(["get", "org.gnome.desktop.notifications", "show-banners"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
        .unwrap_or(false)
}

/// Local wall-clock minutes-since-midnight via `date` (a standard tool — avoids
/// pulling in a timezone crate, consistent with the zero-runtime-deps ethos).
fn now_minutes() -> Option<u32> {
    let out = Command::new("date").arg("+%H:%M").output().ok()?;
    parse_hm(String::from_utf8_lossy(&out.stdout).trim())
}

/// Whether the *noisy* legs (banner + sound) should be suppressed right now:
/// GNOME DND on, OR inside the configured quiet-hours window. Focus is NOT
/// gated by this.
fn notifications_silenced(config: &Config) -> bool {
    if gnome_dnd_active() {
        return true;
    }
    match (config.quiet_hours.as_deref(), now_minutes()) {
        (Some(window), Some(now)) => in_quiet_hours(Some(window), now),
        _ => false,
    }
}

pub fn send_notification(
    notification_type: &str,
    message: &str,
    cwd: Option<&str>,
    config: &Config,
    force: bool,
) {
    // Respect DND / quiet hours for the noisy legs. `test` (force) bypasses so
    // diagnostics always show a banner. Auto-focus is unaffected (it lives in
    // main::dispatch).
    if !force && notifications_silenced(config) {
        return;
    }

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

    #[test]
    fn parse_hm_basic() {
        assert_eq!(parse_hm("00:00"), Some(0));
        assert_eq!(parse_hm("09:30"), Some(570));
        assert_eq!(parse_hm("23:59"), Some(1439));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("12:60"), None);
        assert_eq!(parse_hm("bad"), None);
        assert_eq!(parse_hm("12"), None);
    }

    #[test]
    fn quiet_hours_none_is_never_quiet() {
        assert!(!in_quiet_hours(None, 0));
        assert!(!in_quiet_hours(None, 720));
    }

    #[test]
    fn quiet_hours_simple_window() {
        // 09:00-17:00 -> minutes 540..1020 (end exclusive)
        assert!(!in_quiet_hours(Some("09:00-17:00"), 539));
        assert!(in_quiet_hours(Some("09:00-17:00"), 540));
        assert!(in_quiet_hours(Some("09:00-17:00"), 1019));
        assert!(!in_quiet_hours(Some("09:00-17:00"), 1020));
    }

    #[test]
    fn quiet_hours_wraps_midnight() {
        // 22:00-08:00 -> >=1320 OR <480
        assert!(in_quiet_hours(Some("22:00-08:00"), 1320)); // 22:00
        assert!(in_quiet_hours(Some("22:00-08:00"), 0)); // 00:00
        assert!(in_quiet_hours(Some("22:00-08:00"), 479)); // 07:59
        assert!(!in_quiet_hours(Some("22:00-08:00"), 480)); // 08:00
        assert!(!in_quiet_hours(Some("22:00-08:00"), 720)); // noon
    }

    #[test]
    fn malformed_quiet_hours_is_not_quiet() {
        assert!(!in_quiet_hours(Some("nonsense"), 720));
        assert!(!in_quiet_hours(Some("25:00-26:00"), 720));
        assert!(!in_quiet_hours(Some("22:00"), 720)); // no dash
    }
}
