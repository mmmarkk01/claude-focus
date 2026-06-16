/// Whether the claude-focus hook command is registered in settings.json.
/// Matches `expected_command` by EXACT equality (the same check install.sh uses
/// at lines 69-72). Returns Err with a message if the file is present but
/// unparseable — doctor must report that as a FAIL leg, never panic (Phase 2.1
/// hardens the writer; doctor must be robust before that lands).
pub fn hook_registered(settings_json: &str, expected_command: &str) -> Result<bool, String> {
    let v: serde_json::Value = serde_json::from_str(settings_json)
        .map_err(|e| format!("settings.json is not valid JSON: {e}"))?;
    let found = v
        .get("hooks")
        .and_then(|h| h.get("Notification"))
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
        Ok(s) => match hook_registered(&s, &expected_command) {
            Ok(true) => line(true, "hook registered in settings.json", ""),
            Ok(false) => line(
                false,
                "hook registered in settings.json",
                "re-run scripts/install.sh",
            ),
            Err(e) => line(false, "settings.json parses", &e),
        },
        Err(_) => line(
            false,
            "hook registered in settings.json",
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
}
