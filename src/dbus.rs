use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

/// Call the GNOME Shell extension to highlight a window by PID.
/// Fire-and-forget — spawns gdbus without waiting for it to finish.
pub fn highlight_window(pid: u32, duration_ms: u32) {
    let _ = Command::new("gdbus")
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .args([
            "call",
            "--session",
            "--dest",
            "org.gnome.Shell.Extensions.FocusByPid",
            "--object-path",
            "/org/gnome/Shell/Extensions/FocusByPid",
            "--method",
            "org.gnome.Shell.Extensions.FocusByPid.HighlightByPid",
            &pid.to_string(),
            &duration_ms.to_string(),
        ])
        .spawn();
}
