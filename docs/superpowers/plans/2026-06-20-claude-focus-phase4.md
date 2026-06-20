# claude-focus Phase 4 — Right window, every time (implementation plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Focus the *correct* terminal window across concurrent Claude sessions, skip redundant alerts when the window is already focused, and put the focus path behind a `Focuser` trait — all degrading to today's behavior when the new mechanism isn't available.

**Architecture:** A new `SessionStart` hook tags each terminal's title with the Claude session id (`claude · <project> [cf:<id8>]`) via Claude Code's `terminalSequence` output, kept stable by an opt-in `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`. The `Notification` hook builds a `FocusTarget { pid, session_marker }` and calls a `Focuser` backend; the GNOME Shell extension gains `HighlightBySession(marker, pid, duration) → (found, already_focused)` that matches the title marker, falls back to PID + `get_user_time`, no-ops when already focused (4.3a), and reports focus state so the hook can suppress the banner when you're already there (4.3b).

**Tech Stack:** Rust (std only + serde/serde_json/toml already in tree), GJS GNOME Shell extension (GNOME 45–48; machine is 46), bash + python3 installer.

**Spec:** `docs/superpowers/specs/2026-06-20-claude-focus-phase4-design.md`

---

## Verified facts this plan relies on (do not re-litigate)

- **`terminalSequence`** is a **top-level** hook JSON-output field (`{"terminalSequence": "..."}`), allows OSC 0/1/2, requires Claude Code ≥ 2.1.141 (machine: 2.1.181). Source: code.claude.com/docs/en/hooks.
- **`CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`** disables Claude's own dynamic title; settable in `settings.json` `env`.
- **SessionStart** hooks: receive `session_id`, `cwd`, `source`; honor stdout JSON incl. top-level `terminalSequence`; default command-hook timeout is 600 s (a ~ms–1 s sync subprocess is fine). `session_id` == `CLAUDE_CODE_SESSION_ID`; UUID's first group is 8 hex chars.
- **`gdbus call --timeout SECS`** exists (whole seconds; we cap at `1`). Multi-value return prints as `(true, false)`.
- **GNOME Shell APIs (45–48, verified):** `global.display.get_focus_window()` (Meta.Window | null), `Meta.Window.get_title()/get_pid()/get_user_time()`, `global.get_window_actors()` + `actor.get_meta_window()` (guard null), `Main.activateWindow(win)`. A D-Bus method with two `out` args must **return a JS array** `[found, already_focused]` in declared order.
- **Baseline on `dx-phase4`:** `cargo test` = 32 passing; no `tests/smoke.rs`; no CI; extension is loaded and owns its D-Bus name (live-testable after a reload).
- **Branch:** `dx-phase4` is based on `staging` and lacks Phase 3's tests/CI. That's intentional (maintainer's choice). This plan adds its own tests. Editing a GNOME extension requires a **log out / log in** on Wayland to reload (see the `deploying-claude-focus` skill).

---

## File structure

- **Create** `src/focus.rs` — `Focuser` trait, `FocusTarget`, `FocusOutcome`, `detect_focuser`, `NoopFocuser`, and the pure helpers `session_marker`, `should_use_gnome_shell`, `parse_focus_outcome`, `focus_suppresses_notify`.
- **Modify** `src/dbus.rs` — replace fire-and-forget `highlight_window` with `GnomeShellFocuser` (sync + detached gdbus calls).
- **Modify** `src/main.rs` — `mod focus;`; thread `session_id` into `dispatch`; route `SessionStart` to emit `terminalSequence`; build `FocusTarget` and gate notify on the outcome.
- **Modify** `src/notify.rs` — make `project_basename` `pub(crate)` for reuse.
- **Modify** `src/doctor.rs` — generalize hook check to any event; add SessionStart + disable-title legs.
- **Modify** `extension/extension.js` — add `HighlightBySession` + `_resolveWindow`.
- **Modify** `extension/metadata.json` — `version` 1 → 2.
- **Modify** `scripts/install.sh` — register the SessionStart hook + opt-in `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`.
- **Create** `tests/session_start.rs` — integration test for the SessionStart `terminalSequence` output and the always-exit-0 contract.
- **Modify** `README.md` — document multi-window matching, the opt-in, and the title format.

---

## Task 1: Scaffold `src/focus.rs` with types + `session_marker`

**Files:**
- Create: `src/focus.rs`
- Modify: `src/main.rs` (add `mod focus;`)

- [ ] **Step 1: Create `src/focus.rs` with types and the failing test**

```rust
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
}
```

- [ ] **Step 2: Register the module in `src/main.rs`**

Add `mod focus;` to the module list at the top (after `mod dbus;`):

```rust
mod config;
mod dbus;
mod doctor;
mod focus;
mod notify;
mod process_tree;
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test --lib focus::`
Expected: 3 new tests pass; total still compiles. (A `dead_code` warning on the trait/types is expected until later tasks wire them — that's fine for now; do NOT add `#[allow]` blanket-wide.)

- [ ] **Step 4: Commit**

```bash
git add src/focus.rs src/main.rs
git commit -m "feat: add Focuser types and session marker derivation"
```

---

## Task 2: `parse_focus_outcome` (gdbus output → `FocusOutcome`)

**Files:**
- Modify: `src/focus.rs`

- [ ] **Step 1: Add the failing test to `src/focus.rs`**

Add inside `mod tests`:

```rust
    #[test]
    fn parses_gdbus_tuple_into_outcome() {
        assert_eq!(parse_focus_outcome("(true, false)\n"), FocusOutcome::Raised);
        assert_eq!(parse_focus_outcome("(true, true)\n"), FocusOutcome::AlreadyFocused);
        assert_eq!(parse_focus_outcome("(false, false)\n"), FocusOutcome::NotFound);
    }

    #[test]
    fn malformed_gdbus_output_is_unavailable() {
        assert_eq!(parse_focus_outcome(""), FocusOutcome::Unavailable);
        assert_eq!(parse_focus_outcome("nonsense"), FocusOutcome::Unavailable);
        assert_eq!(parse_focus_outcome("(true)"), FocusOutcome::Unavailable);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib focus::parses_gdbus`
Expected: FAIL — `parse_focus_outcome` not found.

- [ ] **Step 3: Implement `parse_focus_outcome` in `src/focus.rs`**

Add at module level (above `#[cfg(test)]`):

```rust
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
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --lib focus::`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src/focus.rs
git commit -m "feat: parse gdbus focus reply into FocusOutcome"
```

---

## Task 3: `should_use_gnome_shell` backend-selection predicate

**Files:**
- Modify: `src/focus.rs`

- [ ] **Step 1: Add the failing test**

Add inside `mod tests`:

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib focus::gnome`
Expected: FAIL — `should_use_gnome_shell` not found (or arity mismatch).

- [ ] **Step 3: Implement the predicate**

Add at module level in `src/focus.rs`:

```rust
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
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --lib focus::`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add src/focus.rs
git commit -m "feat: GNOME backend selection predicate"
```

---

## Task 4: `focus_suppresses_notify` (4.3b decision, pure)

**Files:**
- Modify: `src/focus.rs`

- [ ] **Step 1: Add the failing test**

Add inside `mod tests`:

```rust
    #[test]
    fn already_focused_suppresses_only_when_not_forced() {
        assert!(focus_suppresses_notify(&FocusOutcome::AlreadyFocused, false));
        assert!(!focus_suppresses_notify(&FocusOutcome::AlreadyFocused, true)); // test/force
    }

    #[test]
    fn other_outcomes_never_suppress() {
        for o in [FocusOutcome::Raised, FocusOutcome::NotFound, FocusOutcome::Unavailable] {
            assert!(!focus_suppresses_notify(&o, false));
        }
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib focus::already_focused`
Expected: FAIL — `focus_suppresses_notify` not found.

- [ ] **Step 3: Implement**

Add at module level in `src/focus.rs`:

```rust
/// 4.3b: suppress the banner+sound when the target window is already focused —
/// but never when `force` (the `test` subcommand), which must always alert.
pub fn focus_suppresses_notify(outcome: &FocusOutcome, force: bool) -> bool {
    !force && *outcome == FocusOutcome::AlreadyFocused
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --lib focus::`
Expected: PASS (10 tests).

- [ ] **Step 5: Commit**

```bash
git add src/focus.rs
git commit -m "feat: notify-suppression decision for already-focused windows"
```

---

## Task 5: `NoopFocuser`, `detect_focuser`, and `GnomeShellFocuser`

**Files:**
- Modify: `src/focus.rs` (NoopFocuser + detect_focuser)
- Modify: `src/dbus.rs` (replace `highlight_window` with `GnomeShellFocuser`)

- [ ] **Step 1: Add `NoopFocuser` + `detect_focuser` to `src/focus.rs`**

Add at module level (above `#[cfg(test)]`):

```rust
/// Fallback backend: no compositor integration available. Still lets the caller
/// notify; never spawns anything.
pub struct NoopFocuser;

impl Focuser for NoopFocuser {
    fn focus(&self, _target: &FocusTarget) -> FocusOutcome {
        FocusOutcome::Unavailable
    }
    fn focus_detached(&self, _target: &FocusTarget) {}
}

/// Whether `gdbus` is found on `$PATH`. A pure filesystem scan (no subprocess),
/// so it's cheap on the notification path, and lets `detect_focuser` avoid even
/// attempting a gdbus call when the tool isn't installed (spec 4.1).
fn gdbus_on_path() -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join("gdbus").is_file())
}

/// Pick a backend from the environment. GNOME desktop + gdbus → the Shell-
/// extension backend; anything else → `NoopFocuser` (so the binary never even
/// attempts a gdbus call into the void). X11/non-GNOME is Phase 5.
pub fn detect_focuser() -> Box<dyn Focuser> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    if should_use_gnome_shell(desktop.as_deref(), gdbus_on_path()) {
        Box::new(crate::dbus::GnomeShellFocuser)
    } else {
        Box::new(NoopFocuser)
    }
}
```

- [ ] **Step 2: Add the NoopFocuser test**

Add inside `mod tests`:

```rust
    #[test]
    fn noop_focuser_is_always_unavailable() {
        let t = FocusTarget { pid: Some(1), session_marker: None, duration_ms: 0 };
        assert_eq!(NoopFocuser.focus(&t), FocusOutcome::Unavailable);
        NoopFocuser.focus_detached(&t); // must not panic
    }
```

- [ ] **Step 3: Replace `src/dbus.rs` contents with `GnomeShellFocuser`**

Replace the entire file `src/dbus.rs` with:

```rust
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
            Ok(o) if o.status.success() => {
                parse_focus_outcome(&String::from_utf8_lossy(&o.stdout))
            }
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
```

- [ ] **Step 4: Run to verify it compiles and tests pass**

Run: `cargo test --lib`
Expected: PASS. `src/main.rs` still calls the old `dbus::highlight_window` — that is removed in Task 6, so if the build fails here with "cannot find function `highlight_window`", proceed directly to Task 6 (these two tasks form one compile unit). To keep this task self-contained, instead run:

Run: `cargo build 2>&1 | head -5`
Expected: the only error is `dbus::highlight_window` not found in `main.rs` (fixed next task). All of `focus.rs`/`dbus.rs` compiles.

- [ ] **Step 5: Commit**

```bash
git add src/focus.rs src/dbus.rs
git commit -m "feat: GnomeShellFocuser backend + NoopFocuser fallback + detect_focuser"
```

---

## Task 6: Wire `dispatch` to the `Focuser` and thread `session_id`

**Files:**
- Modify: `src/main.rs` (`dispatch`, `run`, `run_test`)

- [ ] **Step 1: Replace `dispatch` in `src/main.rs`**

Replace the whole `dispatch` function (currently `fn dispatch(... force: bool) { ... }`) with:

```rust
/// Perform the focus and/or notify actions for a notification.
/// Shared by the real hook path (`force = false`, honors `mode`) and the
/// `test` subcommand (`force = true`, fires BOTH legs regardless of `mode`).
fn dispatch(
    notification_type: &str,
    message: &str,
    cwd: Option<&str>,
    session_id: Option<&str>,
    config: &config::Config,
    force: bool,
) {
    let should_focus = force || config.mode == Mode::Both || config.mode == Mode::FocusOnly;
    let should_notify = force || config.mode == Mode::Both || config.mode == Mode::NotifyOnly;

    let outcome = if should_focus {
        let target = focus::FocusTarget {
            pid: process_tree::find_terminal_pid(),
            session_marker: session_id.and_then(focus::session_marker),
            duration_ms: config.notification_timeout_ms,
        };
        let focuser = focus::detect_focuser();
        if should_notify {
            // Need the result to gate notify (4.3b): synchronous, bounded call.
            focuser.focus(&target)
        } else {
            // Focus-only: nothing to gate — keep the fast fire-and-forget path.
            focuser.focus_detached(&target);
            focus::FocusOutcome::Unavailable
        }
    } else {
        focus::FocusOutcome::Unavailable
    };

    if should_notify && !focus::focus_suppresses_notify(&outcome, force) {
        notify::send_notification(notification_type, message, cwd, config, force);
    }
}
```

- [ ] **Step 2: Update the `run` call site to pass `session_id`**

In `run()`, change the `dispatch(...)` call to thread the session id:

```rust
    dispatch(
        notification_type,
        message,
        hook_input.cwd.as_deref(),
        hook_input.session_id.as_deref(),
        &config,
        false,
    );
```

- [ ] **Step 3: Update the `run_test` call site to pass `None` for session_id**

In `run_test()`, change the `dispatch(&ty, "", cwd.as_deref(), &config, true);` line to:

```rust
        dispatch(&ty, "", cwd.as_deref(), None, &config, true);
```

(`test` highlights the invoking terminal via the PID fallback — it has no Claude session id.)

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: PASS — all prior tests plus the focus tests; no `highlight_window` reference remains.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: route focus through the Focuser trait and pass session_id"
```

---

## Task 7: `SessionStart` title tagging (`terminalSequence`)

**Files:**
- Modify: `src/notify.rs` (make `project_basename` reusable)
- Modify: `src/main.rs` (`run` branch + `run_session_start` + helpers)

- [ ] **Step 1: Make `project_basename` reusable in `src/notify.rs`**

Change its signature from `fn project_basename` to `pub(crate) fn project_basename` (one word added). Nothing else in `notify.rs` changes.

```rust
/// Final path component of `cwd` (the project dir), or None when empty / root.
pub(crate) fn project_basename(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
}
```

- [ ] **Step 2: Add failing tests for the title + sequence builders in `src/main.rs`**

Add inside `src/main.rs`'s `mod tests`:

```rust
    #[test]
    fn session_start_title_includes_project_and_marker() {
        assert_eq!(
            session_start_title(Some("/home/u/git_repos/claude-focus"), "[cf:50613d2b]"),
            "claude · claude-focus [cf:50613d2b]"
        );
    }

    #[test]
    fn session_start_title_without_project() {
        assert_eq!(session_start_title(None, "[cf:abc]"), "claude [cf:abc]");
        assert_eq!(session_start_title(Some("/"), "[cf:abc]"), "claude [cf:abc]");
    }

    #[test]
    fn session_start_sequence_is_osc2_json_with_marker() {
        let json = session_start_terminal_sequence(
            Some("50613d2b-7490-497a-965c-6992e1bc7d45"),
            Some("/home/u/git_repos/claude-focus"),
        )
        .expect("a session id yields a sequence");
        // Parse it back and check the exact OSC 2 string.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v["terminalSequence"].as_str().unwrap(),
            "\u{1b}]2;claude · claude-focus [cf:50613d2b]\u{7}"
        );
    }

    #[test]
    fn session_start_sequence_none_without_session_id() {
        assert_eq!(session_start_terminal_sequence(None, Some("/tmp")), None);
        assert_eq!(session_start_terminal_sequence(Some(""), Some("/tmp")), None);
    }
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test --lib tests::session_start`
Expected: FAIL — builders not found.

- [ ] **Step 4: Implement the builders + `run_session_start` + the `run` branch in `src/main.rs`**

Add these two functions at module level (e.g. just below `should_act`):

```rust
/// The window title set for a Claude session: `claude · <project> [cf:<id8>]`,
/// or `claude [cf:<id8>]` when there is no project dir. The bracketed marker is
/// what the extension matches.
fn session_start_title(cwd: Option<&str>, marker: &str) -> String {
    match cwd.and_then(notify::project_basename) {
        Some(project) => format!("claude · {project} {marker}"),
        None => format!("claude {marker}"),
    }
}

/// Build the SessionStart hook's JSON stdout: a top-level `terminalSequence`
/// carrying an OSC 2 (window title) escape, BEL-terminated. `None` when there is
/// no session id to tag (then no title is set and matching uses the PID fallback).
fn session_start_terminal_sequence(session_id: Option<&str>, cwd: Option<&str>) -> Option<String> {
    let marker = session_id.and_then(focus::session_marker)?;
    let title = session_start_title(cwd, &marker);
    let osc = format!("\u{1b}]2;{title}\u{7}");
    Some(serde_json::json!({ "terminalSequence": osc }).to_string())
}
```

Add the handler function (e.g. below `run`):

```rust
/// SessionStart hook: tag the terminal title so the extension can later match
/// this exact window. Prints the JSON Claude Code consumes, then exits 0.
fn run_session_start(input: &HookInput) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(json) =
        session_start_terminal_sequence(input.session_id.as_deref(), input.cwd.as_deref())
    {
        println!("{json}");
    }
    Ok(())
}
```

In `run()`, branch on the event right after parsing `hook_input` (before loading config / `should_act`):

```rust
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;

    let hook_input: HookInput = serde_json::from_str(&input)?;

    if hook_input.hook_event_name.as_deref() == Some("SessionStart") {
        return run_session_start(&hook_input);
    }

    let config = config::load_config();

    let notification_type = hook_input.notification_type.as_deref().unwrap_or("");
    let message = hook_input.message.as_deref().unwrap_or("");

    if !should_act(notification_type, &config.notify_types) {
        return Ok(());
    }

    dispatch(
        notification_type,
        message,
        hook_input.cwd.as_deref(),
        hook_input.session_id.as_deref(),
        &config,
        false,
    );

    Ok(())
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test`
Expected: PASS (all, incl. the 4 new builder tests).

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/notify.rs
git commit -m "feat: tag terminal title with session id on SessionStart"
```

---

## Task 8: Extension `HighlightBySession` + `_resolveWindow` + version bump

**Files:**
- Modify: `extension/extension.js`
- Modify: `extension/metadata.json`

- [ ] **Step 1: Add the method to `IFACE_XML`**

In `extension/extension.js`, inside the `<interface ...>` block of `IFACE_XML`, add after the `HighlightByPid` method:

```xml
    <method name="HighlightBySession">
      <arg type="s" direction="in" name="marker"/>
      <arg type="u" direction="in" name="pid"/>
      <arg type="u" direction="in" name="duration_ms"/>
      <arg type="b" direction="out" name="found"/>
      <arg type="b" direction="out" name="already_focused"/>
    </method>
```

- [ ] **Step 2: Add `HighlightBySession` and `_resolveWindow` methods**

In the class, add these two methods (e.g. right after `HighlightByPid`):

```javascript
    HighlightBySession(marker, pid, duration_ms) {
        const win = this._resolveWindow(marker, pid);
        if (!win) return [false, false];

        // 4.3a: if it's already the focused window, do nothing (no flash).
        if (global.display.get_focus_window() === win) {
            return [true, true];
        }

        const workspace = win.get_workspace();
        const activeWorkspace = global.workspace_manager.get_active_workspace();
        if (workspace && workspace !== activeWorkspace) {
            workspace.activate(global.get_current_time());
        }
        Main.activateWindow(win);
        this._highlightWindow(win, duration_ms || 3000);
        return [true, false];
    }

    _resolveWindow(marker, pid) {
        // 1. Precise: a window whose title carries this session's marker.
        if (marker && marker.length > 0) {
            const tagged = global.get_window_actors()
                .map(actor => actor.get_meta_window())
                .filter(win => win && win.get_title() && win.get_title().includes(marker));
            if (tagged.length > 0) {
                tagged.sort((a, b) => b.get_user_time() - a.get_user_time());
                return tagged[0];
            }
        }
        // 2. Fallback: today's PID + most-recently-focused selection.
        return this._findBestWindowByPid(pid);
    }
```

(`_findBestWindowByPid` already returns `null` when `pid` matches nothing, including `pid === 0`, so the fallback is safe when no marker and no real pid are supplied.)

- [ ] **Step 3: Bump the extension version**

In `extension/metadata.json`, change `"version": 1` to `"version": 2` (extension.js changed — the documented bump rule).

- [ ] **Step 4: Lint the JS for obvious syntax errors**

Run: `node --check extension/extension.js`
Expected: no output (valid syntax). If `node` is unavailable, skip — it is verified live in Task 13.

- [ ] **Step 5: Commit**

```bash
git add extension/extension.js extension/metadata.json
git commit -m "feat: extension HighlightBySession with title-marker match + already-focused no-op"
```

---

## Task 9: `doctor` legs for SessionStart hook + disable-title env

**Files:**
- Modify: `src/doctor.rs`

- [ ] **Step 1: Add failing tests to `src/doctor.rs`**

Add inside `mod tests`:

```rust
    const WITH_SS: &str = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/home/u/.local/bin/claude-focus"}]}]}}"#;

    #[test]
    fn detects_session_start_hook() {
        assert_eq!(hook_registered_for(WITH_SS, EXPECTED, "SessionStart"), Ok(true));
    }

    #[test]
    fn session_start_absent_when_only_notification() {
        assert_eq!(hook_registered_for(WITH_HOOK, EXPECTED, "SessionStart"), Ok(false));
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib doctor::`
Expected: FAIL — `hook_registered_for` and `disable_title_env_set` not found.

- [ ] **Step 3: Generalize `hook_registered` and add `disable_title_env_set`**

In `src/doctor.rs`, replace the `hook_registered` function with a generic version plus a back-compat wrapper:

```rust
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
```

- [ ] **Step 4: Add a `warn_line` helper**

Add next to `line` in `src/doctor.rs`:

```rust
fn warn_line(ok: bool, label: &str, note: &str) {
    if ok {
        println!("  [PASS] {label}");
    } else {
        println!("  [WARN] {label}\n         {note}");
    }
}
```

- [ ] **Step 5: Replace the hook-registration block in `run()`**

In `doctor::run()`, replace the **final hook-registration block** — the three `let home / expected_command / settings` bindings (currently `src/doctor.rs:132-134`) **through** the end of the `match std::fs::read_to_string(&settings) { ... }` (line 142) — with the block below. The new block re-declares those three `let`s, so delete the originals to avoid duplicate-binding shadows. Note the Notification leg calls the `hook_registered` **wrapper** (keeping it a live non-test caller, so clippy's `dead_code` lint stays quiet):

```rust
    let home = std::env::var("HOME").unwrap_or_default();
    let expected_command = format!("{home}/.local/bin/claude-focus");
    let settings = format!("{home}/.claude/settings.json");
    match std::fs::read_to_string(&settings) {
        Ok(s) => {
            match hook_registered(&s, &expected_command) {
                Ok(true) => line(true, "Notification hook registered", ""),
                Ok(false) => line(false, "Notification hook registered", "re-run scripts/install.sh"),
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
        Err(_) => line(false, "Notification hook registered", "run scripts/install.sh"),
    }
```

- [ ] **Step 6: Run to verify pass**

Run: `cargo test --lib doctor::`
Expected: PASS (the original 3 doctor tests + 4 new).

- [ ] **Step 7: Commit**

```bash
git add src/doctor.rs
git commit -m "feat: doctor checks SessionStart hook + disable-title env"
```

---

## Task 10: Installer — SessionStart registration + opt-in disable-title

**Files:**
- Modify: `scripts/install.sh`

- [ ] **Step 1: Replace `do_hook()` in `scripts/install.sh`**

Replace the entire `do_hook()` function with:

```bash
do_hook() {
    echo "==> Merging hooks into Claude Code settings..."
    mkdir -p "$(dirname "$SETTINGS_FILE")"

    # Opt-in: precise multi-window matching needs Claude's dynamic title OFF so
    # our session-id title tag persists. Prompt only on a real terminal;
    # non-interactive installs never change the user's Claude behavior.
    local disable_title=0
    if [ -t 0 ]; then
        echo ""
        echo "    Precise multi-window matching tags each terminal's title with the Claude"
        echo "    session id and matches it. It needs Claude's own dynamic title OFF"
        echo "    (CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1); your title becomes e.g."
        echo "    'claude · myproject [cf:1a2b3c4d]'. Without it, claude-focus still works"
        echo "    but falls back to best-effort PID matching for gnome-terminal windows."
        read -r -p "    Enable precise window matching? [y/N] " ans
        case "$ans" in [Yy]*) disable_title=1 ;; esac
    else
        echo "    (non-interactive: leaving Claude's title untouched; PID fallback in use)"
    fi

    SETTINGS_FILE="$SETTINGS_FILE" BIN_DIR="$BIN_DIR" DISABLE_TITLE="$disable_title" python3 - <<'PY'
import json, os, stat, tempfile
settings_file = os.environ['SETTINGS_FILE']
hook_command = os.path.join(os.environ['BIN_DIR'], 'claude-focus')
disable_title = os.environ.get('DISABLE_TITLE') == '1'

settings = {}
if os.path.exists(settings_file):
    try:
        with open(settings_file) as f:
            settings = json.load(f)
    except (ValueError, OSError) as e:
        raise SystemExit(
            "    ERROR: %s is not valid JSON (%s).\n"
            "    Fix it or back it up, then re-run install. Left it untouched."
            % (settings_file, e)
        )
    if not isinstance(settings, dict):
        raise SystemExit(
            "    ERROR: %s is valid JSON but not a JSON object.\n"
            "    Fix it or back it up, then re-run install. Left it untouched."
            % settings_file
        )

changed = False


def ensure_hook(event, with_matcher):
    global changed
    entries = settings.setdefault('hooks', {}).setdefault(event, [])
    present = any(
        any(h.get('command') == hook_command for h in entry.get('hooks', []))
        for entry in entries
    )
    if present:
        print('    %s hook already present, skipping' % event)
        return
    entry = {'hooks': [{'type': 'command', 'command': hook_command}]}
    if with_matcher:
        entry['matcher'] = '*'
    entries.append(entry)
    print('    %s hook added' % event)
    changed = True


ensure_hook('Notification', True)
ensure_hook('SessionStart', False)

if disable_title:
    env = settings.setdefault('env', {})
    if env.get('CLAUDE_CODE_DISABLE_TERMINAL_TITLE') != '1':
        env['CLAUDE_CODE_DISABLE_TERMINAL_TITLE'] = '1'
        print('    Set CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1 (precise window matching enabled)')
        changed = True
    else:
        print('    CLAUDE_CODE_DISABLE_TERMINAL_TITLE already set')
else:
    print('    Left Claude title behavior unchanged (precise matching off; PID fallback)')

if changed:
    # Atomic write: temp file in the same dir + os.replace, so a crash never
    # truncates the user's Claude settings.
    d = os.path.dirname(settings_file) or '.'
    fd, tmp = tempfile.mkstemp(dir=d, prefix='.settings.', suffix='.tmp')
    try:
        with os.fdopen(fd, 'w') as f:
            json.dump(settings, f, indent=2)
        if os.path.exists(settings_file):
            os.chmod(tmp, stat.S_IMODE(os.stat(settings_file).st_mode))
        os.replace(tmp, settings_file)
    except BaseException:
        if os.path.exists(tmp):
            os.remove(tmp)
        raise
    print('    Wrote', settings_file)
else:
    print('    No settings changes needed')
PY
}
```

- [ ] **Step 2: Shellcheck the script**

Run: `shellcheck scripts/install.sh`
Expected: no errors (warnings about the heredoc are not emitted; `read -r` is used). If `shellcheck` is not installed, note it and continue (CI/maintainer will catch it).

- [ ] **Step 3: Dry-run the merge against a throwaway settings file**

Do **not** `source` install.sh — its last line is `main "$@"`, which would run a full real install (build + copy binary + enable extension). Run only the merge logic against a temp file:

```bash
tmp=$(mktemp -d)
printf '{}' > "$tmp/settings.json"
SETTINGS_FILE="$tmp/settings.json" BIN_DIR="$HOME/.local/bin" DISABLE_TITLE=0 \
  python3 - <<'PY'
import json, os
settings_file = os.environ['SETTINGS_FILE']
hook_command = os.path.join(os.environ['BIN_DIR'], 'claude-focus')
settings = json.load(open(settings_file)) if os.path.exists(settings_file) else {}
def ensure_hook(event, with_matcher):
    entries = settings.setdefault('hooks', {}).setdefault(event, [])
    if any(any(h.get('command') == hook_command for h in e.get('hooks', [])) for e in entries):
        print('   ', event, 'already present'); return
    ent = {'hooks': [{'type': 'command', 'command': hook_command}]}
    if with_matcher: ent['matcher'] = '*'
    entries.append(ent); print('   ', event, 'added')
ensure_hook('Notification', True); ensure_hook('SessionStart', False)
json.dump(settings, open(settings_file, 'w'), indent=2)
print('env block present:', 'env' in settings)
PY
echo "--- result ---"; cat "$tmp/settings.json"; rm -rf "$tmp"
```
Expected: "Notification added" + "SessionStart added" + "env block present: False"; the JSON contains `hooks.Notification` (with `matcher: "*"`) and `hooks.SessionStart` (no matcher) entries with the command, and **no** `env` block. This mirrors the embedded merge's shape without building/copying/enabling anything.

- [ ] **Step 4: Commit**

```bash
git add scripts/install.sh
git commit -m "feat: install SessionStart hook and opt-in CLAUDE_CODE_DISABLE_TERMINAL_TITLE"
```

---

## Task 11: Integration test for the SessionStart contract

**Files:**
- Create: `tests/session_start.rs`

- [ ] **Step 1: Create `tests/session_start.rs`**

```rust
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
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
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
    assert!(stdout.trim().is_empty(), "no session id -> no terminalSequence");
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
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test --test session_start`
Expected: PASS (5 tests). (`serde_json` is already a dependency, usable in integration tests.)

- [ ] **Step 3: Commit**

```bash
git add tests/session_start.rs
git commit -m "test: SessionStart terminalSequence output and exit-0 contract"
```

---

## Task 12: README — document multi-window matching

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a "Right window, every time (multi-window)" section**

Insert a new section near the existing focus/notification description (after the "what it does" list). Use this content:

```markdown
## Right window, every time (multiple sessions)

`gnome-terminal` runs every window under one `gnome-terminal-server` process, so
matching by PID alone can raise the wrong window. claude-focus disambiguates by
tagging each terminal's title with the Claude session id and matching that:

- A **SessionStart** hook sets the title to `claude · <project> [cf:<id8>]` using
  Claude Code's `terminalSequence` output (no shell snippet, works over SSH/tmux).
- The **Notification** hook asks the GNOME extension to raise the window whose
  title carries the matching `[cf:<id8>]` marker.
- If no marker is present (you didn't opt in, older Claude Code, or a terminal
  that already has distinct PIDs), it falls back to PID + most-recently-focused —
  exactly the previous behavior. **Nothing regresses.**

### Enabling precise matching (opt-in)

The title tag only persists if Claude Code's own dynamic title is off. The
installer asks; choosing yes sets this in `~/.claude/settings.json`:

```json
{ "env": { "CLAUDE_CODE_DISABLE_TERMINAL_TITLE": "1" } }
```

Trade-off: you lose Claude's task-summary spinner title and get a stable
`claude · <project> [cf:…]` title instead (which is easier to tell apart in the
taskbar). To revert, remove that env entry. `claude-focus doctor` reports whether
the SessionStart hook and this env var are set.

### Skipping redundant alerts

If the matched window is already focused, the extension skips the border/raise,
and in `mode = both` the banner + sound are suppressed too — no flash or beep for
the window you're already looking at. `test` always alerts.
```

- [ ] **Step 2: Update the extension version note (if present)**

If `README.md` mentions the extension `version` or a version-bump rule, note the extension is now revision **2** (extension.js changed). If no such note exists, skip.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document multi-window matching, opt-in, and alert suppression"
```

---

## Task 13: Final verification (build, lint, live test on GNOME)

**Files:** none (verification only)

- [ ] **Step 1: Full Rust gate**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: fmt clean, clippy no warnings, all tests pass (32 baseline + the new focus/main/doctor unit tests + 3 integration tests).

- [ ] **Step 2: Shellcheck**

Run: `shellcheck scripts/install.sh`
Expected: clean.

- [ ] **Step 3: Install locally and reload the extension**

Run: `make update` (or `./scripts/install.sh --bin --ext`), answer **y** to the precise-matching prompt, then **log out and back in** to reload the GNOME extension (Wayland). See the `deploying-claude-focus` skill.

- [ ] **Step 4: Confirm the new D-Bus method is live**

Run:
```bash
gdbus introspect --session --dest org.gnome.Shell.Extensions.FocusByPid \
  --object-path /org/gnome/Shell/Extensions/FocusByPid | grep HighlightBySession
```
Expected: the `HighlightBySession` method appears. Then sanity-call it:
```bash
gdbus call --session --timeout 1 --dest org.gnome.Shell.Extensions.FocusByPid \
  --object-path /org/gnome/Shell/Extensions/FocusByPid \
  --method org.gnome.Shell.Extensions.FocusByPid.HighlightBySession "[cf:nope]" 0 1500
```
Expected: `(false, false)` (no window matches a bogus marker and pid 0).

- [ ] **Step 5: Live multi-window check (the acceptance test)**

1. Open **two** gnome-terminal **windows**, run `claude` in each on different projects.
2. Confirm each window's title shows `claude · <project> [cf:…]` (precise matching active).
3. In one session, trigger a notification (e.g. a permission prompt) and confirm **that window** is raised/bordered — not the other.
4. `claude-focus doctor` → SessionStart hook = PASS, `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` = PASS.
5. Focus the Claude window, trigger an `idle_prompt`, and confirm **no** flash/banner (4.3a/4.3b).
6. Temporarily remove the env var (or test a window without a tag) and confirm focus still works via the PID fallback (no regression).
7. **4.3b no-false-suppress:** disable the extension (`gnome-extensions disable focus-by-pid@claude.local`), then in `mode = both` trigger a notification while focused on the Claude window — confirm the banner **still fires** (extension unreachable → `Unavailable` → no suppression). Re-enable afterward.

- [ ] **Step 6: Final commit (only if Step 1 reformatted anything)**

```bash
git add -A
git commit -m "style: cargo fmt" || echo "nothing to commit"
```

---

## Self-review notes (completed by the planner)

- **Spec coverage:** 4.1 (Tasks 1–6: trait, types, detect_focuser, Noop, GnomeShell), 4.2 (Tasks 7–8, 10: SessionStart tag + extension match + install), 4.3a (Task 8), 4.3b (Tasks 4, 6, 8), install/doctor (Tasks 9–10), tests (every task + Task 11), README (Task 12), verification (Task 13). No spec item is unaddressed.
- **Deviations from the spec (intentional, all toward "verified-better"):** the mechanism is a SessionStart `terminalSequence` tag (not a shell rc snippet — the snippet can't see the session id and Claude owns the title); the backend is named `GnomeShellFocuser` (works on GNOME X11+Wayland, not just Wayland); the bounded sync call uses `gdbus --timeout 1` (1 s cap; whole-second granularity) rather than a 250 ms watchdog.
- **DRY:** `project_basename` promoted to `pub(crate)` and reused; `hook_registered` generalized to `hook_registered_for` (the Notification leg keeps calling the `hook_registered` wrapper, so it isn't dead code); `_resolveWindow` reuses `_findBestWindowByPid` for the fallback; the whole gdbus argv is centralized in `GnomeShellFocuser::call_args` (one builder, both call sites).
- **`detect_focuser` honors spec 4.1 literally:** it selects the GNOME backend only when the desktop is GNOME **and** `gdbus` is on `$PATH` (a pure-Rust PATH scan, no subprocess), so with `gdbus` absent the binary never attempts a gdbus call.
- **Adversarial plan-review pass (4 reviewers) applied:** fixed the dead-code clippy regression (Task 9), the `source`-install dry-run footgun (Task 10), the missing gdbus-on-PATH gate (Tasks 3/5), the missing malformed/empty-stdin smoke tests (Task 11), and the duplicate-`let` span ambiguity (Task 9). Extension JS reviewed clean.
- **Type consistency:** `FocusOutcome`/`FocusTarget`/`Focuser`/`focus`/`focus_detached`/`session_marker`/`parse_focus_outcome`/`should_use_gnome_shell`/`focus_suppresses_notify`/`hook_registered_for`/`disable_title_env_set`/`session_start_title`/`session_start_terminal_sequence` are used identically wherever referenced.
```
