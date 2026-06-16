# Phase 2 — Trustworthy install + clearer notifications — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** First-run installation verifies itself and never half-installs; desktop notifications carry enough context (which project, what urgency) to triage which Claude session needs attention, and stop fighting the OS (Do Not Disturb) or playing sound twice.

**Architecture:** Two disjoint surfaces. (1) **Rust** (`src/notify.rs`, `src/config.rs`, `src/main.rs`): thread `cwd` into the notification title, map per-type urgency, gate the *noisy* legs (banner + sound) behind GNOME Do-Not-Disturb / a `quiet_hours` window while leaving auto-focus untouched, and remove the double-play sound-hint path. (2) **Bash** (`scripts/install.sh` + `tests/test_update_flow.sh`): guard + atomically write the `settings.json` merge, preflight dependencies with actionable messages, and end the full install by running `claude-focus doctor` so the install self-verifies. All pure decision logic is extracted into small functions with unit tests; the impure shell-outs (`gsettings`, `date`) are thin wrappers around the tested pure core.

**Tech Stack:** Rust (std only — no new crates), `serde`/`toml` (already present), Bash + Python3 heredoc (install.sh), the dependency-free `tests/test_update_flow.sh` harness.

**Decisions locked in (recorded per spec §2.5 and §2.7 which defer to the planner):**
- **Branch (process):** Merge PR #35 (`dx-phase1` → `staging`) into `staging` first, then branch `dx-phase2` off `staging` and PR into `staging`. (Phase 2 needs Phase 1's `doctor` for item 2.3.)
- **§2.7 sound:** *Remove* the `paplay` references. Drop the `notify-send` `--hint sound-file` path entirely (this is also what fixes the double-play) and keep only the explicit `pw-play` spawn. Delete all `paplay` / PulseAudio-fallback mentions from `README.md` so code and docs agree.
- **§2.5 DND:** *Always focus, silence noise only.* DND / `quiet_hours` suppress **banner + sound** only; the window raise (focus) always fires. **No focus knob.** Degrades to "notify normally" when `gsettings`/GNOME is absent. `claude-focus test` bypasses the silence gate so diagnostics always show a banner.

**Spec:** `docs/superpowers/specs/2026-06-05-claude-focus-dx-roadmap-design.md` §2 (on branch `dx-roadmap`). **Issue:** #30 (part of #28).

---

## Pre-flight (process, before any code)

- [ ] **P1 — Land the branch.** Confirm PR #35 is review-ready, then merge it into `staging` (method: match the `#34` precedent — squash — unless the user wants a merge-commit to preserve the per-task CHANGELOG lines). Then:

```bash
git fetch origin
git checkout staging && git pull --ff-only
git checkout -b dx-phase2
git log --oneline -3   # expect Phase 1's work (doctor/test/--version) present
```

Verify Phase 1 is present (item 2.3 depends on it):

```bash
cargo build --release && ./target/release/claude-focus doctor | head -1
# Expected first line: "claude-focus doctor"
```

> The plan doc itself (`docs/superpowers/plans/2026-06-16-claude-focus-phase2.md`) is an untracked working-tree artifact; commit it alongside the first task or onto `dx-roadmap` to match where Phase 1's plan lives — user's choice.

---

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `src/notify.rs` | title (project), urgency, DND/quiet-hours gate, single sound path | A, B, C, D |
| `src/config.rs` | new optional `quiet_hours` field + parse test | D |
| `src/main.rs` | thread `cwd` + `force` through `dispatch` → `send_notification` | B, D |
| `config/claude-focus.toml` | documented `quiet_hours` example (commented) | D |
| `scripts/install.sh` | atomic/guarded settings merge, dep preflight, run doctor | E, F, G |
| `tests/test_update_flow.sh` | sandbox tests for the three install.sh changes | E, F, G |
| `README.md` | drop `paplay`; document DND-respect + `quiet_hours` | C, D |

`send_notification`'s **final** signature (reached incrementally across Tasks B and D), so later tasks are unambiguous:

```rust
pub fn send_notification(
    notification_type: &str,
    message: &str,
    cwd: Option<&str>,
    config: &Config,
    force: bool,   // true => `test` path: bypass the DND/quiet-hours silence gate
)
```

and `main.rs`:

```rust
fn dispatch(notification_type: &str, message: &str, cwd: Option<&str>, config: &config::Config, force: bool)
```

---

## Task A — §2.6 Per-type urgency (`notify.rs`)

**Files:**
- Modify: `src/notify.rs` (the hardcoded `"--urgency", "normal"` at ~line 34; add a `#[cfg(test)]` module — none exists yet)

- [ ] **A1: Write the failing test.** Add to the bottom of `src/notify.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_is_critical() {
        assert_eq!(urgency_for("permission_prompt"), "critical");
    }

    #[test]
    fn other_types_are_normal() {
        assert_eq!(urgency_for("idle_prompt"), "normal");
        assert_eq!(urgency_for("elicitation_dialog"), "normal");
        assert_eq!(urgency_for(""), "normal");
    }
}
```

- [ ] **A2: Run it, verify it fails.**

Run: `cargo test --lib urgency`
Expected: FAIL to **compile** — `cannot find function urgency_for`.

- [ ] **A3: Implement.** Add near the top of `src/notify.rs` (after the `use` lines):

```rust
/// notify-send urgency. `critical` cuts through and persists (GNOME ignores the
/// expire-time for it), used for permission prompts; everything else is `normal`.
fn urgency_for(notification_type: &str) -> &'static str {
    match notification_type {
        "permission_prompt" => "critical",
        _ => "normal",
    }
}
```

Then in `send_notification`, replace the hardcoded urgency. Change:

```rust
    let mut args = vec![
        "--urgency", "normal",
        "--expire-time", &timeout_ms,
        "--app-name", "Claude Code",
    ];
```

to:

```rust
    let urgency = urgency_for(notification_type);
    let mut args = vec![
        "--urgency", urgency,
        "--expire-time", &timeout_ms,
        "--app-name", "Claude Code",
    ];
```

- [ ] **A4: Run tests, verify pass.**

Run: `cargo test --lib`
Expected: PASS (new urgency tests green; existing tests unaffected).

- [ ] **A5: Commit.**

```bash
git add src/notify.rs
git commit -m "feat(notify): per-type urgency — critical for permission prompts"
```

---

## Task B — §2.4 Show the project name in the notification title (`notify.rs` + `main.rs`)

**Files:**
- Modify: `src/notify.rs` (add `title_with_project` + `project_basename`; add `cwd` param; build title as `String`)
- Modify: `src/main.rs` (`dispatch` gains a `cwd` param; `run` passes `hook_input.cwd`; `run_test` passes the current dir)

- [ ] **B1: Write the failing tests.** Add to `mod tests` in `src/notify.rs`:

```rust
    #[test]
    fn title_includes_project_basename() {
        assert_eq!(
            title_with_project("Claude Code — Permission Required", Some("/home/u/git_repos/claude-focus")),
            "Claude Code — Permission Required · claude-focus"
        );
    }

    #[test]
    fn title_unchanged_without_cwd() {
        assert_eq!(title_with_project("Claude Code", None), "Claude Code");
        assert_eq!(title_with_project("Claude Code", Some("")), "Claude Code");
        assert_eq!(title_with_project("Claude Code", Some("/")), "Claude Code");
    }
```

- [ ] **B2: Run, verify fail.**

Run: `cargo test --lib title`
Expected: FAIL to compile — `cannot find function title_with_project`.

- [ ] **B3: Implement the helpers** in `src/notify.rs` (near `urgency_for`):

```rust
/// Final path component of `cwd` (the project dir), or None when empty / root.
fn project_basename(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
}

/// Append the project name to the title so concurrent sessions are
/// distinguishable, e.g. "Claude Code — Permission Required · claude-focus".
/// A None / empty / root cwd yields the base title unchanged.
fn title_with_project(base: &str, cwd: Option<&str>) -> String {
    match cwd.and_then(project_basename) {
        Some(name) => format!("{base} · {name}"),
        None => base.to_string(),
    }
}
```

- [ ] **B4: Thread `cwd` into `send_notification`.** Change its signature and build the title as an owned `String`. The match that currently yields `let title = match ... { ... };` becomes `base_title`, then:

```rust
pub fn send_notification(
    notification_type: &str,
    message: &str,
    cwd: Option<&str>,
    config: &Config,
) {
    let base_title = match notification_type {
        "permission_prompt" => "Claude Code — Permission Required",
        "idle_prompt" => "Claude Code — Ready for Input",
        "elicitation_dialog" => "Claude Code — Question",
        "auth_success" => "Claude Code — Authenticated",
        _ => "Claude Code",
    };
    let title = title_with_project(base_title, cwd);
```

Then where the args are finalized, push `&title` (it now outlives the `args` vec because it is declared above):

```rust
    args.push(&title);
    args.push(body);
```

- [ ] **B5: Thread `cwd` through `dispatch` in `src/main.rs`.** Change the signature and the `send_notification` call:

```rust
fn dispatch(notification_type: &str, message: &str, cwd: Option<&str>, config: &config::Config, force: bool) {
    let should_focus = force || config.mode == Mode::Both || config.mode == Mode::FocusOnly;
    let should_notify = force || config.mode == Mode::Both || config.mode == Mode::NotifyOnly;

    if should_focus {
        if let Some(pid) = process_tree::find_terminal_pid() {
            dbus::highlight_window(pid, config.notification_timeout_ms);
        }
    }
    if should_notify {
        notify::send_notification(notification_type, message, cwd, config);
    }
}
```

Update `run()` to pass the hook's cwd:

```rust
    dispatch(notification_type, message, hook_input.cwd.as_deref(), &config, false);
```

Update `run_test()` to pass the invoking terminal's working directory (so the diagnostic shows the feature). Replace the loop's `dispatch(&ty, "", &config, true);` with:

```rust
    let cwd = std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned());
    let plan = test_plan(which, &config.notify_types);
    for (ty, in_allowlist) in plan {
        let note = if in_allowlist {
            ""
        } else {
            "  (not in your notify_types — forcing anyway)"
        };
        println!("→ firing {ty}{note}");
        dispatch(&ty, "", cwd.as_deref(), &config, true);
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
```

(Move the existing `let plan = ...` up so `cwd` is computed once before the loop.)

- [ ] **B6: Run, verify pass.**

Run: `cargo test --lib && cargo build`
Expected: PASS + clean build (all `dispatch`/`send_notification` callers updated).

- [ ] **B7: Commit.**

```bash
git add src/notify.rs src/main.rs
git commit -m "feat(notify): show project name in notification title"
```

---

## Task C — §2.7 Stop sound double-play + remove `paplay` docs drift (`notify.rs` + `README.md`)

**Decision (recorded):** remove the `notify-send` sound-hint path (the cause of the double-play on hint-honoring daemons) and keep only the explicit `pw-play` spawn; delete every `paplay`/PulseAudio-fallback mention from the README.

**Files:**
- Modify: `src/notify.rs` (delete the `sound_hint` block)
- Modify: `README.md` (remove all `paplay` references)

- [ ] **C1: Delete the sound-hint block in `src/notify.rs`.** Remove these lines entirely:

```rust
    let sound_hint;
    if config.play_sound {
        if let Some(ref sound_file) = config.sound_file {
            sound_hint = format!("string:sound-file:{sound_file}");
            args.extend_from_slice(&["--hint", &sound_hint]);
        }
    }
```

Leave the explicit `pw-play` spawn (the `if config.play_sound { if let Some(ref sound_file) ... pw-play ... }` block) untouched — it is now the single sound path.

- [ ] **C2: Verify the hint path is gone and pw-play remains.**

Run: `grep -n 'sound-file\|--hint' src/notify.rs ; echo "---" ; grep -n 'pw-play' src/notify.rs`
Expected: first grep prints **nothing**; second still shows the `pw-play` spawn.

- [ ] **C3: Remove `paplay` from `README.md`.** Find every occurrence and rewrite so code and docs agree (pw-play only):

Run first: `grep -n 'paplay\|PulseAudio\|pulseaudio' README.md`

Then apply these edits (line numbers approximate — match on text):
- Flow line: `→ plays sound via pw-play (PipeWire) or paplay (PulseAudio)` → `→ plays sound via pw-play (PipeWire)`
- Architecture ASCII box cell that reads `pw-play /` over `paplay` → collapse to a single `pw-play` label (keep the box border alignment; the cell becomes just `pw-play` / `(sound)`).
- Requirements bullet `- **pw-play** (PipeWire) or **paplay** (PulseAudio) for sound alerts` → `- **pw-play** (PipeWire) for sound alerts`
- Troubleshooting: `If pw-play isn't found, install PipeWire tools or PulseAudio (sudo apt install pulseaudio-utils)` → `If pw-play isn't found, install PipeWire tools (sudo apt install pipewire-bin)`
- Project-structure comment `# notify-send + pw-play/paplay` → `# notify-send + pw-play`

- [ ] **C4: Verify the docs are clean.**

Run: `grep -rn 'paplay' README.md ; echo "exit=$?"`
Expected: no matches (`grep` exit 1; "exit=1").

- [ ] **C5: Manual sound check (record result).** Build, set `play_sound = true` in your real config, DND off, then:

Run: `cargo build --release && ./target/release/claude-focus test idle_prompt`
Expected: the alert sound plays **exactly once** per fired type (no double beep).

- [ ] **C6: Commit.**

```bash
git add src/notify.rs README.md
git commit -m "fix(notify): stop sound double-play; drop paplay docs drift"
```

---

## Task D — §2.5 Respect Do Not Disturb / quiet hours (`config.rs` + `notify.rs` + docs)

**Behaviour (locked):** suppress **banner + sound** when GNOME DND is on OR the local time is inside an optional `quiet_hours` window. **Auto-focus is never gated** (it lives in `main.rs::dispatch`, which this task does not touch beyond passing `force`). `claude-focus test` (`force = true`) bypasses the gate so diagnostics always show a banner. Absent `gsettings`/GNOME ⇒ DND reads as off (notify normally — never false-suppress).

**Files:**
- Modify: `src/config.rs` (add `quiet_hours: Option<String>`)
- Modify: `src/notify.rs` (pure `parse_hm` + `in_quiet_hours`; impure `gnome_dnd_active`, `now_minutes`, `notifications_silenced`; add `force` param + early return)
- Modify: `src/main.rs` (pass `force` into `send_notification`)
- Modify: `config/claude-focus.toml` (commented example)
- Modify: `README.md` (DND-respect troubleshooting + `quiet_hours` option row)

### D-config — add the field

- [ ] **D1: Write the failing config tests.** Add to `mod tests` in `src/config.rs`:

```rust
    #[test]
    fn quiet_hours_parses_when_present() {
        let cfg = parse_config_or_default("quiet_hours = \"22:00-08:00\"\n");
        assert_eq!(cfg.quiet_hours.as_deref(), Some("22:00-08:00"));
    }

    #[test]
    fn quiet_hours_defaults_to_none() {
        let cfg = parse_config_or_default("");
        assert_eq!(cfg.quiet_hours, None);
    }
```

- [ ] **D2: Run, verify fail.**

Run: `cargo test --lib quiet_hours`
Expected: FAIL to compile — `no field quiet_hours on type Config`.

- [ ] **D3: Add the field.** In the `Config` struct in `src/config.rs`, after `sound_file`:

```rust
    #[serde(default)]
    pub quiet_hours: Option<String>,
```

And in `impl Default for Config`, add `quiet_hours: None,` to the constructed struct.

- [ ] **D4: Run, verify pass.**

Run: `cargo test --lib quiet_hours`
Expected: PASS.

### D-logic — pure quiet-hours core in `notify.rs`

- [ ] **D5: Write the failing tests.** Add to `mod tests` in `src/notify.rs`:

```rust
    #[test]
    fn parse_hm_basic() {
        assert_eq!(parse_hm("00:00"), Some(0));
        assert_eq!(parse_hm("09:30"), Some(570));
        assert_eq!(parse_hm("23:59"), Some(1439));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("12:60"), None);
        assert_eq!(parse_hm("bad"), None);
        assert_eq!(parse_hm("12"), None);
    }

    #[test]
    fn quiet_hours_none_is_never_quiet() {
        assert!(!in_quiet_hours(None, 0));
        assert!(!in_quiet_hours(None, 720));
    }

    #[test]
    fn quiet_hours_simple_window() {
        // 09:00-17:00 -> minutes 540..1020 (end exclusive)
        assert!(!in_quiet_hours(Some("09:00-17:00"), 539));
        assert!(in_quiet_hours(Some("09:00-17:00"), 540));
        assert!(in_quiet_hours(Some("09:00-17:00"), 1019));
        assert!(!in_quiet_hours(Some("09:00-17:00"), 1020));
    }

    #[test]
    fn quiet_hours_wraps_midnight() {
        // 22:00-08:00 -> >=1320 OR <480
        assert!(in_quiet_hours(Some("22:00-08:00"), 1320)); // 22:00
        assert!(in_quiet_hours(Some("22:00-08:00"), 0));    // 00:00
        assert!(in_quiet_hours(Some("22:00-08:00"), 479));  // 07:59
        assert!(!in_quiet_hours(Some("22:00-08:00"), 480)); // 08:00
        assert!(!in_quiet_hours(Some("22:00-08:00"), 720)); // noon
    }

    #[test]
    fn malformed_quiet_hours_is_not_quiet() {
        assert!(!in_quiet_hours(Some("nonsense"), 720));
        assert!(!in_quiet_hours(Some("25:00-26:00"), 720));
        assert!(!in_quiet_hours(Some("22:00"), 720)); // no dash
    }
```

- [ ] **D6: Run, verify fail.**

Run: `cargo test --lib quiet_hours`
Expected: FAIL to compile — `cannot find function parse_hm` / `in_quiet_hours`.

- [ ] **D7: Implement the pure core** in `src/notify.rs`:

```rust
/// Parse "HH:MM" into minutes-since-midnight (0..=1439). None if malformed.
fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// True if `now_min` (minutes since midnight) is inside the quiet window
/// `"HH:MM-HH:MM"`. Supports windows that wrap past midnight (e.g.
/// "22:00-08:00"). End is exclusive. A malformed/None window is "no quiet
/// hours" (false) — never silently swallow a bad value into "always quiet".
fn in_quiet_hours(window: Option<&str>, now_min: u32) -> bool {
    let Some(window) = window else {
        return false;
    };
    let Some((start, end)) = window.split_once('-') else {
        return false;
    };
    let (Some(start), Some(end)) = (parse_hm(start), parse_hm(end)) else {
        return false;
    };
    if start <= end {
        now_min >= start && now_min < end
    } else {
        now_min >= start || now_min < end
    }
}
```

- [ ] **D8: Run, verify pass.**

Run: `cargo test --lib quiet_hours`
Expected: PASS.

### D-wire — impure wrappers + gate, then thread `force`

- [ ] **D9: Add the impure wrappers + gate** in `src/notify.rs` (these shell out to standard tools — justified against ethos #2 exactly as `gsettings` is in the spec; not unit-tested, exercised manually in D13):

```rust
/// GNOME "show banners" == false ⇒ Do Not Disturb on. Best-effort: any failure
/// (no gsettings, non-GNOME, parse miss) ⇒ false, i.e. notify normally. Never
/// false-suppress.
fn gnome_dnd_active() -> bool {
    Command::new("gsettings")
        .args(["get", "org.gnome.desktop.notifications", "show-banners"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
        .unwrap_or(false)
}

/// Local wall-clock minutes-since-midnight via `date` (a standard tool — avoids
/// pulling in a timezone crate, consistent with the zero-runtime-deps ethos).
fn now_minutes() -> Option<u32> {
    let out = Command::new("date").arg("+%H:%M").output().ok()?;
    parse_hm(String::from_utf8_lossy(&out.stdout).trim())
}

/// Whether the *noisy* legs (banner + sound) should be suppressed right now:
/// GNOME DND on, OR inside the configured quiet-hours window. Focus is NOT
/// gated by this.
fn notifications_silenced(config: &Config) -> bool {
    if gnome_dnd_active() {
        return true;
    }
    match (config.quiet_hours.as_deref(), now_minutes()) {
        (Some(window), Some(now)) => in_quiet_hours(Some(window), now),
        _ => false,
    }
}
```

- [ ] **D10: Add `force` to `send_notification` + the early return.** Final signature and gate at the top of the body:

```rust
pub fn send_notification(
    notification_type: &str,
    message: &str,
    cwd: Option<&str>,
    config: &Config,
    force: bool,
) {
    // Respect DND / quiet hours for the noisy legs. `test` (force) bypasses so
    // diagnostics always show a banner. Auto-focus is unaffected (it lives in
    // main::dispatch).
    if !force && notifications_silenced(config) {
        return;
    }
    // ... existing body (base_title, title, urgency, args, notify-send, pw-play)
}
```

- [ ] **D11: Pass `force` through `dispatch`** in `src/main.rs` — update the `send_notification` call inside `dispatch`:

```rust
    if should_notify {
        notify::send_notification(notification_type, message, cwd, config, force);
    }
```

(No other change: `run()` already passes `force = false`, `run_test()` passes `force = true`.)

- [ ] **D12: Run all Rust tests + build.**

Run: `cargo test && cargo build`
Expected: PASS + clean build.

- [ ] **D13: Manual DND check (record result).** With your real config:

```bash
cargo build --release
# DND ON:
gsettings set org.gnome.desktop.notifications show-banners false
echo '{"notification_type":"permission_prompt","cwd":"'"$PWD"'","message":"x"}' | ./target/release/claude-focus
#   -> NO banner/sound; window IS raised.
./target/release/claude-focus test permission_prompt
#   -> banner DOES show (test bypasses the gate).
# restore:
gsettings set org.gnome.desktop.notifications show-banners true
```

Expected: hook path silent-but-focuses under DND; `test` still shows the banner; with DND off, the hook shows the banner normally.

### D-docs — config example + README

- [ ] **D14: Document `quiet_hours` in `config/claude-focus.toml`.** Append:

```toml

# Quiet hours: suppress banner + sound during this local-time window
# (auto-focus still runs). Format "HH:MM-HH:MM", may wrap past midnight.
# quiet_hours = "22:00-08:00"
```

- [ ] **D15: Update `README.md`.** (a) In the Configuration "Options" table, add a row after the `sound_file` row:

```
| `quiet_hours` | `"HH:MM-HH:MM"` or unset | unset | Suppress banner + sound during this local-time window (auto-focus still runs); may wrap past midnight |
```

(b) Replace the troubleshooting line `- Check that Do Not Disturb is off in GNOME settings` with:

```
- claude-focus **respects** GNOME Do Not Disturb: while DND is on, banners and sound are suppressed (auto-focus still works). Turn DND off, or check `quiet_hours` in your config, if you expect a banner and see none.
```

- [ ] **D16: Commit.**

```bash
git add src/config.rs src/notify.rs src/main.rs config/claude-focus.toml README.md
git commit -m "feat(notify): respect GNOME DND + quiet_hours for banner and sound"
```

---

## Task E — §2.1 Guard + atomically write the `settings.json` merge (`install.sh`)

**Files:**
- Modify: `scripts/install.sh` (the `do_hook` Python heredoc)
- Modify: `tests/test_update_flow.sh` (two new test blocks)

- [ ] **E1: Write the failing tests.** Add to `tests/test_update_flow.sh` (before the "Makefile" section):

```bash
echo "== install.sh: malformed settings.json fails loudly, leaves file intact =="
make_sandbox
mkdir -p "$(dirname "$SETTINGS_FILE")"
printf '{ this is not valid json ' > "$SETTINGS_FILE"
before="$(cat "$SETTINGS_FILE")"
run_install
assert_eq       "malformed settings aborts nonzero"  "$RC" "1"
assert_contains "malformed settings explains why"    "$OUT" "not valid JSON"
assert_eq       "malformed settings left intact"     "$(cat "$SETTINGS_FILE")" "$before"
rm -rf "$SB"

echo "== install.sh: merges into existing valid settings, preserving keys =="
make_sandbox
mkdir -p "$(dirname "$SETTINGS_FILE")"
printf '{"otherKey": 42}' > "$SETTINGS_FILE"
run_install
assert_eq       "merge exits 0"                "$RC" "0"
assert_contains "merge preserves existing key" "$(cat "$SETTINGS_FILE")" "otherKey"
assert_contains "merge adds hook"              "$(cat "$SETTINGS_FILE")" "$BIN_DIR/claude-focus"
rm -rf "$SB"
```

- [ ] **E2: Run, verify the malformed test fails.**

Run: `bash tests/test_update_flow.sh 2>&1 | grep -A1 'malformed'`
Expected: `FAIL` lines — current `json.load` raises an uncaught `JSONDecodeError`; the message won't contain "not valid JSON" and (depending on Python's traceback) the test for a clean guarded message fails. (The preserve-keys test likely already passes.)

- [ ] **E3: Implement.** Replace the entire `do_hook` Python heredoc body (between `<<'PY'` and `PY`) with:

```python
import json, os, tempfile
settings_file = os.environ['SETTINGS_FILE']
hook_command = os.path.join(os.environ['BIN_DIR'], 'claude-focus')

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

hook_entry = {'matcher': '*', 'hooks': [{'type': 'command', 'command': hook_command}]}
hooks = settings.setdefault('hooks', {})
notifications = hooks.setdefault('Notification', [])
already_present = any(
    any(h.get('command') == hook_command for h in entry.get('hooks', []))
    for entry in notifications
)
if already_present:
    print('    Hook already present, skipping')
else:
    notifications.append(hook_entry)
    # Atomic write: temp file in the same dir + os.replace, so a crash never
    # truncates the user's Claude settings.
    d = os.path.dirname(settings_file) or '.'
    fd, tmp = tempfile.mkstemp(dir=d, prefix='.settings.', suffix='.tmp')
    try:
        with os.fdopen(fd, 'w') as f:
            json.dump(settings, f, indent=2)
        os.replace(tmp, settings_file)
    except BaseException:
        if os.path.exists(tmp):
            os.remove(tmp)
        raise
    print('    Hook added to', settings_file)
```

(`raise SystemExit("...")` prints the message to stderr and exits non-zero; under `set -euo pipefail` that aborts the install. The original file was only *read*, so it stays intact.)

- [ ] **E4: Run, verify pass.**

Run: `bash tests/test_update_flow.sh`
Expected: all PASS (malformed + preserve-keys blocks now green; existing blocks unaffected).

- [ ] **E5: Commit.**

```bash
git add scripts/install.sh tests/test_update_flow.sh
git commit -m "fix(install): guard + atomically write the settings.json merge"
```

---

## Task F — §2.2 Dependency preflight (`install.sh`)

**Scope decision (recorded):** preflight runs at the start of the **full install** (the spec's "before building" flow). `--bin`/`--ext` partial flows are unchanged; a missing `cargo` there still surfaces via cargo's own error. Hard-fail only on `cargo`; everything else warns.

**Files:**
- Modify: `scripts/install.sh` (new `preflight()`; call it in the full-install branch)
- Modify: `tests/test_update_flow.sh` (one new test block)

- [ ] **F1: Write the failing test.** Add to `tests/test_update_flow.sh`:

```bash
echo "== install.sh: preflight hard-fails when cargo is missing =="
make_sandbox
rm -f "$SB/fakebin/cargo"   # remove the fake cargo; controlled PATH hides any real one
OUT="$(PATH="$SB/fakebin:/usr/bin:/bin" PROJECT_DIR="$PROJECT_DIR" BIN_DIR="$BIN_DIR" \
       EXT_DIR="$EXT_DIR" CONFIG_DIR="$CONFIG_DIR" SETTINGS_FILE="$SETTINGS_FILE" \
       bash "$INSTALL" 2>&1)"; RC=$?
assert_eq       "missing cargo exits 1"        "$RC" "1"
assert_contains "missing cargo names rustup"   "$OUT" "rustup"
assert_absent   "missing cargo builds nothing" "$BIN_DIR/claude-focus"
rm -rf "$SB"
```

> Assumption: the dev/CI machine's `cargo` lives in `~/.cargo/bin` (not `/usr/bin`), so the controlled `PATH` hides it. If a real `cargo` is in `/usr/bin`, this test will not see a missing cargo — note it in the run log if so.

- [ ] **F2: Run, verify fail.**

Run: `bash tests/test_update_flow.sh 2>&1 | grep -A2 'cargo is missing'`
Expected: FAIL — today there is no preflight, so it proceeds and either exits 0 or fails later without naming `rustup`.

- [ ] **F3: Implement `preflight()`** in `scripts/install.sh` (add the function above `print_full_summary`):

```bash
preflight() {
    echo "==> Checking dependencies..."
    local missing_hard=0 dep pkg
    if ! command -v cargo &>/dev/null; then
        echo "    [FAIL] cargo not found — install Rust: https://rustup.rs" >&2
        missing_hard=1
    fi
    for dep in notify-send gdbus pw-play; do
        if ! command -v "$dep" &>/dev/null; then
            case "$dep" in
                notify-send) pkg="libnotify-bin" ;;
                gdbus)       pkg="libglib2.0-bin" ;;
                pw-play)     pkg="pipewire-bin (or set play_sound=false)" ;;
            esac
            echo "    [warn] $dep not found — sudo apt install $pkg"
        fi
    done
    if [ "${XDG_SESSION_TYPE:-}" != "wayland" ] || ! printf '%s' "${XDG_CURRENT_DESKTOP:-}" | grep -qi gnome; then
        echo "    [warn] auto-focus needs GNOME/Wayland; desktop notifications still work elsewhere"
    fi
    if [ "$missing_hard" -eq 1 ]; then
        echo "    Aborting: install the hard requirement(s) above and re-run." >&2
        exit 1
    fi
}
```

Then in `main()`, call it at the start of the full-install branch:

```bash
    if [ "$do_all" -eq 1 ]; then
        preflight
        do_build; do_bin; do_ext; do_config; do_hook; do_enable
        print_full_summary
    else
```

- [ ] **F4: Run, verify pass.**

Run: `bash tests/test_update_flow.sh`
Expected: all PASS. (The existing full-install test still exits 0 — fake `cargo` is present, so no hard fail; the warn lines for any absent `notify-send`/`gdbus`/`pw-play`/non-GNOME do not change the exit code.)

- [ ] **F5: Commit.**

```bash
git add scripts/install.sh tests/test_update_flow.sh
git commit -m "feat(install): dependency preflight with actionable messages"
```

---

## Task G — §2.3 Run `doctor` at the end of install (`install.sh`)

**Files:**
- Modify: `tests/test_update_flow.sh` (upgrade the sandbox fake `cargo` to emit an executable stub that handles `doctor`; assert the full install runs doctor)
- Modify: `scripts/install.sh` (`print_full_summary` runs `claude-focus doctor`)

- [ ] **G1: Upgrade the sandbox fake binary.** In `tests/test_update_flow.sh`, in `make_sandbox`, replace the `printf ... > "$SB/fakebin/cargo"` line so fake `cargo` writes an *executable stub* that responds to `doctor`:

```bash
  mkdir -p "$SB/fakebin"
  cat > "$SB/fakebin/cargo" <<'CARGO'
#!/usr/bin/env bash
mkdir -p target/release
cat > target/release/claude-focus <<'BIN'
#!/usr/bin/env bash
[ "$1" = doctor ] && echo "claude-focus doctor (stub) — DOCTOR RAN"
exit 0
BIN
chmod +x target/release/claude-focus
CARGO
  printf '#!/usr/bin/env bash\nexit 0\n' > "$SB/fakebin/gnome-extensions"
  chmod +x "$SB/fakebin/cargo" "$SB/fakebin/gnome-extensions"
```

(Other tests only check the binary's *presence*, so the richer stub is harmless. The `--bin`/`--ext`/combo tests are unaffected.)

- [ ] **G2: Add the assertion to the existing full-install block.** In the `== install.sh: full install (no flags) ==` block, after the existing asserts and before `rm -rf "$SB"`:

```bash
assert_contains "full install runs doctor"          "$OUT" "DOCTOR RAN"
```

- [ ] **G3: Run, verify fail.**

Run: `bash tests/test_update_flow.sh 2>&1 | grep 'runs doctor'`
Expected: FAIL — `print_full_summary` doesn't run the binary yet.

- [ ] **G4: Implement.** Replace `print_full_summary` in `scripts/install.sh` with:

```bash
print_full_summary() {
    echo ""
    echo "  Binary:    $BIN_DIR/claude-focus"
    echo "  Config:    $CONFIG_DIR/config.toml"
    echo "  Extension: $EXT_DIR/"
    echo ""
    echo "==> Verifying install (claude-focus doctor):"
    "$BIN_DIR/claude-focus" doctor || true
}
```

(`|| true`: `doctor` reports legs as PASS/FAIL but always exits 0 in real use; the guard also keeps a non-real stub from aborting the script. The doctor output — including the conditional log-out requirement — replaces the old static "Installation complete!" banner.)

- [ ] **G5: Run, verify pass.**

Run: `bash tests/test_update_flow.sh`
Expected: all PASS, including "full install runs doctor".

- [ ] **G6: Manual end-to-end (record result).** On a real machine:

Run: `./scripts/install.sh | tail -15`
Expected: the install ends with the `claude-focus doctor` checklist (PASS/FAIL legs), not a static success banner.

- [ ] **G7: Commit.**

```bash
git add scripts/install.sh tests/test_update_flow.sh
git commit -m "feat(install): run doctor at end of install to self-verify"
```

---

## Final verification (whole-phase gate, before review/PR)

- [ ] **V1: Format + lint + test (Rust).**

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```
Expected: formatted, zero clippy warnings, all unit tests pass.

- [ ] **V2: Install-flow suite (Bash).**

```bash
bash tests/test_update_flow.sh
```
Expected: `N passed, 0 failed`.

- [ ] **V3: Acceptance greps (code↔docs agreement).**

```bash
grep -rn 'paplay' README.md ; echo "paplay refs exit=$?  (want 1)"
grep -n 'sound-file\|--hint' src/notify.rs ; echo "hint path exit=$?  (want 1)"
```
Expected: both empty (exit 1).

- [ ] **V4: Per-item acceptance checklist** (tick each against spec §2):
  - 2.1 malformed `settings.json` → loud fail + file intact (test + manual)
  - 2.2 missing dep reported up front with package name (test + read output)
  - 2.3 install ends with the `doctor` report (test + manual G6)
  - 2.4 two sessions → each title carries its project basename (manual: fire from two dirs)
  - 2.5 DND on → no banner/sound, window still raised; no GNOME → notify normally (manual D13)
  - 2.6 permission = critical, idle = normal (test A + manual)
  - 2.7 sound plays exactly once; `grep paplay README.md` empty (test C + manual C5)

---

## Execution & review (ultracode)

Implement the tasks **sequentially** in this session (the `notify.rs` changes in Tasks A–D interlock inside one function, so parallel editors would collide — TDD in order keeps full context). After **V1–V4 are green**, run a **multi-agent adversarial review workflow** over the `dx-phase2` diff (this is where ultracode parallelism pays): independent reviewers for (a) Rust correctness incl. the DND gate & wrap-midnight math, (b) `install.sh` safety / atomicity / preflight, (c) spec §2 acceptance coverage per item, (d) DRY/bloat vs existing helpers, (e) README↔code agreement — each finding adversarially verified before it's accepted. Fix confirmed findings, re-run V1–V4, then open the PR into `staging`.

## DRY & anti-bloat note (Step 4 — fill in at execution)
- Reused: `parse_hm` is the single time parser (shared by `in_quiet_hours` and `now_minutes`).
- Promoted: none (no second caller yet — extraction would be premature).
- Kept local: test helpers stay per-file per repo convention; `urgency_for`/`title_with_project`/quiet-hours live in `notify.rs` (single owner).

## Commit / release impact (Step 5)
Per-item conventional commits (release-please reads them): `feat(notify):` ×3 (2.6, 2.4, 2.5), `fix(notify):` (2.7), `fix(install):` (2.1), `feat(install):` ×2 (2.2, 2.3). Net bump: **minor** (pre-1.0). Do **not** touch version literals — release-please owns those.
