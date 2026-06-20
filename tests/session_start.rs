//! Integration tests for the SessionStart title-tagging path and the
//! always-exit-0 hook contract. Runs the real built binary the way Claude Code
//! does: pipe a JSON payload on stdin. Hermetic: HOME/XDG point at a throwaway
//! dir and PATH is emptied so no real focus/notify subprocess can fire.

use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_claude-focus");

/// Run the binary in hook mode with `payload` on stdin; return (exit_ok, stdout).
fn run_hook(payload: &str) -> (bool, String) {
    let home = std::env::temp_dir().join(format!("cf-ss-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("create temp HOME");

    let mut child = Command::new(BIN)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn claude-focus");

    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");

    let out = child.wait_with_output().expect("wait for child");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn session_start_emits_terminal_sequence_and_exits_zero() {
    let payload = r#"{"session_id":"50613d2b-7490-497a-965c-6992e1bc7d45","cwd":"/home/u/git_repos/claude-focus","hook_event_name":"SessionStart","source":"startup"}"#;
    let (ok, stdout) = run_hook(payload);
    assert!(ok, "SessionStart must exit 0");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout is JSON");
    assert_eq!(
        v["terminalSequence"].as_str().unwrap(),
        "\u{1b}]2;claude · claude-focus [cf:50613d2b]\u{7}"
    );
}

#[test]
fn session_start_without_session_id_exits_zero_and_is_silent() {
    let payload = r#"{"cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}"#;
    let (ok, stdout) = run_hook(payload);
    assert!(ok);
    assert!(
        stdout.trim().is_empty(),
        "no session id -> no terminalSequence"
    );
}

#[test]
fn notification_payload_still_exits_zero() {
    let payload = r#"{"session_id":"s","cwd":"/tmp","hook_event_name":"Notification","notification_type":"permission_prompt","message":"hi"}"#;
    let (ok, _stdout) = run_hook(payload);
    assert!(ok);
}

#[test]
fn malformed_stdin_still_exits_zero() {
    // Garbage input must never block Claude Code (design ethos #3).
    let (ok, _stdout) = run_hook("{ this is not json");
    assert!(ok);
}

#[test]
fn empty_stdin_still_exits_zero() {
    let (ok, _stdout) = run_hook("");
    assert!(ok);
}
