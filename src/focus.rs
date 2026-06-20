//! The focus abstraction: a backend-agnostic way to raise the window a Claude
//! session lives in. Phase 4 ships one real backend (the GNOME Shell extension);
//! Phase 5 adds X11 behind the same trait. Everything here that is pure is unit
//! tested; the subprocess-touching backend lives in `dbus.rs`.

/// What `Focuser::focus` is asked to act on. `pid` is best-effort (gnome-terminal
/// shares one server PID across windows, so it is only a fallback); `session_marker`
/// is the precise per-window key embedded in the title by the SessionStart hook.
pub struct FocusTarget {
    pub pid: Option<u32>,
    pub session_marker: Option<String>,
    pub duration_ms: u32,
}

/// The result of a focus attempt. Drives 4.3b notify-suppression.
#[derive(Debug, PartialEq, Eq)]
pub enum FocusOutcome {
    Raised,         // a window was found and raised/highlighted
    AlreadyFocused, // a window was found but was already focused (4.3a no-op'd it)
    NotFound,       // backend ran but matched no window
    Unavailable,    // no usable backend, or the call failed/timed out
}

pub trait Focuser {
    /// Synchronous, bounded. Returns the outcome so the caller can gate notify.
    fn focus(&self, target: &FocusTarget) -> FocusOutcome;
    /// Fire-and-forget: request focus without waiting for a result. Used in
    /// focus-only mode, where the outcome is not needed, to keep the fast path.
    fn focus_detached(&self, target: &FocusTarget);
}

/// Derive the per-window marker from a Claude `session_id`: the first 8 chars
/// wrapped as `[cf:<id8>]`. `None` for an empty id (then matching uses the PID
/// fallback). The bracketed form is what both the title and the matcher use, so
/// substring matching can't collide with bare hex elsewhere in a title.
pub fn session_marker(session_id: &str) -> Option<String> {
    let id = session_id.trim();
    if id.is_empty() {
        return None;
    }
    let short: String = id.chars().take(8).collect();
    Some(format!("[cf:{short}]"))
}

/// Parse gdbus's textual reply for `HighlightBySession`, e.g. `"(true, false)"`,
/// into a `FocusOutcome`. Anything unexpected → `Unavailable` (never panics,
/// never false-suppresses).
pub fn parse_focus_outcome(stdout: &str) -> FocusOutcome {
    let inner = stdout.trim().trim_start_matches('(').trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
    match (parts.first().copied(), parts.get(1).copied()) {
        (Some("true"), Some("true")) => FocusOutcome::AlreadyFocused,
        (Some("true"), Some("false")) => FocusOutcome::Raised,
        (Some("false"), Some(_)) => FocusOutcome::NotFound,
        _ => FocusOutcome::Unavailable,
    }
}

/// Whether the GNOME Shell extension backend should be used: GNOME desktop (from
/// `XDG_CURRENT_DESKTOP`) AND `gdbus` present. GNOME-on-X11 works too (the
/// extension is compositor-agnostic), so we gate on the desktop, not the session
/// type. Non-GNOME or no gdbus → `NoopFocuser`, so the binary never even attempts
/// a gdbus call into the void (spec 4.1). Mirrors install.sh's GNOME check.
pub fn should_use_gnome_shell(current_desktop: Option<&str>, gdbus_present: bool) -> bool {
    gdbus_present
        && current_desktop
            .map(|d| d.to_ascii_lowercase().contains("gnome"))
            .unwrap_or(false)
}

/// 4.3b: suppress the banner+sound when the target window is already focused —
/// but never when `force` (the `test` subcommand), which must always alert.
pub fn focus_suppresses_notify(outcome: &FocusOutcome, force: bool) -> bool {
    !force && *outcome == FocusOutcome::AlreadyFocused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_takes_first_eight_chars() {
        assert_eq!(
            session_marker("50613d2b-7490-497a-965c-6992e1bc7d45").as_deref(),
            Some("[cf:50613d2b]")
        );
    }

    #[test]
    fn marker_none_for_empty_or_blank() {
        assert_eq!(session_marker(""), None);
        assert_eq!(session_marker("   "), None);
    }

    #[test]
    fn marker_handles_short_ids() {
        assert_eq!(session_marker("abc").as_deref(), Some("[cf:abc]"));
    }

    #[test]
    fn parses_gdbus_tuple_into_outcome() {
        assert_eq!(parse_focus_outcome("(true, false)\n"), FocusOutcome::Raised);
        assert_eq!(
            parse_focus_outcome("(true, true)\n"),
            FocusOutcome::AlreadyFocused
        );
        assert_eq!(
            parse_focus_outcome("(false, false)\n"),
            FocusOutcome::NotFound
        );
    }

    #[test]
    fn malformed_gdbus_output_is_unavailable() {
        assert_eq!(parse_focus_outcome(""), FocusOutcome::Unavailable);
        assert_eq!(parse_focus_outcome("nonsense"), FocusOutcome::Unavailable);
        assert_eq!(parse_focus_outcome("(true)"), FocusOutcome::Unavailable);
    }

    #[test]
    fn gnome_desktop_with_gdbus_selects_gnome_backend() {
        assert!(should_use_gnome_shell(Some("ubuntu:GNOME"), true));
        assert!(should_use_gnome_shell(Some("GNOME"), true));
    }

    #[test]
    fn no_gdbus_never_selects_gnome_backend() {
        // Spec 4.1: with gdbus absent, do NOT attempt a gdbus call.
        assert!(!should_use_gnome_shell(Some("ubuntu:GNOME"), false));
    }

    #[test]
    fn non_gnome_or_absent_desktop_does_not() {
        assert!(!should_use_gnome_shell(Some("KDE"), true));
        assert!(!should_use_gnome_shell(Some("XFCE"), true));
        assert!(!should_use_gnome_shell(None, true));
        assert!(!should_use_gnome_shell(Some(""), true));
    }

    #[test]
    fn already_focused_suppresses_only_when_not_forced() {
        assert!(focus_suppresses_notify(
            &FocusOutcome::AlreadyFocused,
            false
        ));
        assert!(!focus_suppresses_notify(
            &FocusOutcome::AlreadyFocused,
            true
        )); // test/force
    }

    #[test]
    fn other_outcomes_never_suppress() {
        for o in [
            FocusOutcome::Raised,
            FocusOutcome::NotFound,
            FocusOutcome::Unavailable,
        ] {
            assert!(!focus_suppresses_notify(&o, false));
        }
    }
}
