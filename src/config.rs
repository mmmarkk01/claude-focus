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

pub fn load_config() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
    PathBuf::from(home)
        .join(".config")
        .join("claude-focus")
        .join("config.toml")
}
