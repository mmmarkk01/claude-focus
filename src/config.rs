use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Both,
    FocusOnly,
    NotifyOnly,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Both
    }
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub mode: Mode,

    #[serde(default = "default_notify_types")]
    pub notify_types: Vec<String>,

    #[serde(default = "default_timeout")]
    pub notification_timeout_ms: u32,

    #[serde(default)]
    pub play_sound: bool,

    #[serde(default = "default_sound_file")]
    pub sound_file: Option<String>,
}

fn default_notify_types() -> Vec<String> {
    vec![
        "permission_prompt".into(),
        "idle_prompt".into(),
        "elicitation_dialog".into(),
    ]
}

fn default_timeout() -> u32 {
    5000
}

fn default_sound_file() -> Option<String> {
    Some("/usr/share/sounds/freedesktop/stereo/bell.oga".into())
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mode: Mode::default(),
            notify_types: default_notify_types(),
            notification_timeout_ms: default_timeout(),
            play_sound: false,
            sound_file: default_sound_file(),
        }
    }
}

/// Parse config from a TOML string. On a parse error, print the error (which
/// carries line/column) to stderr and fall back to defaults — never panic,
/// never silently discard config without a signal.
pub fn parse_config_or_default(contents: &str) -> Config {
    match toml::from_str(contents) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "claude-focus: invalid config at {}, using defaults: {e}",
                config_path().display()
            );
            Config::default()
        }
    }
}

pub fn load_config() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => parse_config_or_default(&contents),
        Err(_) => Config::default(),
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    config_path_from(&home, xdg.as_deref())
}

/// Resolve the config path. XDG_CONFIG_HOME wins only when set AND non-empty
/// (the XDG spec treats empty as unset; Rust's env::var returns Ok("") for an
/// empty var, so we must guard explicitly). Otherwise fall back to $HOME/.config.
fn config_path_from(home: &str, xdg: Option<&str>) -> PathBuf {
    let base = match xdg {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => PathBuf::from(home).join(".config"),
    };
    base.join("claude-focus").join("config.toml")
}

/// Public accessor for the resolved config path (used by `doctor`).
pub fn public_config_path() -> PathBuf {
    config_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_config_parses() {
        let cfg = parse_config_or_default("mode = \"notify-only\"\n");
        assert_eq!(cfg.mode, Mode::NotifyOnly);
    }

    #[test]
    fn invalid_config_falls_back_to_defaults() {
        // A broken table header must yield defaults, never a panic.
        let cfg = parse_config_or_default("mode = \"notify-only\"\n[ broken");
        assert_eq!(cfg.mode, Mode::default()); // Mode::Both
    }

    #[test]
    fn xdg_set_nonempty_wins() {
        assert_eq!(
            config_path_from("/home/u", Some("/cfg")),
            std::path::PathBuf::from("/cfg/claude-focus/config.toml")
        );
    }

    #[test]
    fn xdg_empty_falls_back_to_home() {
        // Per the XDG spec, an empty value means "unset".
        assert_eq!(
            config_path_from("/home/u", Some("")),
            std::path::PathBuf::from("/home/u/.config/claude-focus/config.toml")
        );
    }

    #[test]
    fn xdg_unset_falls_back_to_home() {
        assert_eq!(
            config_path_from("/home/u", None),
            std::path::PathBuf::from("/home/u/.config/claude-focus/config.toml")
        );
    }
}
