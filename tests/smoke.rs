//! End-to-end smoke tests for the core "never block Claude Code — always exit 0"
//! contract (design ethos #3). Runs the real built binary the way Claude Code's
//! Notification hook does: pipe a JSON payload on stdin and assert a clean exit,
//! even for malformed or empty input.
//!
//! Hermetic by construction: HOME/XDG point at a throwaway dir (so config is
//! deterministic defaults, not the developer's real config) and PATH is emptied
//! so the fire-and-forget focus/notify spawns (notify-send, gdbus, gsettings,
//! pw-play) resolve to nothing. The dispatch path is still exercised, but no
//! desktop banner or window-raise can fire during `cargo test`.

use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_claude-focus");

/// Run the binary in hook mode with `payload` on stdin; return whether it
/// exited 0.
fn exits_zero(payload: &str) -> bool {
    // A throwaway HOME so config resolution finds nothing and falls back to
    // defaults (deterministic, independent of the developer's real config). It
    // is only ever READ here — the hook never writes under HOME — so the tests
    // sharing this dir within one test process is safe. Scoped by PID so a
    // concurrent `cargo test` run can't collide.
    let home = std::env::temp_dir().join(format!("claude-focus-smoke-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("create temp HOME");

    let mut child = Command::new(BIN)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn claude-focus");

    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(payload.as_bytes())
        .expect("write payload"); // ChildStdin drops here, closing the pipe

    child.wait().expect("wait for child").success()
}

#[test]
fn valid_permission_prompt_exits_zero() {
    let payload = r#"{"session_id":"s","cwd":"/tmp","hook_event_name":"Notification","notification_type":"permission_prompt","message":"needs you"}"#;
    assert!(exits_zero(payload));
}

#[test]
fn malformed_json_still_exits_zero() {
    // Garbage input must never block Claude Code — still a clean exit.
    assert!(exits_zero("{ this is not json"));
}

#[test]
fn empty_stdin_still_exits_zero() {
    // Empty stdin fails JSON parsing and prints to stderr (silenced here), but
    // must still exit 0 — the hook never blocks Claude Code.
    assert!(exits_zero(""));
}
