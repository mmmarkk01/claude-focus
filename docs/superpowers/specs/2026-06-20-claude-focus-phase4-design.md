# claude-focus Phase 4 — Right window, every time (design)

**Issue:** [#32](https://github.com/mmmarkk01/claude-focus/issues/32) · **Part of:** [#28](https://github.com/mmmarkk01/claude-focus/issues/28)
**Roadmap spec §4:** `docs/superpowers/specs/2026-06-05-claude-focus-dx-roadmap-design.md` (on `dx-roadmap`)
**Date:** 2026-06-20

## Goal

The correct terminal is focused even with multiple concurrent Claude sessions,
redundant alerts are suppressed when you are already looking at the window, and
the focus path is abstracted behind a trait so Phase 5 (X11) can add a backend
cleanly. Every change degrades gracefully and never regresses today's behavior.

## Design ethos (inherited, constrains every decision)

Minimal & fast · no daemon / no standing state · zero runtime deps beyond
standard Linux tools · never block Claude Code (always exit 0) · degrade
gracefully · **don't lie** (no silent no-ops, no silently-discarded config).

Phase 4 takes **one deliberate, documented exception** to "fire-and-forget" — see
§4.3b — gated so it only applies on the path that needs it.

---

## What the spike established (why the spec's original 4.2 plan changed)

The roadmap spec proposed disambiguating gnome-terminal windows with "a shell rc
snippet that emits an OSC title escape carrying the Claude `session_id`, at shell
init." Empirical investigation on the target machine (GNOME 48 / Wayland,
gnome-terminal, Claude Code 2.1.181) disproved the premises of that plan and
revealed a better mechanism:

1. **One server PID for all windows (the core bug, confirmed).**
   `gnome-terminal-server` is a single process; every window/tab reports the same
   PID. The extension's `_findBestWindowByPid` sorts PID matches by
   `get_user_time()` and can raise the wrong window.

2. **No D-Bus screen→window map.** gnome-terminal exposes each tab as a
   `…/screen/<uuid>` object (interface `org.gnome.Terminal.Terminal0`, only an
   `Exec` method) and each window as `…/window/<n>` (`org.gtk.Actions`). The root
   `ObjectManager` lists screens but links them to **no** window. So a process
   that knows its own `GNOME_TERMINAL_SCREEN` **cannot** derive its window — the
   "match `get_gtk_window_object_path()`" idea is impossible here.

3. **The hook cannot write to the terminal.** A Notification hook runs with a
   piped stdin and **no controlling tty** (`/dev/tty` → "No such device or
   address"). It cannot emit an OSC title escape itself. (Confirms the spec.)

4. **Claude Code owns the title.** Claude dynamically rewrites the terminal title
   (task summary + spinner). Any marker a shell snippet set before launch is
   clobbered, and `PROMPT_COMMAND` never fires while Claude is in the foreground.
   So the shell-snippet approach cannot keep a marker visible. **The title is the
   only per-window field the extension can read (`win.get_title()`), but a snippet
   cannot keep a value in it.**

5. **A sanctioned, race-free title channel exists.** Claude Code hooks (≥ v2.1.141;
   machine has 2.1.181) accept a `terminalSequence` JSON output field —
   *"a terminal escape sequence for Claude Code to emit on your behalf … window
   title … Use this instead of writing to `/dev/tty`, which is unavailable to
   hooks."* It allows OSC 0/1/2 (titles). It is available on **all** hook events.
   `session_id` is a common input field on every event and equals the exported
   `CLAUDE_CODE_SESSION_ID`. And `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` turns off
   Claude's own dynamic title so an explicitly-set title persists.

**Conclusion:** replace the shell-rc-snippet with a **SessionStart hook that emits
a `terminalSequence` title tag**, made persistent by
`CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1`. This is strictly better than the spec's
idea: no per-shell snippet, works for any shell and over SSH/tmux, no `/dev/tty`,
race-free, and it uses the real `session_id` (which a launching shell never has).

Sources: Claude Code hooks reference (`terminalSequence`), env-vars docs and
issue #16572 (`CLAUDE_CODE_DISABLE_TERMINAL_TITLE`), verified 2026-06-20.

---

## Architecture overview

```
SessionStart event ─▶ claude-focus (hook)
                        └─ emits {"terminalSequence": OSC2 "claude · <project> [cf:<id8>]"}
                           (persists because CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1)

Notification event ─▶ claude-focus (hook)
                        ├─ build FocusTarget { pid, session_marker: "cf:<id8>", duration }
                        ├─ detect_focuser() → GnomeWaylandFocuser | NoopFocuser
                        ├─ focuser.focus(&target) ──gdbus──▶ extension.HighlightBySession
                        │        extension: match win.get_title() ∋ marker
                        │                   else PID + get_user_time (today's fallback)
                        │                   if already focused → no-op raise/border (4.3a)
                        │                   return (found, already_focused)
                        └─ notify unless outcome == AlreadyFocused (4.3b)
```

The marker is `cf:` + the first 8 hex chars of `session_id` (e.g. `cf:50613d2b`).
Collisions across concurrent sessions are effectively impossible at 8 hex chars.

---

## 4.1 — `Focuser` trait + graceful fallback

**New module `src/focus.rs`:**

```rust
pub struct FocusTarget {
    pub pid: u32,
    pub session_marker: Option<String>, // "cf:<id8>"; None when no session_id
    pub duration_ms: u32,
}

pub enum FocusOutcome {
    Raised,         // a window was found and raised/highlighted
    AlreadyFocused, // a window was found but was already focused (4.3a no-op'd it)
    NotFound,       // backend ran but matched no window
    Unavailable,    // no usable backend, or the call failed/timed out
}

pub trait Focuser {
    fn focus(&self, target: &FocusTarget) -> FocusOutcome;
}

pub fn detect_focuser() -> Box<dyn Focuser>;
```

- `dbus.rs` becomes the `GnomeWaylandFocuser` implementation.
- `NoopFocuser` always returns `Unavailable` (notify still happens).
- `detect_focuser()` returns `GnomeWaylandFocuser` only when the environment looks
  like GNOME/Wayland **and** `gdbus` is on `PATH`; otherwise `NoopFocuser`. This is
  what stops the binary from silently firing a gdbus call into the void when the
  extension/compositor can't service it. The selection predicate is a pure,
  unit-tested function over an env snapshot.
- X11 (`X11Focuser`) is **out of scope** — Phase 5, behind this same trait.

**Acceptance:** behavior on GNOME/Wayland is unchanged; on a non-GNOME/non-Wayland
environment (or with `gdbus` absent) the tool notifies and does **not** attempt a
gdbus call.

---

## 4.2 — Disambiguate windows via a `session_id` title tag

### SessionStart title tagging (in `claude-focus`, the binary)

`run()` already parses `hook_event_name`. Branch on it:

- **`SessionStart`** → derive the marker from `session_id`, build the title
  `claude · <project> [cf:<id8>]` (project = basename of `cwd`; omitted if absent),
  and print `{"terminalSequence": "<ESC>]2;<title><BEL>"}` to stdout, then exit 0.
  No focus/notify on this event.
- **`Notification`** (and absent/unknown) → today's focus+notify path.
- The OSC string builder and the title/marker derivation are pure, unit-tested
  functions. The escape uses `\x1b]2;…\x07` (OSC 2, BEL-terminated) — within the
  `terminalSequence` allowlist.

### Matching in the extension

New D-Bus method on the existing interface:

```
HighlightBySession(in s marker, in u pid, in u duration_ms,
                   out b found, out b already_focused)
```

Resolution order inside the extension:

1. If `marker` is non-empty, select the window whose `get_title()` **contains**
   `marker`. (Among multiple — should not happen — most-recently-focused wins.)
2. Otherwise, or if no title matches, fall back to **today's** behavior:
   `get_pid() === pid`, sorted by `get_user_time()`.
3. If a window is resolved and it is already
   `global.display.get_focus_window()`, **do not** raise/border it → return
   `(found=true, already_focused=true)` (this is 4.3a).
4. If resolved and not focused → raise + highlight, return
   `(found=true, already_focused=false)`.
5. If nothing resolves → `(found=false, already_focused=false)`.

The legacy `ActivateByPid` / `HighlightByPid` methods stay for backward
compatibility (a new binary against an old extension, or vice-versa, degrades
rather than breaks). `metadata.json` `version` bumps `1 → 2` (extension.js
changed — the documented bump rule from Phase 3.3).

**Guaranteed fallback (no regression):** when the marker is absent from every
title — user declined the opt-in, Claude < 2.1.141, or a terminal that already
works by distinct PID — resolution falls to PID + `get_user_time`, i.e. exactly
today's selection.

**Acceptance:** with 2+ gnome-terminal windows running different tagged sessions,
the window whose title carries the payload's marker is raised; with no tag
present, selection matches current behavior.

---

## 4.3a — Skip the redundant raise/border when already focused

Implemented inside `HighlightBySession` (step 3 above): if the resolved window is
already the focused window, the border/raise is a no-op and the method reports
`already_focused=true`. The D-Bus return value being read is what enables 4.3b;
4.3a itself stays fire-and-forget-shaped (the extension still returns quickly).

**Acceptance:** typing in the Claude terminal and hitting an `idle_prompt`
produces no border/raise flash for the window you are already in.

---

## 4.3b — Skip the redundant notification when already focused (opt-in, included)

The maintainer chose to include 4.3b. It needs the focus state *before* notifying,
which requires reading the extension's return value — a **synchronous** gdbus
round-trip. This is the one documented exception to fire-and-forget, scoped to
keep its cost off every other path:

- **Only `mode = both` pays it.** In `both`, the notify decision depends on the
  focus result, so `GnomeWaylandFocuser.focus()` runs the gdbus call
  **synchronously** with a hard timeout (≈ 250 ms; spawn + wait, kill on timeout),
  parses `(found, already_focused)`, and returns the matching `FocusOutcome`.
  `dispatch` then **suppresses notify + sound** when the outcome is
  `AlreadyFocused`.
- **`focus-only`** does not gate notify, so it need not wait on the result —
  the fast fire-and-forget spawn is preserved there. **`notify-only`** never
  focuses and notifies normally.
- **`force` (the `test` subcommand) never suppresses.** Exactly as `test` already
  bypasses the DND/quiet-hours gate, it bypasses already-focused suppression — a
  diagnostic must always show its banner. The two suppression gates **compose**:
  notify fires iff `force || (not DND/quiet-silenced && not AlreadyFocused)`.
- **Never false-suppress, never block.** Any timeout / parse failure / non-GNOME →
  `Unavailable` → notify normally. The binary still always exits 0. The audit
  established the hook is ~2 ms today; the bounded round-trip applies only to the
  `both` path and is capped, so the worst case is a small, bounded delay.
- The gdbus-output parser, and the `(FocusOutcome, mode, force, silenced) → notify?`
  decision, are pure, unit-tested functions.

**Acceptance:** in `mode = both`, an `idle_prompt` for the window you are already
focused on produces neither a flash nor a banner/sound; if the extension is
unreachable the banner still fires (no false-suppression).

---

## Install & self-diagnosis changes

- **SessionStart hook registration (`install.sh`).** Register the same
  `claude-focus` binary under `hooks.SessionStart` in `settings.json`, using the
  same idempotent, atomic-write merge the Notification hook already uses.
- **Opt-in `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` (`install.sh`).** The mechanism
  needs Claude's dynamic title off so the tag persists. The installer **prompts**:
  on yes, it sets `env.CLAUDE_CODE_DISABLE_TERMINAL_TITLE = "1"` in `settings.json`
  (atomic merge) and explains the trade-off (stable `claude · <project> [cf:…]`
  title instead of Claude's task-summary spinner) and how to revert; on no, it
  prints that precise window-matching is off and Phase 4 will use the PID
  fallback. Non-interactive installs (no tty) default to **not** setting it and
  print the same note — never silently change the user's Claude behavior.
- **`doctor` legs.** Add: SessionStart hook registered; `CLAUDE_CODE_DISABLE_
  TERMINAL_TITLE` set (WARN, not FAIL, when absent — the tool still works via
  fallback); each with an actionable fix line. Keep every leg crash-proof.

---

## Testing strategy

**Rust unit tests (TDD — write first):**
- marker derivation: `session_id` → `cf:<first-8-hex>`; missing/short id handled.
- title builder: `(project, marker)` → `claude · <project> [cf:…]`; project absent.
- `terminalSequence` JSON: correct OSC 2 + BEL, within the allowlist.
- `detect_focuser()` selection over an env snapshot (Wayland+GNOME+gdbus → Gnome;
  else Noop).
- gdbus-output parser: `(found, already_focused)` from representative gdbus stdout,
  plus malformed/empty/timeout → `Unavailable`.
- notify-suppression decision: `FocusOutcome` × `mode` × `force` → notify? (pure).

**Smoke tests (`tests/smoke.rs`):**
- `SessionStart` payload → exits 0 and stdout is valid JSON with a `terminalSequence`.
- `Notification` payload → still exits 0 (existing contract preserved).
- malformed / empty stdin → still exits 0.

**Not unit-testable → interactive verification on the machine (documented):**
- extension `HighlightBySession` matching, the already-focused no-op, and the
  `terminalSequence`↔`CLAUDE_CODE_DISABLE_TERMINAL_TITLE` interaction.
- multi-window gnome-terminal: confirm the matching session's window is raised;
  confirm fallback selection when the marker is absent.

All Phase 3 regression tests must stay green.

---

## Dependencies, risks, scope

- **Phase 3 regression net.** This branch is based on `staging`, which does **not**
  yet carry the Phase 3 tests (they live on `dx-phase3`). The spec intends those
  tests to protect this refactor; merging `dx-phase3` → `staging` first restores
  that net. Phase 4 also adds its own tests regardless.
- **`terminalSequence` × disable-title interaction** is verified by docs but
  confirmed empirically during implementation (interactive step).
- **In scope:** 4.1, 4.2, 4.3a, 4.3b (all GNOME/Wayland + gnome-terminal focused;
  other terminals already disambiguate by PID).
- **Out of scope:** X11 backend (Phase 5); any non-GNOME title mechanism.

## Versioning / release impact

- `extension.js` + `metadata.json` change → extension `version` `1 → 2`.
- Crate change is a user-facing **feature** → `feat:` commit (minor bump under the
  repo's semver scheme). No release-please; CI = `cargo fmt`/`clippy`/`test` +
  `shellcheck`. Commits carry the `Co-Authored-By` trailer.
