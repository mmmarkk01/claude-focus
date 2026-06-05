# claude-focus — DX & Feature Roadmap

**Date:** 2026-06-05
**Status:** Approved design (phasing confirmed). Phase 1 is the next slice to plan.
**Scope:** A single sequenced roadmap of correctness, DX, and feature work for claude-focus, decomposed into independently-shippable phases. Each phase gets its own implementation plan when it comes up; this document is the spec for the *sequence* and the contents of each phase, with Phase 1 specified in plan-ready detail.

---

## Background

claude-focus is a Linux tool that registers as a Claude Code **Notification hook**. When Claude Code emits a notification event (permission prompt, question, idle), it pipes a JSON payload to the `claude-focus` binary via stdin, which then — depending on config — focuses the terminal window, sends a desktop notification, and plays a sound. Three components:

- **Rust CLI** (`src/`) — `main.rs` (dispatch), `config.rs` (TOML load), `process_tree.rs` (/proc walker, tmux-aware), `dbus.rs` (gdbus spawn), `notify.rs` (notify-send + pw-play).
- **GNOME Shell extension** (`extension/extension.js`) — D-Bus methods `ActivateByPid` / `HighlightByPid`; required because Wayland forbids window activation from outside the compositor.
- **Config + installer** — `config/claude-focus.toml`, `scripts/install.sh`.

A prior whole-codebase audit (2026-06-05) established the key constraint that shapes this entire roadmap: **latency is already solved.** The binary cold-starts in ~0.6–0.9 ms and the full triggering hook runs end-to-end in ~2.2 ms; the `gdbus` call is fire-and-forget and off the critical path. A daemon would buy essentially nothing for speed. **This roadmap therefore spends zero effort on performance** and targets correctness, trust, and developer experience instead.

## Design principles (non-negotiable ethos)

These constrain every phase. An idea that violates one needs an explicit, strong justification.

1. **Minimal & fast.** Single fire-and-forget binary, no daemon, no standing state.
2. **Zero runtime deps beyond standard Linux tools.** notify-send, pw-play/gdbus, tmux are acceptable; network services and heavyweight libraries are not (without an explicit opt-in).
3. **Never block Claude Code.** Always exit 0. New subcommands must not change the default stdin-hook contract.
4. **Degrade gracefully.** Missing a dependency or compositor support means do less, never crash or hang.
5. **Don't lie.** The tool must not silently do nothing, silently discard config, or claim success it didn't achieve.

## Goals

- Make the tool's core promise — "bring the right terminal to the foreground" — actually true.
- Eliminate silent-failure footguns that erode trust.
- Make the black box diagnosable: a user who sees nothing happen can find out why in seconds.
- Make first-run installation verify itself.
- Then expand: disambiguate the correct window across multiple sessions, suppress redundant alerts, and broaden platform reach to X11.

## Non-goals

- Performance work / a daemon (latency is already ~2.2 ms).
- macOS and KDE-Wayland ports (real ports, unproven demand — see Parking Lot).
- Standing state of any kind (panel indicators, decision-log files, session registries).
- A broad CLI surface. Exactly three subcommands ship: `test`, `doctor`, `--version`.

---

## Roadmap overview

| Phase | Theme | Outcome | Depends on |
|---|---|---|---|
| 1 | Stop the lies + make it pokeable | Binary stops lying; `test`/`doctor`/`--version` exist | — |
| 2 | Trustworthy install + clearer notifications | Install verifies itself; banners say which project/urgency | Phase 1.5 (`doctor`) — 2.3 only |
| 3 | Lock it in | CI + tests pin the fixes before the big refactor | Phases 1–2 |
| 4 | Right window, every time | Correct window across sessions; no redundant alerts | Phase 3 (tests protect the refactor) |
| 5 | Reach | Works on X11 desktops with no extension | Phase 4 (`Focuser` trait) |

**Sequencing rationale (Approach A — leverage/trust first):** The P0 fixes are tiny and high-impact, so they ship first. CI/tests (Phase 3) land *before* the `Focuser` trait refactor (Phase 4) so there is a regression net protecting the riskiest change. The trait introduced in Phase 4 is the dependency for both the session-id disambiguation (Phase 4) and the X11 backend (Phase 5), so it precedes them.

---

## Phase 1 — Stop the lies + make it pokeable

**Goal:** The binary stops silently lying, the one core focus bug is fixed, and the tool becomes interactively testable and self-diagnosing. All items are small and code-verified.

### 1.1 Fix the same-workspace raise bug *(extension.js)*
`HighlightByPid` (extension.js:85–99) only calls `Main.activateWindow(win)` inside the `workspace !== activeWorkspace` branch (lines 92–95). On the common same-workspace case it draws the green border but **never raises the window**, contradicting the README's "bring to foreground." `ActivateByPid` (lines 70–83) already calls `Main.activateWindow(win)` unconditionally (line 81), proving the asymmetry is accidental.

- **Change:** Call `Main.activateWindow(win)` unconditionally before `_highlightWindow(win, ...)`. Keep the workspace-switch (`workspace.activate(...)`) inside the cross-workspace branch.
- **Acceptance:** A Claude session on the *current* workspace is raised to the foreground (not merely bordered) when the hook fires.

### 1.2 Warn-and-keep-defaults on config parse error *(config.rs)*
`load_config` (config.rs:64–70) calls `toml::from_str(&contents).unwrap_or_default()` (line 67), so a single TOML typo silently discards the **entire** config (mode → both, play_sound → off) with no signal.

- **Change:** Replace the `unwrap_or_default()` with a `match` that, on `Err(e)`, `eprintln!`s the `toml::de::Error` (it carries exact line/col) before falling back to `Config::default()`. stderr is captured in Claude Code's hook log.
- **Acceptance:** A malformed config still yields working defaults and the binary still **exits 0**, after emitting the `toml::de::Error` (which includes line/column) to stderr; a valid config is unaffected.

### 1.3 Fix the empty-`notification_type` filter bypass *(main.rs)*
The filter at main.rs:43–47 is `if !notification_type.is_empty() && !config.notify_types.iter().any(...)`. An empty/missing `notification_type` therefore **bypasses the allowlist** and always fires focus+notify.

- **Change:** Drop the `!notification_type.is_empty() &&` guard so an empty type simply fails the allowlist check and is ignored (consistent with "only act on configured types").
- **Acceptance:** A payload with empty/missing `notification_type` produces no action unless an empty entry is explicitly in `notify_types`.

### 1.4 Honor `XDG_CONFIG_HOME` *(config.rs + install.sh)*
`config_path()` (config.rs:72–78) hardcodes `$HOME/.config`; install.sh:8 does the same. On non-default XDG setups the binary reads a different file than the user edits.

- **Change:** Read `XDG_CONFIG_HOME` (treated as set only when **non-empty**, per the XDG spec — Rust's `env::var` returns `Ok("")` for an empty var, so guard against it) with a `$HOME/.config` fallback in both the binary and the installer.
- **Acceptance:** With `XDG_CONFIG_HOME` set to a real dir, both agree on that location; with `XDG_CONFIG_HOME=""` (empty), both fall back to `$HOME/.config`.

### 1.5 argv dispatch + `test` / `doctor` / `--version` *(main.rs, new module)*
`run()` unconditionally reads stdin (main.rs:33–34), making the binary a write-only black box. Add an argv branch in `main`:

- **No args (or explicit `hook`)** → today's read-stdin path. **The default hook contract is unchanged.**
- **`--version` / `-V`** → print `env!("CARGO_PKG_VERSION")`.
- **`test [type]`** → an explicit diagnostic that **bypasses the `notify_types` allowlist** (it is a manual check, not a hook event) and exercises the real focus+notify code, printing what it does. With no type, loop over all four payload types — `permission_prompt`, `idle_prompt`, `elicitation_dialog`, `auth_success` — annotating which are filtered out of the *default* config (`auth_success` is silent by default). It also **performs the focus leg**, which raises/borders the *invoking* terminal (the `/proc` walk in `process_tree.rs` starts from the test process itself) — this is intentional and is how you confirm focus actually works. Replaces the brittle hand-typed `echo | claude-focus` snippets in the README.
- **`doctor`** → a PASS/WARN/FAIL health check of every independent leg. It must **never crash** — e.g. an unparseable `settings.json` is reported as a FAIL leg, not a panic (the Phase 2.1 hardening doesn't exist yet, so `doctor` must be robust on its own). Each failure prints the exact fix command. Legs and their concrete probes:
  - **config parses** — `toml::from_str` succeeds; if not, print the precise `toml::de::Error` (this is the only "config validate" we need)
  - **deps on `PATH`** — `notify-send`, `pw-play`, `gdbus`
  - **sound file** — `sound_file` exists when `play_sound = true`
  - **D-Bus name owned** — `gdbus introspect --session --dest org.gnome.Shell.Extensions.FocusByPid --object-path /org/gnome/Shell/Extensions/FocusByPid` exits 0; else FAIL with fix "log out/in or restart GNOME Shell". The #1 "it doesn't work yet" cause before logout, which the README cannot check. Depends on the `gdbus`-on-PATH leg (a missing `gdbus` reports once, not twice).
  - **extension enabled** — the UUID `focus-by-pid@claude.local` appears in `gnome-extensions list --enabled`
  - **hook registered** — some `hooks.Notification[].hooks[]` entry in `~/.claude/settings.json` has a `command` matching `claude-focus` by the same equality `install.sh:65–68` uses (so installer and doctor never disagree)
- **Acceptance:** `claude-focus` with no args behaves exactly as today. `claude-focus test idle_prompt` fires one notification and highlights the invoking terminal regardless of `notify_types`; no-arg `test` reports all four types with their default-filtered status. `claude-focus doctor` reports each leg's PASS/WARN/FAIL with an actionable fix line and never crashes on malformed input. `claude-focus --version` prints the crate version.

**Phase 1 dependencies:** none. 1.5's argv dispatch is the backbone that 2.x (install runs `doctor`) and 3.x build on.

**Phase 1 scope guard:** Phase 1 does **not** modify the sound-emission path in `notify.rs` — the double-play and the README `paplay` drift are owned by 2.7, even though `test`/`doctor` will surface them.

---

## Phase 2 — Trustworthy install + clearer notifications

**Goal:** First-run installation verifies itself, never half-installs, and notifications carry enough context to triage which session needs attention.

### 2.1 Guard + atomically write the settings.json merge *(install.sh)*
install.sh runs under `set -euo pipefail` and does a bare `json.load` of `~/.claude/settings.json` (lines 36–77, load at 43–44) **after** copying the binary and extension. A malformed/hand-edited settings file aborts the install mid-way, leaving no hook and a damaged-trust user.

- **Change:** Wrap the load in `try/except` with a clear message ("your settings.json is not valid JSON — fix or back it up"); write atomically (temp file + `os.replace`) so a crash never truncates the user's Claude settings.
- **Acceptance:** Installing against a deliberately-malformed settings.json fails loudly with guidance and leaves the original file intact.

### 2.2 Dependency preflight *(install.sh)*
Today install.sh checks only `gnome-extensions` (line 80).

- **Change:** Before building, `command -v` for `cargo` (hard fail → rustup link), `notify-send`, `gdbus`, `pw-play` (warn → apt package name), and warn if not GNOME/Wayland.
- **Acceptance:** Missing dependencies are reported up front with the correct package names, not discovered after the hook is wired and nothing happens.

### 2.3 Run `doctor` at the end of install.sh *(install.sh)*
- **Change:** After all steps, exec `~/.local/bin/claude-focus doctor` so the final output is a green/red checklist confirming end-to-end function (including the conditional logout requirement), replacing the static "Installation complete!" (line 88).
- **Acceptance:** A fresh install ends with the `doctor` report; a broken leg (e.g. missing notify-send) shows as FAIL rather than a misleading success banner.
- **Depends on:** Phase 1.5 (`doctor`).

### 2.4 Show the project name in the notification *(main.rs → notify.rs)*
`cwd` is parsed into `HookInput` (main.rs:16) but never passed to `send_notification`.

- **Change:** Thread `cwd` through and append its basename to the **title** (body acceptable as a fallback), e.g. `Claude Code — Permission Required · claude-focus`.
- **Acceptance:** With two sessions in different projects, each notification's title contains its project basename. Highest impact-per-line in the repo.

### 2.5 Respect Do Not Disturb / quiet hours *(notify.rs + config.rs)*
The README currently tells users to turn DND **off** (a bad-citizen workaround).

- **Change:** Before spawning notify-send/pw-play, read GNOME DND state (`gsettings get org.gnome.desktop.notifications show-banners`) and an optional `quiet_hours` config window; when active, suppress banner+sound. A config knob decides whether silent auto-focus still runs. The `gsettings` spawn is justified against ethos #2 as a standard GNOME tool already implied by the GNOME-extension dependency. It is **GNOME-specific** and must degrade to "no DND info → notify normally" when `gsettings`/GNOME is absent; when the Phase 4/5 `Focuser` backends land, this read should become backend-aware (or remain a best-effort GNOME-only gate) so it doesn't misread DND on X11/non-GNOME sessions.
- **Acceptance:** With GNOME DND on, no banner/sound fires; with no GNOME/`gsettings` present, notifications fire normally (no crash, no false suppression); focus behavior follows the configured knob. Treated as a correctness fix (stop fighting the OS), not a feature.

### 2.6 Per-type urgency *(notify.rs)*
notify.rs:34 hardcodes `--urgency normal` for all types.

- **Change:** `critical` for `permission_prompt` (cuts through, persists), `normal` for `idle_prompt`/others. (Optional, low priority: per-type sound override.)
- **Acceptance:** A permission prompt produces a persistent/critical notification; an idle prompt a normal one.

### 2.7 Stop sound double-play + fix README paplay drift *(notify.rs + README)*
notify.rs adds a `sound-file` hint to notify-send (lines 39–45) **and** separately spawns `pw-play` (lines 57–66); on daemons that honor the hint, `play_sound = true` plays twice. The README also references a `paplay` fallback at lines 28, 47, 71, and 280 that no code path implements.

- **Change:** Drop the hint path (keep the explicit `pw-play` that the README documents). **Decision for the planner to make and record:** either remove all four `paplay` README references, or implement a real `paplay` fallback when `pw-play` is absent — pick one and make code and docs agree.
- **Acceptance:** `play_sound = true` plays exactly once; `grep -n paplay README.md` matches the chosen path (empty if references removed, or every remaining reference backed by a real code path).

---

## Phase 3 — Lock it in

**Goal:** A regression net exists before the Phase 4 refactor, and the installed build is identifiable.

### 3.1 CI *(new `.github/workflows/`)*
- **Change:** A push/PR GitHub Actions workflow: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo build --release`, and a smoke test that pipes a sample `permission_prompt` payload and asserts exit 0. Optionally `shellcheck install.sh`.
- **Acceptance:** CI runs on PRs and fails on fmt/clippy/build/smoke-test violations.

### 3.2 Unit tests for the regression-prone logic
- **Change:** Table-driven tests for: the `notify_types` filter incl. the now-fixed empty-type case (main.rs:43), the 15-char-truncated terminal-name match in process_tree.rs (lines 49–51, e.g. `gnome-terminal-` → `gnome-terminal-server`), tmux name detection (line 56), and config parse-error handling (config.rs:67).
- **Acceptance:** Tests cover each fixed footgun and fail if the fix is reverted.

### 3.3 Single source of version truth *(Cargo.toml, metadata.json, --version)*
Three uncoordinated versions exist (Cargo.toml semver `0.1.0`, metadata.json GNOME-mandated integer `1`, no runtime flag). They use **incompatible schemes** (GNOME rejects non-integer extension versions), so string-equality is not the goal.

- **Change:** `--version` (from 1.5) prints `CARGO_PKG_VERSION`. For the manifests, assert a **documented mapping** rather than equality: `metadata.json`'s integer `version` is the extension revision and bumps when `extension.js` changes. (If a clean mapping isn't worth it, narrow 3.3 to just the `--version` flag and move the cross-manifest check to the Parking Lot.)
- **Acceptance:** `claude-focus --version` prints the crate version; the documented version-bump rule is written down (and CI-checked if a mapping is adopted).

---

## Phase 4 — Right window, every time

**Goal:** The correct terminal is focused even with multiple concurrent sessions, redundant alerts are suppressed, and the focus path is abstracted so Phase 5 can add platforms cleanly.

### 4.1 Introduce a `Focuser` trait + graceful fallback *(new module; refactor main.rs, dbus.rs)*
Today main.rs:52–56 calls `find_terminal_pid()` then `dbus::highlight_window()` directly; main.rs:54 calls `dbus::highlight_window()`, which spawns `gdbus` (dbus.rs:7) even when the extension isn't loaded — failing silently into the void. (Phase 1.5's `doctor` already *reports* the unowned-D-Bus-name case; 4.1 is what stops the binary from silently *attempting* the call — the two are complementary.)

- **Change:** Extract the focus path behind `Focuser { fn focus(&self, target: &FocusTarget) -> FocusOutcome }`, with `FocusTarget { pid, session_id, title }`. `dbus.rs` becomes `GnomeWaylandFocuser`. `detect_focuser()` picks a backend at runtime from the environment (`XDG_SESSION_TYPE`, `WAYLAND_DISPLAY`, compositor sockets) and returns a `NoopFocuser` (still notifies) when nothing usable is found.
- **Acceptance:** Behavior on GNOME/Wayland is unchanged; on an unsupported environment the tool notifies and does not attempt (and silently fail) a gdbus call. The refactor is covered by Phase 3 tests.
- **Depends on:** Phase 3 (regression tests protect the 4.1 refactor).

### 4.2 Disambiguate multi-window terminals via `session_id` title-tag
`gnome-terminal-server` shares one PID across all its windows, so `_findBestWindowByPid` (extension.js:101–114) sorts by `get_user_time()` and can light up the wrong window. `session_id` is in every payload but currently dead code.

- **Change:** Tag the terminal title with the session id, then match it in the extension. The hook binary runs at notification time on a transient stdin pipe and has no stable handle to the interactive tty, so tagging must happen at shell init: ship a small **shell rc snippet** (installed by `install.sh`, with a `doctor` check that the tag is present) that emits an OSC title escape carrying the Claude `session_id`. Pass `session_id` through D-Bus to a `HighlightBySession`-style path; match `win.get_title()` in the extension. **Guaranteed fallback:** when no title tag is found, fall back to today's PID + `get_user_time` ordering, so the phase ships even for users who haven't installed the snippet. `FocusTarget.{title, session_id}` carry it so any backend can match.
- **Acceptance:** With 2+ gnome-terminal windows running different tagged sessions, the window whose `session_id` matches the payload is raised; with no tag present, behavior falls back to the current PID-based selection (no regression). Acknowledged cost: a per-shell rc setup step.

### 4.3 Skip the redundant alert when the Claude window is already focused
Two independently-shippable parts, because they have very different costs:

- **(a) Extension-side no-op (cheap, stays fire-and-forget):** In `HighlightByPid`, compare the resolved target against `global.display.get_focus_window()`; if it's already focused, no-op the border/raise. The D-Bus return value is still ignored — no change to the fire-and-forget model.
- **(b) Rust-side notify suppression (has a cost — defer/optional):** Suppressing notify-send/pw-play requires knowing the focus state *before* notifying, which means a **synchronous** gdbus round-trip (spawn+wait+parse) — conflicting with ethos #1 (fire-and-forget) and the Background premise that the gdbus call is off the critical path. **Default: do not ship (b).** Only adopt it as an explicit, documented ethos exception (e.g. a bounded, Wayland-only sync call) if the redundant-banner annoyance proves worth it in practice.
- **Acceptance:** (a) Typing in the Claude terminal and hitting an `idle_prompt` produces no border/raise flash for the window you're already in, with the focus call still fire-and-forget. (b) is out of scope unless explicitly justified; if adopted, the banner/sound are also suppressed via a bounded sync call.

---

## Phase 5 — Reach (X11)

**Goal:** Works on X11 desktops with no shipped extension.

### 5.1 X11 backend via wmctrl/xdotool *(new backend behind the Phase 4 trait)*
- **Change:** An `X11Focuser` that takes the terminal PID and runs `wmctrl -l -p` + `wmctrl -i -a <id>` (or `xdotool search --pid <pid> windowactivate`), reusing `find_terminal_pid()` verbatim. Pure fire-and-forget spawn like `dbus.rs`. Selected by `detect_focuser()` on X11. `wmctrl`/`xdotool` are optional deps gated behind X11 detection; without them the tool degrades to notify-only.
- **Acceptance:** On an X11 session (XFCE/MATE/i3/Cinnamon/KDE-on-X11/older GNOME), a hook event raises the terminal with no GNOME extension installed; on X11 without wmctrl/xdotool it notifies and reports the missing dep via `doctor`.
- **Depends on:** Phase 4.1 (`Focuser` trait).

---

## Parking lot (explicitly deferred)

Documented so the decision to *not* build these is intentional and revisitable, not forgotten.

| Idea | Why deferred |
|---|---|
| **Phone push (ntfy.sh / Pushover / Telegram)** | The one genuinely-missing capability — away-from-machine alerts — but it adds a network dependency and a third-party service against the zero-extra-deps ethos. **Most likely future promotion: a clearly-labeled opt-in Phase 6.** |
| GNOME panel/tray indicator listing waiting sessions | Persistent shell-side state + a register/clear lifecycle; contradicts the no-daemon/fire-and-forget ethos. |
| "Jump to next waiting session" method + keybinding | Depends on the cut indicator's session list; stateful cycling for a niche flow. |
| Wire the `Stop` hook to dismiss a resolved session | Only useful once there is standing "waiting" state to clear. |
| macOS backend (osascript / terminal-notifier) | No `/proc`, pervasive `std::os::unix` usage — a real port, not a thin backend; unproven demand. The `Focuser` trait leaves the door open. |
| KDE/KWin-Wayland backend | Needs a registered KWin script (≈ the GNOME-extension effort again) for a smaller audience; X11 backend already covers KDE-on-X11. |
| Clickable "Jump to session" notification action | notify-send actions require the process to stay alive to receive the signal, breaking fire-and-forget. |
| Multi-monitor off-screen arrow / full-edge flash | Cosmetic; the unconditional-raise fix already brings the window forward. |
| Opt-in JSONL decision log + tail | Introduces a state dir + rotation + a second read surface; the config-warn eprintln + `doctor` already answer "why did nothing happen." |
| Full `config` subcommand (show/path/edit/validate) | `validate` folds into `doctor`; the rest is trivial shell. CLI surface the tool hasn't earned. |
| `--explain` dry-run trace | Redundant with `doctor` + `test` + the new config-warn stderr. |
| Per-type border color / per-type mode tables | Cosmetic / L-effort config surface; per-type urgency already captures the real "know what it is without looking" need. |
| Prebuilt release tarballs + dual-mode installer | A tag-triggered build pipeline is real maintenance; the cargo-build path works. Revisit when non-cloner install demand appears. |

---

## Open question carried forward

- **Phone push** is parked per the design discussion. Promote to an opt-in **Phase 6** at any time; it is the single capability with no on-machine substitute.

## What gets planned first

**Phase 1** is the next slice. `writing-plans` should turn Phase 1 (sections 1.1–1.5) into a detailed implementation plan. Later phases are scoped here at the "what + why + acceptance + dependencies" level and will each get their own plan when they come up.
