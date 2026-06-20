/// Whether `expected_command` is registered under `hooks.<event>` in
/// settings.json (EXACT command match, matcher-agnostic). Err if the file is
/// present but unparseable — doctor reports that as a FAIL leg, never panics.
pub fn hook_registered_for(
    settings_json: &str,
    expected_command: &str,
    event: &str,
) -> Result<bool, String> {
    let v: serde_json::Value = serde_json::from_str(settings_json)
        .map_err(|e| format!("settings.json is not valid JSON: {e}"))?;
    let found = v
        .get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|n| n.as_array())
        .map(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hooks| {
                        hooks.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .map(|c| c == expected_command)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    Ok(found)
}

/// Back-compat: the Notification hook check.
pub fn hook_registered(settings_json: &str, expected_command: &str) -> Result<bool, String> {
    hook_registered_for(settings_json, expected_command, "Notification")
}

/// Whether `env.CLAUDE_CODE_DISABLE_TERMINAL_TITLE` == "1" in settings.json.
pub fn disable_title_env_set(settings_json: &str) -> Result<bool, String> {
    let v: serde_json::Value = serde_json::from_str(settings_json)
        .map_err(|e| format!("settings.json is not valid JSON: {e}"))?;
    Ok(v.get("env")
        .and_then(|e| e.get("CLAUDE_CODE_DISABLE_TERMINAL_TITLE"))
        .and_then(|x| x.as_str())
        .map(|s| s == "1")
        .unwrap_or(false))
}

use std::process::{Command, Stdio};

fn on_path(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn dbus_name_owned() -> bool {
    Command::new("gdbus")
        .args([
            "introspect",
            "--session",
            "--dest",
            "org.gnome.Shell.Extensions.FocusByPid",
            "--object-path",
            "/org/gnome/Shell/Extensions/FocusByPid",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn extension_enabled() -> bool {
    Command::new("gnome-extensions")
        .args(["list", "--enabled"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("focus-by-pid@claude.local"))
        .unwrap_or(false)
}

fn line(ok: bool, label: &str, fix: &str) {
    if ok {
        println!("  [PASS] {label}");
    } else {
        println!("  [FAIL] {label}\n         fix: {fix}");
    }
}

fn warn_line(ok: bool, label: &str, note: &str) {
    if ok {
        println!("  [PASS] {label}");
    } else {
        println!("  [WARN] {label}\n         {note}");
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("claude-focus doctor\n");

    // Config parses (tolerant — prints the exact error, never panics). Keep the
    // effective Config to reuse for the sound-file leg.
    let cfg_path = crate::config::public_config_path();
    let effective: crate::config::Config = match std::fs::read_to_string(&cfg_path) {
        Ok(c) => match toml::from_str(&c) {
            Ok(cfg) => {
                line(true, "config parses", "");
                cfg
            }
            Err(e) => {
                line(false, "config parses", &format!("fix the TOML error: {e}"));
                crate::config::Config::default()
            }
        },
        Err(_) => {
            line(true, "config parses (none yet — defaults in use)", "");
            crate::config::Config::default()
        }
    };

    // Sound file exists when play_sound is on (read-only — does not touch the
    // notify.rs emission path, which Phase 2.7 owns).
    if effective.play_sound {
        match effective.sound_file {
            Some(ref sf) => line(
                std::path::Path::new(sf).exists(),
                "sound file exists",
                "set sound_file to a real path or play_sound=false",
            ),
            None => line(
                false,
                "sound file set",
                "set sound_file or play_sound=false",
            ),
        }
    }

    // Dependencies on PATH.
    let gdbus = on_path("gdbus");
    line(
        on_path("notify-send"),
        "notify-send on PATH",
        "install libnotify-bin",
    );
    line(
        on_path("pw-play"),
        "pw-play on PATH",
        "install pipewire-bin (or set play_sound=false)",
    );
    line(gdbus, "gdbus on PATH", "install libglib2.0-bin");

    // D-Bus service owned (depends on gdbus; report once).
    if gdbus {
        line(
            dbus_name_owned(),
            "FocusByPid D-Bus service owned",
            "log out/in or restart GNOME Shell to load the extension",
        );
    }

    // Extension enabled.
    line(
        extension_enabled(),
        "extension enabled",
        "gnome-extensions enable focus-by-pid@claude.local",
    );

    // Hook registered (exact match to install.sh; tolerant of malformed JSON).
    let home = std::env::var("HOME").unwrap_or_default();
    let expected_command = format!("{home}/.local/bin/claude-focus");
    let settings = format!("{home}/.claude/settings.json");
    match std::fs::read_to_string(&settings) {
        Ok(s) => {
            match hook_registered(&s, &expected_command) {
                Ok(true) => line(true, "Notification hook registered", ""),
                Ok(false) => line(
                    false,
                    "Notification hook registered",
                    "re-run scripts/install.sh",
                ),
                Err(e) => line(false, "settings.json parses", &e),
            }
            match hook_registered_for(&s, &expected_command, "SessionStart") {
                Ok(true) => line(true, "SessionStart hook registered (title tagging)", ""),
                Ok(false) => line(
                    false,
                    "SessionStart hook registered (title tagging)",
                    "re-run scripts/install.sh to enable precise window matching",
                ),
                Err(_) => {} // already reported by the Notification parse above
            }
            match disable_title_env_set(&s) {
                Ok(true) => warn_line(true, "CLAUDE_CODE_DISABLE_TERMINAL_TITLE set", ""),
                Ok(false) => warn_line(
                    false,
                    "CLAUDE_CODE_DISABLE_TERMINAL_TITLE set",
                    "optional: set env.CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1 for precise window matching (PID fallback works without it)",
                ),
                Err(_) => {}
            }
        }
        Err(_) => line(
            false,
            "Notification hook registered",
            "run scripts/install.sh",
        ),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: &str = "/home/u/.local/bin/claude-focus";
    const WITH_HOOK: &str = r#"{"hooks":{"Notification":[{"matcher":"*","hooks":[{"type":"command","command":"/home/u/.local/bin/claude-focus"}]}]}}"#;
    const WRONG_PATH: &str = r#"{"hooks":{"Notification":[{"matcher":"*","hooks":[{"type":"command","command":"/usr/bin/not-claude-focus"}]}]}}"#;
    const WITH_SS: &str = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/home/u/.local/bin/claude-focus"}]}]}}"#;

    #[test]
    fn detects_registered_hook() {
        assert_eq!(hook_registered(WITH_HOOK, EXPECTED), Ok(true));
    }

    #[test]
    fn rejects_a_different_command_path() {
        // ends_with("claude-focus") would wrongly PASS this; exact match rejects it.
        assert_eq!(hook_registered(WRONG_PATH, EXPECTED), Ok(false));
    }

    #[test]
    fn malformed_json_is_err_not_panic() {
        assert!(hook_registered("{not json", EXPECTED).is_err());
    }

    #[test]
    fn detects_session_start_hook() {
        assert_eq!(
            hook_registered_for(WITH_SS, EXPECTED, "SessionStart"),
            Ok(true)
        );
    }

    #[test]
    fn session_start_absent_when_only_notification() {
        assert_eq!(
            hook_registered_for(WITH_HOOK, EXPECTED, "SessionStart"),
            Ok(false)
        );
    }

    #[test]
    fn disable_title_env_detected() {
        assert_eq!(
            disable_title_env_set(r#"{"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":"1"}}"#),
            Ok(true)
        );
    }

    #[test]
    fn disable_title_env_absent() {
        assert_eq!(disable_title_env_set("{}"), Ok(false));
        assert_eq!(
            disable_title_env_set(r#"{"env":{"CLAUDE_CODE_DISABLE_TERMINAL_TITLE":"0"}}"#),
            Ok(false)
        );
    }
}
