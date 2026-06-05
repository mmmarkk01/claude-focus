# Phase 1 — "Stop the lies + make it pokeable" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the one core focus bug and the silent-failure footguns, and turn the write-only hook binary into a self-testable, self-diagnosing tool via `test` / `doctor` / `--version`.

**Architecture:** Small, surgical changes to the existing Rust crate plus one GNOME-extension fix. The recurring move is to extract pure, testable seams (`should_act`, `config_path_from`, `parse_config_or_default`, `parse_args`, `hook_registered`, `test_plan`) so the new logic is unit-tested with plain `cargo test` (no new harness — Phase 3 adds CI around these). Side-effecting legs (window raise, notification firing, D-Bus/PATH probes) get explicit manual-verification steps.

**Tech Stack:** Rust 2021 (serde, serde_json, toml), GJS (GNOME Shell extension), bash. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-06-05-claude-focus-dx-roadmap-design.md` (Phase 1, §1.1–1.5).

**Scope guard:** Do **not** touch the sound-emission path in `notify.rs` (double-play / `paplay` drift are owned by Phase 2.7), even though `test`/`doctor` will surface them.

---

## File structure

| File | Responsibility | Change |
|---|---|---|
| `extension/extension.js` | GNOME window activation | Modify `HighlightByPid` to always raise (§1.1) |
| `src/config.rs` | Config load + path resolution | Add `parse_config_or_default`, `config_path_from` seams (§1.2, §1.4) |
| `src/main.rs` | Dispatch + new subcommands | Add `should_act`, `dispatch`, `parse_args`, `Command`, `run_test`, `run_doctor` (§1.3, §1.5) |
| `src/doctor.rs` | `doctor` health checks | **Create** — leg probes incl. tolerant `hook_registered` (§1.5) |
| `scripts/install.sh` | Installer | Honor `XDG_CONFIG_HOME` (§1.4) |

Each task ends in a commit. Run `cargo test` and `cargo build --release` before every commit touching Rust.

---

## Task 1: Always raise the window (§1.1, extension.js)

**Files:**
- Modify: `extension/extension.js:85-99` (`HighlightByPid`)

This is GNOME-Shell JS with no unit-test harness in the repo, so it is verified manually via a direct D-Bus call.

- [ ] **Step 1: Make the activation unconditional**

In `HighlightByPid`, move `Main.activateWindow(win)` out of the cross-workspace branch so it always runs:

```js
    HighlightByPid(pid, duration_ms) {
        const win = this._findBestWindowByPid(pid);
        if (!win) return false;

        const workspace = win.get_workspace();
        const activeWorkspace = global.workspace_manager.get_active_workspace();

        // Switch workspace first only if the window lives on another one...
        if (workspace && workspace !== activeWorkspace) {
            workspace.activate(global.get_current_time());
        }
        // ...then ALWAYS raise/activate it. Previously this was inside the
        // cross-workspace branch, so a same-workspace window got a border but
        // was never raised (contradicting the README). ActivateByPid already
        // calls activateWindow unconditionally — this mirrors it.
        Main.activateWindow(win);

        this._highlightWindow(win, duration_ms || 3000);
        return true;
    }
```

- [ ] **Step 2: Install the updated extension and reload GNOME Shell**

```bash
cp extension/extension.js "$HOME/.local/share/gnome-shell/extensions/focus-by-pid@claude.local/"
```
On Wayland, log out/in (or restart GNOME Shell) to reload. On X11: `Alt+F2`, type `r`, Enter.

- [ ] **Step 3: Verify a same-workspace window is actually raised**

Open two terminal windows on the **same** workspace. Note the PID of the *background* one:
```bash
# in the window you want raised:
echo "my pid is $$"   # note this number, call it PID
```
From the **other** (foreground) window, call the extension directly:
```bash
gdbus call --session \
  --dest org.gnome.Shell.Extensions.FocusByPid \
  --object-path /org/gnome/Shell/Extensions/FocusByPid \
  --method org.gnome.Shell.Extensions.FocusByPid.HighlightByPid PID 3000
```
Expected: the background terminal is **raised to the foreground** (not merely bordered green). Before this fix it would only get the border.

- [ ] **Step 4: Commit**

```bash
git add extension/extension.js
git commit -m "fix(extension): always raise the window in HighlightByPid

Same-workspace windows previously got a border but were never raised,
contradicting the README. Mirrors ActivateByPid's unconditional activate."
```

---

## Task 2: Warn instead of silently wiping config (§1.2, config.rs)

**Files:**
- Modify: `src/config.rs` (`load_config`, add `parse_config_or_default`, add `#[cfg(test)]`)

- [ ] **Step 1: Write the failing tests**

Add to the bottom of `src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_config_parses() {
        let cfg = parse_config_or_default("mode = \"notify-only\"\n");
        assert_eq!(cfg.mode, Mode::NotifyOnly);
    }

    #[test]
    fn invalid_config_falls_back_to_defaults() {
        // A broken table header must yield defaults, never a panic.
        let cfg = parse_config_or_default("mode = \"notify-only\"\n[ broken");
        assert_eq!(cfg.mode, Mode::default()); // Mode::Both
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test 2>&1 | tail -20`
Expected: FAIL — `cannot find function `parse_config_or_default`` (the compile error aborts the whole test build). Note: this is a binary-only crate, so `cargo test` (no `--lib`) is the only correct form, and a name filter would match test-function names, not the function under test.

- [ ] **Step 3: Implement the seam and use it in `load_config`**

In `src/config.rs`, add the function and route `load_config` through it:

```rust
/// Parse config from a TOML string. On a parse error, print the error (which
/// carries line/column) to stderr and fall back to defaults — never panic,
/// never silently discard config without a signal.
pub fn parse_config_or_default(contents: &str) -> Config {
    match toml::from_str(contents) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("claude-focus: invalid config at {}, using defaults: {e}", config_path().display());
            Config::default()
        }
    }
}

pub fn load_config() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => parse_config_or_default(&contents),
        Err(_) => Config::default(),
    }
}
```

(Leave `config_path()` as-is for now; Task 4 changes it.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS — 2 config tests green. stderr will show the warning line during the invalid-config test — that is correct.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "fix(config): warn and keep defaults on a parse error

A single TOML typo previously wiped all config silently via
unwrap_or_default(). Now the toml::de::Error (with line/col) is printed
to stderr before falling back to defaults."
```

---

## Task 3: Fix the empty-type filter bypass + extract dispatch (§1.3, main.rs)

**Files:**
- Modify: `src/main.rs` (`run`, add `should_act`, add `dispatch`, add `#[cfg(test)]`)

- [ ] **Step 1: Write the failing tests**

Add to the bottom of `src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> Vec<String> {
        vec!["permission_prompt".into(), "idle_prompt".into()]
    }

    #[test]
    fn configured_type_acts() {
        assert!(should_act("idle_prompt", &types()));
    }

    #[test]
    fn unconfigured_type_skips() {
        assert!(!should_act("auth_success", &types()));
    }

    #[test]
    fn empty_type_skips_by_default() {
        // The old code let an empty type bypass the allowlist and always fire.
        assert!(!should_act("", &types()));
    }

    #[test]
    fn empty_type_acts_only_if_explicitly_listed() {
        assert!(should_act("", &[String::new()]));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test 2>&1 | tail -20`
Expected: FAIL — `cannot find function `should_act`` (the compile error aborts the whole test build).

- [ ] **Step 3: Implement `should_act` and `dispatch`, and rewire `run`**

Add these free functions to `src/main.rs`:

```rust
/// Whether this notification type should trigger action, given the allowlist.
/// An empty/missing type is treated as non-matching: it must be explicitly
/// listed to act (fixes the old empty-type bypass).
fn should_act(notification_type: &str, notify_types: &[String]) -> bool {
    notify_types.iter().any(|t| t == notification_type)
}

/// Perform the focus and/or notify actions for a notification.
/// Shared by the real hook path (`force = false`, honors `mode`) and the
/// `test` subcommand (`force = true`, fires BOTH legs regardless of `mode`).
fn dispatch(notification_type: &str, message: &str, config: &config::Config, force: bool) {
    let should_focus = force || config.mode == Mode::Both || config.mode == Mode::FocusOnly;
    let should_notify = force || config.mode == Mode::Both || config.mode == Mode::NotifyOnly;

    if should_focus {
        if let Some(pid) = process_tree::find_terminal_pid() {
            dbus::highlight_window(pid, config.notification_timeout_ms);
        }
    }
    if should_notify {
        notify::send_notification(notification_type, message, config);
    }
}
```

Then replace the filter + action block in `run()` (currently main.rs:43-60) with:

```rust
    if !should_act(notification_type, &config.notify_types) {
        return Ok(());
    }

    dispatch(notification_type, message, &config, false);

    Ok(())
```

Note: `config::Config` must be reachable by that name in `main.rs` — it already is via `mod config;`. `Mode` is already imported (`use config::Mode;`).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test 2>&1 | tail -20`
Expected: PASS — all tests green, including the 4 new `should_act` tests.

- [ ] **Step 5: Verify the full crate still builds**

Run: `cargo build 2>&1 | tail -5`
Expected: builds with no errors.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "fix(filter): empty notification_type no longer bypasses the allowlist

Extracts should_act() and a shared dispatch() helper. An empty/missing
type now fails the allowlist check instead of always firing."
```

---

## Task 4: Honor XDG_CONFIG_HOME (§1.4, config.rs + install.sh)

**Files:**
- Modify: `src/config.rs` (`config_path`, add `config_path_from`, extend tests)
- Modify: `scripts/install.sh:8`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/config.rs`:

```rust
    #[test]
    fn xdg_set_nonempty_wins() {
        assert_eq!(
            config_path_from("/home/u", Some("/cfg")),
            std::path::PathBuf::from("/cfg/claude-focus/config.toml")
        );
    }

    #[test]
    fn xdg_empty_falls_back_to_home() {
        // Per the XDG spec, an empty value means "unset".
        assert_eq!(
            config_path_from("/home/u", Some("")),
            std::path::PathBuf::from("/home/u/.config/claude-focus/config.toml")
        );
    }

    #[test]
    fn xdg_unset_falls_back_to_home() {
        assert_eq!(
            config_path_from("/home/u", None),
            std::path::PathBuf::from("/home/u/.config/claude-focus/config.toml")
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test xdg 2>&1 | tail -20`
Expected: FAIL — `cannot find function `config_path_from``.

- [ ] **Step 3: Implement `config_path_from` and route `config_path` through it**

In `src/config.rs`, replace `config_path`:

```rust
fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    config_path_from(&home, xdg.as_deref())
}

/// Resolve the config path. XDG_CONFIG_HOME wins only when set AND non-empty
/// (the XDG spec treats empty as unset; Rust's env::var returns Ok("") for an
/// empty var, so we must guard explicitly). Otherwise fall back to $HOME/.config.
fn config_path_from(home: &str, xdg: Option<&str>) -> PathBuf {
    let base = match xdg {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => PathBuf::from(home).join(".config"),
    };
    base.join("claude-focus").join("config.toml")
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test xdg 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Match the installer to the binary**

In `scripts/install.sh`, change line 8 from:
```bash
CONFIG_DIR="$HOME/.config/claude-focus"
```
to (bash `:-` already treats empty as unset, matching the Rust rule):
```bash
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/claude-focus"
```

- [ ] **Step 6: Verify the installer change**

Run: `XDG_CONFIG_HOME="" bash -c 'source <(grep -m1 CONFIG_DIR scripts/install.sh); echo "$CONFIG_DIR"'`
Expected: prints `<your-home>/.config/claude-focus` (empty falls back, no leading `/claude-focus`).

- [ ] **Step 7: Commit**

```bash
git add src/config.rs scripts/install.sh
git commit -m "fix(config): honor XDG_CONFIG_HOME in binary and installer

Treats an empty value as unset per the XDG spec so the binary and
install.sh always agree on the config location."
```

---

## Task 5: argv dispatch + `--version` (§1.5, main.rs)

**Files:**
- Modify: `src/main.rs` (`main`, add `Command`, `parse_args`, `print_help`, stubs for `run_test`/`run_doctor`)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/main.rs`:

```rust
    #[test]
    fn no_args_is_hook() {
        assert!(matches!(parse_args(&[]), Command::Hook));
    }

    #[test]
    fn version_flag_parses() {
        assert!(matches!(parse_args(&["--version".to_string()]), Command::Version));
        assert!(matches!(parse_args(&["-V".to_string()]), Command::Version));
    }

    #[test]
    fn doctor_parses() {
        assert!(matches!(parse_args(&["doctor".to_string()]), Command::Doctor));
    }

    #[test]
    fn test_with_type_parses() {
        match parse_args(&["test".to_string(), "idle_prompt".to_string()]) {
            Command::Test(Some(t)) => assert_eq!(t, "idle_prompt"),
            other => panic!("expected Test(Some), got {other:?}"),
        }
    }

    #[test]
    fn test_without_type_parses() {
        assert!(matches!(parse_args(&["test".to_string()]), Command::Test(None)));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test 2>&1 | tail -20`
Expected: FAIL — `cannot find type `Command`` / `cannot find function `parse_args`` (the compile error aborts the whole test build).

- [ ] **Step 3: Add the command enum, parser, help, and stubs**

Add to `src/main.rs`:

```rust
#[derive(Debug)]
enum Command {
    Hook,
    Version,
    Test(Option<String>),
    Doctor,
    Help,
}

/// Parse CLI args (already stripped of argv[0]). No args (or `hook`) preserves
/// the stdin-hook contract Claude Code relies on; anything unknown shows help
/// rather than blocking on stdin.
fn parse_args(args: &[String]) -> Command {
    match args.first().map(|s| s.as_str()) {
        None | Some("hook") => Command::Hook,
        Some("--version") | Some("-V") => Command::Version,
        Some("doctor") => Command::Doctor,
        Some("test") => Command::Test(args.get(1).cloned()),
        Some("--help") | Some("-h") => Command::Help,
        Some(_) => Command::Help,
    }
}

fn print_help() {
    eprintln!(
        "claude-focus {}\n\n\
         Usage:\n  \
         claude-focus            Run as a Claude Code Notification hook (reads JSON on stdin)\n  \
         claude-focus test [t]   Fire synthetic notification(s) through the real focus+notify path\n  \
         claude-focus doctor     Check deps, config, extension, and hook registration\n  \
         claude-focus --version  Print version",
        env!("CARGO_PKG_VERSION")
    );
}

fn run_test(_which: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("claude-focus: `test` not yet implemented");
    Ok(())
}

fn run_doctor() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("claude-focus: `doctor` not yet implemented");
    Ok(())
}
```

- [ ] **Step 4: Route `main` through the parser**

Replace `main()` in `src/main.rs` with:

```rust
fn main() {
    // Always exit 0 — never block Claude Code.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match parse_args(&args) {
        Command::Hook => run(),
        Command::Version => {
            println!("claude-focus {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Test(t) => run_test(t.as_deref()),
        Command::Doctor => run_doctor(),
        Command::Help => {
            print_help();
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("claude-focus: {e}");
    }
}
```

- [ ] **Step 5: Run tests + build, and smoke-check the unchanged hook path**

Run: `cargo test 2>&1 | tail -20` → Expected: PASS — all tests green, including the 5 new `parse_args` tests.
Run: `cargo build 2>&1 | tail -5` → Expected: clean build.
Run: `cargo run -- --version` → Expected: `claude-focus 0.1.0`.
Run: `echo '{"notification_type":"idle_prompt","message":"hi"}' | cargo run --` → Expected: behaves exactly as before (notification/focus per your config), proving the default path is unchanged.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): argv dispatch with --version (test/doctor stubbed)

No args still reads stdin as the hook; adds --version/-V, help, and
stubs for the test and doctor subcommands."
```

---

## Task 6: `claude-focus test [type]` (§1.5, main.rs)

**Files:**
- Modify: `src/main.rs` (implement `run_test`, add `ALL_TYPES`, `test_plan`, extend tests)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/main.rs`:

```rust
    #[test]
    fn test_plan_no_arg_covers_all_four_with_allowlist_flags() {
        let notify = vec!["permission_prompt".to_string(), "idle_prompt".to_string()];
        let plan = test_plan(None, &notify);
        assert_eq!(plan.len(), 4);
        // auth_success is absent from the allowlist -> flagged false (forced anyway).
        let auth = plan.iter().find(|(t, _)| t == "auth_success").unwrap();
        assert!(!auth.1);
        let perm = plan.iter().find(|(t, _)| t == "permission_prompt").unwrap();
        assert!(perm.1);
    }

    #[test]
    fn test_plan_single_type() {
        let plan = test_plan(Some("idle_prompt"), &[]);
        assert_eq!(plan, vec![("idle_prompt".to_string(), false)]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test test_plan 2>&1 | tail -20`
Expected: FAIL — `cannot find function `test_plan``.

- [ ] **Step 3: Implement `test_plan` and `run_test`**

In `src/main.rs`, add the constant and helper, and replace the `run_test` stub:

```rust
const ALL_TYPES: [&str; 4] = [
    "permission_prompt",
    "idle_prompt",
    "elicitation_dialog",
    "auth_success",
];

/// Build the (type, in_allowlist) list `test` will fire. Pure, for testing.
fn test_plan(which: Option<&str>, notify_types: &[String]) -> Vec<(String, bool)> {
    let types: Vec<&str> = match which {
        Some(t) => vec![t],
        None => ALL_TYPES.to_vec(),
    };
    types
        .into_iter()
        .map(|t| (t.to_string(), notify_types.iter().any(|x| x == t)))
        .collect()
}

/// Diagnostic: fire synthetic notification(s) through the REAL focus+notify
/// code, intentionally BYPASSING both the notify_types allowlist AND the config
/// `mode` gate (`force = true`) — `test` always exercises focus AND notify so
/// you can confirm each leg. Highlights the invoking terminal (the /proc walk
/// starts from this process).
fn run_test(which: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let config = config::load_config();
    let plan = test_plan(which, &config.notify_types);
    for (ty, in_allowlist) in plan {
        let note = if in_allowlist {
            ""
        } else {
            "  (not in your notify_types — forcing anyway)"
        };
        println!("→ firing {ty}{note}");
        dispatch(&ty, "", &config, true);
        std::thread::sleep(std::time::Duration::from_millis(800));
    }
    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test test_plan 2>&1 | tail -20`
Expected: PASS (2 tests).

- [ ] **Step 5: Manual verification of the firing path**

Run: `cargo build --release && ./target/release/claude-focus test idle_prompt`
Expected: one desktop notification appears AND the current terminal is raised/bordered (focus leg targets the invoking terminal — intentional).
Run: `./target/release/claude-focus test`
Expected: four lines printed; `auth_success` annotated `(not in your notify_types — forcing anyway)`; four notifications fire ~0.8s apart.
Run (proves `test` ignores the `mode` gate, not just the allowlist):
```bash
mkdir -p /tmp/dfm/.config/claude-focus
printf 'mode = "notify-only"\n' > /tmp/dfm/.config/claude-focus/config.toml
env -u XDG_CONFIG_HOME HOME=/tmp/dfm ./target/release/claude-focus test idle_prompt
```
Expected: BOTH a notification AND a terminal raise/border fire, despite `mode = "notify-only"`.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): implement \`test\` subcommand

Fires synthetic notifications through the real focus+notify path,
bypassing the allowlist, annotating types absent from notify_types."
```

---

## Task 7: `claude-focus doctor` (§1.5, src/doctor.rs)

**Files:**
- Create: `src/doctor.rs`
- Modify: `src/config.rs` (add `public_config_path`)
- Modify: `src/main.rs` (`mod doctor;`, wire `run_doctor` to `doctor::run`)

- [ ] **Step 1: Write the failing tests (tolerant, exact-match hook detection)**

Create `src/doctor.rs` with the settings parser and its tests. It matches the hook command by EXACT equality with the path `install.sh` writes (`$HOME/.local/bin/claude-focus`, install.sh:40,65-66), so installer and doctor never disagree — a loose `ends_with` would wrongly pass `/usr/bin/not-claude-focus`:

```rust
/// Whether the claude-focus hook command is registered in settings.json.
/// Matches `expected_command` by EXACT equality (the same check install.sh uses
/// at lines 65-68). Returns Err with a message if the file is present but
/// unparseable — doctor must report that as a FAIL leg, never panic (Phase 2.1
/// hardens the writer; doctor must be robust before that lands).
pub fn hook_registered(settings_json: &str, expected_command: &str) -> Result<bool, String> {
    let v: serde_json::Value = serde_json::from_str(settings_json)
        .map_err(|e| format!("settings.json is not valid JSON: {e}"))?;
    let found = v
        .get("hooks")
        .and_then(|h| h.get("Notification"))
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

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: &str = "/home/u/.local/bin/claude-focus";
    const WITH_HOOK: &str = r#"{"hooks":{"Notification":[{"matcher":"*","hooks":[{"type":"command","command":"/home/u/.local/bin/claude-focus"}]}]}}"#;
    const WRONG_PATH: &str = r#"{"hooks":{"Notification":[{"matcher":"*","hooks":[{"type":"command","command":"/usr/bin/not-claude-focus"}]}]}}"#;

    #[test]
    fn detects_registered_hook() {
        assert_eq!(hook_registered(WITH_HOOK, EXPECTED), Ok(true));
    }

    #[test]
    fn rejects_a_different_command_path() {
        // ends_with("claude-focus") would wrongly PASS this; exact match rejects it.
        assert_eq!(hook_registered(WRONG_PATH, EXPECTED), Ok(false));
    }

    #[test]
    fn malformed_json_is_err_not_panic() {
        assert!(hook_registered("{not json", EXPECTED).is_err());
    }
}
```

(No `use std::process` yet — `hook_registered` uses only `serde_json`, which is in scope as a dependency. The process probes and their `use` arrive in Step 4, avoiding an unused-import warning.)

- [ ] **Step 2: Register the module and run the tests**

Add `mod doctor;` near the top of `src/main.rs` (with the other `mod` lines).
Run: `cargo test 2>&1 | tail -20`
Expected: the crate COMPILES and all tests pass, including the 3 new doctor tests (`detects_registered_hook`, `rejects_a_different_command_path`, `malformed_json_is_err_not_panic`).

- [ ] **Step 3: Add the public config-path accessor (before doctor uses it)**

`doctor::run` (next step) needs the resolved config path and the `Config` type. Add the wrapper FIRST so every step leaves the crate compilable. In `src/config.rs`, add a public wrapper (keep `config_path` private); `Config` is already `pub`:

```rust
/// Public accessor for the resolved config path (used by `doctor`).
pub fn public_config_path() -> PathBuf {
    config_path()
}
```

Run: `cargo build 2>&1 | tail -5` → Expected: clean build (the new fn is unused until Step 4 — a dead-code warning is acceptable here).

- [ ] **Step 4: Implement the leg probes and `run`, and wire it up**

Append to `src/doctor.rs` (the `use` for the process API is added here, where it is first used):

```rust
use std::process::{Command, Stdio};

fn on_path(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn dbus_name_owned() -> bool {
    Command::new("gdbus")
        .args([
            "introspect",
            "--session",
            "--dest",
            "org.gnome.Shell.Extensions.FocusByPid",
            "--object-path",
            "/org/gnome/Shell/Extensions/FocusByPid",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn extension_enabled() -> bool {
    Command::new("gnome-extensions")
        .args(["list", "--enabled"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("focus-by-pid@claude.local"))
        .unwrap_or(false)
}

fn line(ok: bool, label: &str, fix: &str) {
    if ok {
        println!("  [PASS] {label}");
    } else {
        println!("  [FAIL] {label}\n         fix: {fix}");
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("claude-focus doctor\n");

    // Config parses (tolerant — prints the exact error, never panics). Keep the
    // effective Config to reuse for the sound-file leg.
    let cfg_path = crate::config::public_config_path();
    let effective: crate::config::Config = match std::fs::read_to_string(&cfg_path) {
        Ok(c) => match toml::from_str(&c) {
            Ok(cfg) => {
                line(true, "config parses", "");
                cfg
            }
            Err(e) => {
                line(false, "config parses", &format!("fix the TOML error: {e}"));
                crate::config::Config::default()
            }
        },
        Err(_) => {
            line(true, "config parses (none yet — defaults in use)", "");
            crate::config::Config::default()
        }
    };

    // Sound file exists when play_sound is on (read-only — does not touch the
    // notify.rs emission path, which Phase 2.7 owns).
    if effective.play_sound {
        match effective.sound_file {
            Some(ref sf) => line(
                std::path::Path::new(sf).exists(),
                "sound file exists",
                "set sound_file to a real path or play_sound=false",
            ),
            None => line(false, "sound file set", "set sound_file or play_sound=false"),
        }
    }

    // Dependencies on PATH.
    let gdbus = on_path("gdbus");
    line(on_path("notify-send"), "notify-send on PATH", "install libnotify-bin");
    line(on_path("pw-play"), "pw-play on PATH", "install pipewire-bin (or set play_sound=false)");
    line(gdbus, "gdbus on PATH", "install libglib2.0-bin");

    // D-Bus service owned (depends on gdbus; report once).
    if gdbus {
        line(dbus_name_owned(), "FocusByPid D-Bus service owned",
             "log out/in or restart GNOME Shell to load the extension");
    }

    // Extension enabled.
    line(extension_enabled(), "extension enabled",
         "gnome-extensions enable focus-by-pid@claude.local");

    // Hook registered (exact match to install.sh; tolerant of malformed JSON).
    let home = std::env::var("HOME").unwrap_or_default();
    let expected_command = format!("{home}/.local/bin/claude-focus");
    let settings = format!("{home}/.claude/settings.json");
    match std::fs::read_to_string(&settings) {
        Ok(s) => match hook_registered(&s, &expected_command) {
            Ok(true) => line(true, "hook registered in settings.json", ""),
            Ok(false) => line(false, "hook registered in settings.json", "re-run scripts/install.sh"),
            Err(e) => line(false, "settings.json parses", &e),
        },
        Err(_) => line(false, "hook registered in settings.json", "run scripts/install.sh"),
    }

    Ok(())
}
```

Then wire `run_doctor` in `src/main.rs` — replace the `run_doctor` stub body with:

```rust
fn run_doctor() -> Result<(), Box<dyn std::error::Error>> {
    doctor::run()
}
```

- [ ] **Step 5: Run tests + build**

Run: `cargo test 2>&1 | tail -20` → Expected: all tests pass (config + main + the 3 doctor tests).
Run: `cargo build 2>&1 | tail -5` → Expected: clean build, no warnings.

- [ ] **Step 6: Manual verification, including the no-panic guarantees**

Run: `cargo build --release && ./target/release/claude-focus doctor`
Expected: a PASS/FAIL checklist for config, sound file (only when `play_sound = true`), deps, D-Bus, extension, hook.
Robustness checks (must NOT panic, must report FAIL, exit 0). doctor reads `$HOME/.claude/settings.json` and `public_config_path()`, so override `HOME` and clear `XDG_CONFIG_HOME`, and write to the exact paths doctor reads:
```bash
# malformed settings.json (note the .claude/ subdir doctor actually reads)
mkdir -p /tmp/df/.claude && printf '{not json' > /tmp/df/.claude/settings.json
env -u XDG_CONFIG_HOME HOME=/tmp/df ./target/release/claude-focus doctor   # -> settings.json FAIL leg, no panic
# malformed config
mkdir -p /tmp/df/.config/claude-focus && printf '[broken' > /tmp/df/.config/claude-focus/config.toml
env -u XDG_CONFIG_HOME HOME=/tmp/df ./target/release/claude-focus doctor   # -> config leg FAIL with the TOML error, no panic
```
Expected both: clean checklist output, no Rust panic/backtrace.

- [ ] **Step 7: Commit**

```bash
git add src/doctor.rs src/main.rs src/config.rs
git commit -m "feat(cli): implement \`doctor\` health check

Checks config parse, sound file, deps on PATH, D-Bus service ownership,
extension enabled, and hook registration (exact path match to install.sh).
Tolerant of malformed config/settings — reports FAIL, never panics."
```

---

## Self-review (completed during authoring + adversarial plan review)

- **Spec coverage:** §1.1 → Task 1; §1.2 → Task 2; §1.3 → Task 3; §1.4 → Task 4; §1.5 argv/`--version` → Task 5, `test` → Task 6, `doctor` (config-parse, **sound-file**, deps, D-Bus, extension, hook) → Task 7. All Phase 1 acceptance criteria map to a task, including the spec-review decisions: `test` bypasses BOTH the notify_types allowlist and the `mode` gate (`dispatch(..., force = true)`) and focuses the invoking terminal; `doctor` tolerates malformed config and settings.json without panicking and matches the hook by the EXACT path `install.sh` writes.
- **Type consistency:** `should_act`, `dispatch(_, _, _, force)`, `Command`, `parse_args`, `test_plan`, `ALL_TYPES`, `config_path_from`, `parse_config_or_default`, `public_config_path`, `hook_registered(_, expected_command)` are each defined once and referenced with the same signature throughout. `config::Config` / `config::Mode` reuse existing names.
- **Incremental compile:** every task's final commit compiles; within Task 7 the `public_config_path` accessor lands (Step 3) before `doctor::run` references it (Step 4). Test verifications use a plain `cargo test` (a name filter would match test-function names, not the function under test, and `--lib` fails on this binary-only crate).
- **Scope guard honored:** no task edits the `notify.rs` sound path (deferred to Phase 2.7); doctor's sound-file leg is read-only.
- **No placeholders:** every code/step is concrete; no TBD/TODO.

## Post-Phase-1

After all tasks pass and are committed, push the branch and confirm the Phase 1 sub-issue checklist is complete. Phase 2 gets its own plan (`writing-plans`) when it starts.
