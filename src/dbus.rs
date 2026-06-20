use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::focus::{parse_focus_outcome, FocusOutcome, FocusTarget, Focuser};

const DEST: &str = "org.gnome.Shell.Extensions.FocusByPid";
const OBJECT_PATH: &str = "/org/gnome/Shell/Extensions/FocusByPid";
const METHOD: &str = "org.gnome.Shell.Extensions.FocusByPid.HighlightBySession";

/// Focus backend that drives the GNOME Shell extension over D-Bus (`gdbus`).
/// Works on GNOME under both Wayland and X11.
pub struct GnomeShellFocuser;

impl GnomeShellFocuser {
    /// The full gdbus argv for `HighlightBySession(marker, pid, duration_ms)`.
    /// Returned as `Vec<String>` so both call sites share one builder and there
    /// is no `&str`/`&String` array-type mismatch (`Vec<String>` satisfies
    /// `Command::args`' `IntoIterator<Item: AsRef<OsStr>>`).
    fn call_args(target: &FocusTarget) -> Vec<String> {
        vec![
            "call".into(),
            "--session".into(),
            "--timeout".into(),
            "1".into(), // `--timeout 1` bounds the D-Bus wait (whole seconds)
            "--dest".into(),
            DEST.into(),
            "--object-path".into(),
            OBJECT_PATH.into(),
            "--method".into(),
            METHOD.into(),
            target.session_marker.clone().unwrap_or_default(),
            target.pid.unwrap_or(0).to_string(),
            target.duration_ms.to_string(),
        ]
    }
}

impl Focuser for GnomeShellFocuser {
    fn focus(&self, target: &FocusTarget) -> FocusOutcome {
        // Synchronous, bounded: `.output()` waits, `--timeout 1` caps the wait.
        let out = Command::new("gdbus")
            .args(Self::call_args(target))
            .stderr(Stdio::null())
            .output();
        match out {
            Ok(o) if o.status.success() => parse_focus_outcome(&String::from_utf8_lossy(&o.stdout)),
            _ => FocusOutcome::Unavailable,
        }
    }

    fn focus_detached(&self, target: &FocusTarget) {
        let _ = Command::new("gdbus")
            .args(Self::call_args(target))
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}
