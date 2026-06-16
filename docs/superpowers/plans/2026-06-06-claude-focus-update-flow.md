# Claude Focus — Local Update Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give developers a fast, granular, scripted way to redeploy local changes — `make update`/`bin`/`ext`/`watch` over a flag-enhanced, backward-compatible `install.sh`.

**Architecture:** Refactor `scripts/install.sh` into discrete shell functions (`do_build`/`do_bin`/`do_ext`/`do_config`/`do_hook`/`do_enable`) driven by a `main` arg parser; **no flags = today's full install** (backward compatible). A root `Makefile` adds discoverable shortcuts. A dependency-free bash test suite (`tests/test_update_flow.sh`) sandboxes all install paths via overridable env vars (`PROJECT_DIR`/`BIN_DIR`/`EXT_DIR`/`CONFIG_DIR`/`SETTINGS_FILE`) plus fake `cargo`/`gnome-extensions` on `PATH`, so tests never touch the real system or run a real build.

**Tech Stack:** Bash, GNU Make, Python 3 (existing settings-merge step), `cmp` (smart change-detection), `cargo-watch` (optional dev dep for `make watch`).

**Spec:** `docs/superpowers/specs/2026-06-06-claude-focus-update-flow-design.md`

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `scripts/install.sh` | Build/deploy each artifact; dispatch on flags | **Modify** (refactor + flags + smart notice) |
| `Makefile` | Discoverable shortcuts over `install.sh` + `make check` | **Create** |
| `tests/test_update_flow.sh` | Hermetic, dependency-free test suite | **Create** |
| `README.md` | User-facing "Updating & Development" docs | **Modify** |

**Testing strategy (read before Task 1):** The *current* `install.sh` hardcodes `$HOME/...` paths and runs a real `cargo build` + real `cp` into the real environment on **any** invocation. It is therefore **destructive and unsandboxable as-is** — you cannot safely "run it to watch a test go red." Task 1 makes it sandboxable (overridable vars + fakes) as a behavior-preserving refactor; its red state is confirmed **by inspection**, not execution. Every task after Task 1 runs fully sandboxed, so red/green are both executed normally.

---

## Task 1: Behavior-preserving refactor + sandboxed test harness

Make `install.sh` testable (overridable paths, extracted functions, a `main`) while keeping the no-flags behavior byte-for-byte equivalent, and stand up the test harness with the full-install test.

**Files:**
- Modify: `scripts/install.sh` (whole-file rewrite, behavior-preserving)
- Create: `tests/test_update_flow.sh`

- [ ] **Step 1: Write the test harness + the full-install test**

Create `tests/test_update_flow.sh`:

```bash
#!/usr/bin/env bash
# Dependency-free test suite for the local update flow (install.sh flags + Makefile).
# Runs every install path in a sandbox: overridable env vars redirect all writes into
# a tempdir, and fake cargo/gnome-extensions on PATH avoid real builds / real GNOME state.
set -uo pipefail   # deliberately NOT -e: run all tests, tally failures

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL="$REPO/scripts/install.sh"
PASS=0; FAIL=0

ok()  { PASS=$((PASS+1)); echo "  ok   - $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL - $1"; }
assert_contains()     { case "$2" in *"$3"*) ok "$1";; *) bad "$1 (missing: '$3')";; esac; }
assert_not_contains() { case "$2" in *"$3"*) bad "$1 (unexpected: '$3')";; *) ok "$1";; esac; }
assert_present()      { if [ -e "$2" ]; then ok "$1"; else bad "$1 ($2 missing)"; fi; }
assert_absent()       { if [ ! -e "$2" ]; then ok "$1"; else bad "$1 ($2 exists)"; fi; }
assert_eq()           { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (got '$2' want '$3')"; fi; }

make_sandbox() {
  SB="$(mktemp -d)"
  PROJECT_DIR="$SB/project"; BIN_DIR="$SB/bin"; EXT_DIR="$SB/ext"
  CONFIG_DIR="$SB/config"; SETTINGS_FILE="$SB/claude/settings.json"
  mkdir -p "$PROJECT_DIR/extension" "$PROJECT_DIR/config"
  cp "$REPO/extension/metadata.json"  "$PROJECT_DIR/extension/"
  cp "$REPO/extension/extension.js"   "$PROJECT_DIR/extension/"
  cp "$REPO/config/claude-focus.toml" "$PROJECT_DIR/config/"
  mkdir -p "$SB/fakebin"
  printf '#!/usr/bin/env bash\nmkdir -p target/release\necho dummy > target/release/claude-focus\n' > "$SB/fakebin/cargo"
  printf '#!/usr/bin/env bash\nexit 0\n' > "$SB/fakebin/gnome-extensions"
  chmod +x "$SB/fakebin/cargo" "$SB/fakebin/gnome-extensions"
}

# Run install.sh inside the sandbox. Captures combined output in $OUT and exit code in $RC.
run_install() {
  OUT="$(PATH="$SB/fakebin:$PATH" PROJECT_DIR="$PROJECT_DIR" BIN_DIR="$BIN_DIR" \
         EXT_DIR="$EXT_DIR" CONFIG_DIR="$CONFIG_DIR" SETTINGS_FILE="$SETTINGS_FILE" \
         bash "$INSTALL" "$@" 2>&1)"; RC=$?
}

echo "== install.sh: full install (no flags) =="
make_sandbox
run_install
assert_eq       "full install exits 0"              "$RC" "0"
assert_present  "full install creates binary"        "$BIN_DIR/claude-focus"
assert_present  "full install creates extension.js"  "$EXT_DIR/extension.js"
assert_present  "full install creates metadata.json" "$EXT_DIR/metadata.json"
assert_present  "full install creates config"        "$CONFIG_DIR/config.toml"
assert_present  "full install writes settings"       "$SETTINGS_FILE"
assert_contains "settings reference the hook path"   "$(cat "$SETTINGS_FILE")" "$BIN_DIR/claude-focus"
rm -rf "$SB"

echo ""
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
```

- [ ] **Step 2: Confirm the red state by inspection (do NOT run against the current script)**

The current `install.sh` is destructive and unsandboxable (see Testing strategy above), so do not execute the suite yet. Verify it *would* fail:

Run: `grep -nE 'BIN_DIR=|EXT_DIR=|PROJECT_DIR=' scripts/install.sh`
Expected: the vars are hardcoded to `$HOME/...` / `$(dirname …)` with **no** `${VAR:-…}` override — so the sandbox env vars are ignored and the suite cannot run safely. This is the red state.

- [ ] **Step 3: Rewrite `scripts/install.sh` (overridable vars + functions + `main`, behavior-preserving)**

Replace the entire contents of `scripts/install.sh` with:

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="${PROJECT_DIR:-$(dirname "$SCRIPT_DIR")}"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
EXT_DIR="${EXT_DIR:-$HOME/.local/share/gnome-shell/extensions/focus-by-pid@claude.local}"
CONFIG_DIR="${CONFIG_DIR:-$HOME/.config/claude-focus}"
SETTINGS_FILE="${SETTINGS_FILE:-$HOME/.claude/settings.json}"

do_build() {
    echo "==> Building claude-focus..."
    ( cd "$PROJECT_DIR" && cargo build --release )
}

do_bin() {
    echo "==> Installing binary to $BIN_DIR..."
    mkdir -p "$BIN_DIR"
    cp "$PROJECT_DIR/target/release/claude-focus" "$BIN_DIR/claude-focus"
    chmod +x "$BIN_DIR/claude-focus"
}

do_ext() {
    echo "==> Installing GNOME Shell extension..."
    mkdir -p "$EXT_DIR"
    cp "$PROJECT_DIR/extension/metadata.json" "$EXT_DIR/"
    cp "$PROJECT_DIR/extension/extension.js" "$EXT_DIR/"
}

do_config() {
    echo "==> Installing config..."
    mkdir -p "$CONFIG_DIR"
    if [ ! -f "$CONFIG_DIR/config.toml" ]; then
        cp "$PROJECT_DIR/config/claude-focus.toml" "$CONFIG_DIR/config.toml"
        echo "    Created $CONFIG_DIR/config.toml"
    else
        echo "    Config already exists, skipping"
    fi
}

do_hook() {
    echo "==> Merging hook into Claude Code settings..."
    mkdir -p "$(dirname "$SETTINGS_FILE")"
    SETTINGS_FILE="$SETTINGS_FILE" BIN_DIR="$BIN_DIR" python3 - <<'PY'
import json, os
settings_file = os.environ['SETTINGS_FILE']
hook_command = os.path.join(os.environ['BIN_DIR'], 'claude-focus')

if os.path.exists(settings_file):
    with open(settings_file) as f:
        settings = json.load(f)
else:
    settings = {}

hook_entry = {'matcher': '*', 'hooks': [{'type': 'command', 'command': hook_command}]}
hooks = settings.setdefault('hooks', {})
notifications = hooks.setdefault('Notification', [])
already_present = any(
    any(h.get('command') == hook_command for h in entry.get('hooks', []))
    for entry in notifications
)
if not already_present:
    notifications.append(hook_entry)
    with open(settings_file, 'w') as f:
        json.dump(settings, f, indent=2)
    print('    Hook added to', settings_file)
else:
    print('    Hook already present, skipping')
PY
}

do_enable() {
    echo "==> Enabling GNOME Shell extension..."
    if command -v gnome-extensions &>/dev/null; then
        gnome-extensions enable focus-by-pid@claude.local 2>/dev/null || true
        echo "    Extension enabled (may require log out/in to take effect)"
    else
        echo "    gnome-extensions not found, skip enabling"
    fi
}

print_full_summary() {
    echo ""
    echo "Installation complete!"
    echo ""
    echo "  Binary:    $BIN_DIR/claude-focus"
    echo "  Config:    $CONFIG_DIR/config.toml"
    echo "  Extension: $EXT_DIR/"
    echo ""
    echo "NOTE: For auto-focus to work, you must log out and log back in"
    echo "      (or restart GNOME Shell) to load the extension."
    echo "      Desktop notifications work immediately."
}

main() {
    do_build
    do_bin
    do_ext
    do_config
    do_hook
    do_enable
    print_full_summary
}

main "$@"
```

Note: `do_hook` now passes paths to Python via environment variables instead of shell-interpolating them into the source — same behavior, no quoting fragility. `do_build` uses a subshell `( cd … )` so it never changes the caller's working directory; `do_bin` copies via the absolute `$PROJECT_DIR` path.

- [ ] **Step 4: Run the suite to verify it passes**

Run: `bash tests/test_update_flow.sh`
Expected: `7 passed, 0 failed`, exit 0.

- [ ] **Step 5: Confirm real-world backward compatibility (non-destructive check)**

Run: `bash scripts/install.sh --help >/dev/null 2>&1; echo "rc=$?"`
Expected: `rc=0` is NOT required here — `--help` isn't implemented until Task 2; this step only confirms the script still parses. Instead run:
Run: `bash -n scripts/install.sh && echo "syntax-ok"`
Expected: `syntax-ok` (no syntax errors in the refactor).

- [ ] **Step 6: Commit**

```bash
git add scripts/install.sh tests/test_update_flow.sh
git commit -m "$(printf '%s\n' \
  'Refactor install.sh into functions + add sandboxed test harness' \
  '' \
  'Behavior-preserving: no-flags install is unchanged. Paths are now' \
  'overridable (PROJECT_DIR/BIN_DIR/EXT_DIR/CONFIG_DIR/SETTINGS_FILE) so the' \
  'new dependency-free suite can sandbox every path with fake cargo/gnome-extensions.' \
  '' \
  'Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 2: Flag parsing — `--bin`, `--ext`, combos, `--help`, unknown

Teach `main` to dispatch on flags. No flags still means full install.

**Files:**
- Modify: `scripts/install.sh` (replace `main`, add `usage`)
- Modify: `tests/test_update_flow.sh` (add a flag-dispatch section)

- [ ] **Step 1: Add failing tests for flag dispatch**

In `tests/test_update_flow.sh`, insert the following block immediately **before** the final `echo ""` / `echo "$PASS passed…"` lines:

```bash
echo "== install.sh: --bin (binary only) =="
make_sandbox
run_install --bin
assert_eq      "--bin exits 0"            "$RC" "0"
assert_present "--bin installs binary"     "$BIN_DIR/claude-focus"
assert_absent  "--bin skips extension"     "$EXT_DIR/extension.js"
assert_absent  "--bin skips config"        "$CONFIG_DIR/config.toml"
assert_absent  "--bin skips settings"      "$SETTINGS_FILE"
rm -rf "$SB"

echo "== install.sh: --ext (extension only) =="
make_sandbox
run_install --ext
assert_eq      "--ext exits 0"             "$RC" "0"
assert_present "--ext installs extension"   "$EXT_DIR/extension.js"
assert_absent  "--ext skips binary"         "$BIN_DIR/claude-focus"
assert_absent  "--ext skips config"         "$CONFIG_DIR/config.toml"
rm -rf "$SB"

echo "== install.sh: --bin --ext (combined) =="
make_sandbox
run_install --bin --ext
assert_present "combo installs binary"      "$BIN_DIR/claude-focus"
assert_present "combo installs extension"   "$EXT_DIR/extension.js"
assert_absent  "combo skips config"         "$CONFIG_DIR/config.toml"
rm -rf "$SB"

echo "== install.sh: --help =="
make_sandbox
run_install --help
assert_eq       "--help exits 0"           "$RC" "0"
assert_contains "--help prints usage"      "$OUT" "Usage:"
assert_absent   "--help installs nothing"  "$BIN_DIR/claude-focus"
rm -rf "$SB"

echo "== install.sh: unknown flag =="
make_sandbox
run_install --nope
assert_eq       "unknown flag exits 1"     "$RC" "1"
assert_contains "unknown flag warns"       "$OUT" "Unknown option"
rm -rf "$SB"
```

- [ ] **Step 2: Run the suite to verify the new tests fail**

Run: `bash tests/test_update_flow.sh`
Expected: FAIL — e.g. `--bin skips config` fails because `main` currently ignores args and runs the full install (config + extension created). Several new assertions fail; the script exits non-zero.

- [ ] **Step 3: Replace `main` and add `usage` in `scripts/install.sh`**

In `scripts/install.sh`, replace the `main()` function (the block from `main() {` through its closing `}`, just above `main "$@"`) with:

```bash
usage() {
    cat <<'EOF'
Usage: install.sh [--bin] [--ext]

  (no flags)   Full install: build + binary + extension + config + hook + enable
  --bin        Rebuild and reinstall the binary only (live immediately)
  --ext        Reinstall the GNOME extension only (relogin notice if it changed)
  -h, --help   Show this help

Env overrides (testing): PROJECT_DIR BIN_DIR EXT_DIR CONFIG_DIR SETTINGS_FILE
EOF
}

main() {
    local want_bin=0 want_ext=0 do_all=1
    while [ $# -gt 0 ]; do
        case "$1" in
            --bin) want_bin=1; do_all=0 ;;
            --ext) want_ext=1; do_all=0 ;;
            -h|--help) usage; exit 0 ;;
            *) echo "Unknown option: $1" >&2; echo >&2; usage >&2; exit 1 ;;
        esac
        shift
    done

    if [ "$do_all" -eq 1 ]; then
        do_build; do_bin; do_ext; do_config; do_hook; do_enable
        print_full_summary
    else
        if [ "$want_bin" -eq 1 ]; then do_build; do_bin; fi
        if [ "$want_ext" -eq 1 ]; then do_ext; fi
    fi
}
```

(Leave the final `main "$@"` line untouched.) Note: dispatch uses `if` blocks, **not** `cond && cmd`, because under `set -e` a false `&&` test would abort the script.

- [ ] **Step 4: Run the suite to verify it passes**

Run: `bash tests/test_update_flow.sh`
Expected: all assertions pass (`… passed, 0 failed`), exit 0.

- [ ] **Step 5: Commit**

```bash
git add scripts/install.sh tests/test_update_flow.sh
git commit -m "$(printf '%s\n' \
  'Add --bin/--ext flag dispatch to install.sh (no flags = full install)' \
  '' \
  'Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 3: Smart extension-changed relogin notice

`do_ext` should only tell the user to log out when the extension files actually changed.

**Files:**
- Modify: `scripts/install.sh` (`do_ext`)
- Modify: `tests/test_update_flow.sh` (add a notice section)

- [ ] **Step 1: Add failing tests for the notice**

In `tests/test_update_flow.sh`, insert this block before the final summary lines:

```bash
echo "== install.sh: --ext smart notice =="
make_sandbox
run_install --ext                                   # first install: files are new
assert_contains "first --ext says relogin"   "$OUT" "Log out/in to load"
run_install --ext                                   # second install: identical
assert_contains "unchanged --ext says nothing to reload" "$OUT" "nothing to reload"
echo "// changed" >> "$PROJECT_DIR/extension/extension.js"
run_install --ext                                   # now the source differs
assert_contains "changed --ext says relogin" "$OUT" "Log out/in to load"
rm -rf "$SB"
```

- [ ] **Step 2: Run the suite to verify the new tests fail**

Run: `bash tests/test_update_flow.sh`
Expected: FAIL — `do_ext` prints no notice yet, so `first --ext says relogin`, `unchanged …`, and `changed …` all fail.

- [ ] **Step 3: Update `do_ext` with change detection**

In `scripts/install.sh`, replace the entire `do_ext()` function with:

```bash
do_ext() {
    echo "==> Installing GNOME Shell extension..."
    mkdir -p "$EXT_DIR"
    local changed=0 f
    for f in metadata.json extension.js; do
        if [ ! -f "$EXT_DIR/$f" ] || ! cmp -s "$PROJECT_DIR/extension/$f" "$EXT_DIR/$f"; then
            changed=1
        fi
    done
    cp "$PROJECT_DIR/extension/metadata.json" "$EXT_DIR/"
    cp "$PROJECT_DIR/extension/extension.js" "$EXT_DIR/"
    if [ "$changed" -eq 1 ]; then
        echo "    Log out/in to load the updated extension."
    else
        echo "    Extension unchanged — nothing to reload."
    fi
}
```

(The `! cmp -s …` inside the `if` condition is safe under `set -e` — commands in test positions are exempt from `errexit`.)

- [ ] **Step 4: Run the suite to verify it passes**

Run: `bash tests/test_update_flow.sh`
Expected: all pass, exit 0.

- [ ] **Step 5: Commit**

```bash
git add scripts/install.sh tests/test_update_flow.sh
git commit -m "$(printf '%s\n' \
  'Only prompt for relogin when the extension actually changed' \
  '' \
  'Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 4: Makefile with discoverable targets + `make check`

**Files:**
- Create: `Makefile` (repo root)
- Modify: `tests/test_update_flow.sh` (add a Makefile section)

- [ ] **Step 1: Add failing tests for the Makefile**

In `tests/test_update_flow.sh`, insert this block before the final summary lines:

```bash
echo "== Makefile: targets map to the right commands =="
cd "$REPO"
assert_contains "bin -> install.sh --bin"        "$(make -n bin 2>&1)"       "scripts/install.sh --bin"
assert_contains "ext -> install.sh --ext"        "$(make -n ext 2>&1)"       "scripts/install.sh --ext"
assert_contains "update -> --bin --ext"          "$(make -n update 2>&1)"    "scripts/install.sh --bin --ext"
assert_contains "install -> install.sh"          "$(make -n install 2>&1)"   "scripts/install.sh"
assert_contains "uninstall -> uninstall.sh"      "$(make -n uninstall 2>&1)" "scripts/uninstall.sh"
assert_contains "watch -> cargo watch"           "$(make -n watch 2>&1)"     "cargo watch -w src"
assert_contains "watch guards cargo-watch"       "$(make -n watch 2>&1)"     "cargo install cargo-watch"
assert_contains "check -> test suite"            "$(make -n check 2>&1)"     "tests/test_update_flow.sh"
HELP="$(make help 2>&1)"
assert_contains "help lists update"  "$HELP" "update"
assert_contains "help lists watch"   "$HELP" "watch"
assert_contains "default goal = help" "$(make 2>&1)" "update"
```

(No `rm -rf "$SB"` here — this section uses no sandbox.)

- [ ] **Step 2: Run the suite to verify the new tests fail**

Run: `bash tests/test_update_flow.sh`
Expected: FAIL — `make: *** No rule … / No such file Makefile`, every Makefile assertion fails.

- [ ] **Step 3: Create `Makefile` at the repo root**

Create `Makefile`. **Recipe lines MUST be indented with a literal TAB, not spaces** (Make requires tabs):

```makefile
SHELL := bash
.DEFAULT_GOAL := help
.PHONY: help install update bin ext uninstall watch check

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

install: ## Full install (binary + extension + config + hook + enable)
	./scripts/install.sh

update: ## Deploy local changes (binary live now; extension after relogin)
	./scripts/install.sh --bin --ext

bin: ## Rebuild + reinstall the binary only (live instantly)
	./scripts/install.sh --bin

ext: ## Reinstall the GNOME extension only (relogin notice if changed)
	./scripts/install.sh --ext

uninstall: ## Remove everything (config preserved)
	./scripts/uninstall.sh

watch: ## Auto-rebuild + reinstall binary on every source save (needs cargo-watch)
	@command -v cargo-watch >/dev/null 2>&1 || { \
	  echo "cargo-watch not found. Install with: cargo install cargo-watch"; exit 1; }
	cargo watch -w src -w Cargo.toml -s './scripts/install.sh --bin'

check: ## Run the update-flow test suite
	@bash tests/test_update_flow.sh
```

- [ ] **Step 4: Run the suite to verify it passes**

Run: `bash tests/test_update_flow.sh`
Expected: all pass, exit 0.

- [ ] **Step 5: Verify `make check` runs the suite (and the watch guard works on this box)**

Run: `make check`
Expected: the suite runs and ends `… passed, 0 failed`.
Run: `make watch`
Expected (cargo-watch is absent here): prints `cargo-watch not found. Install with: cargo install cargo-watch` and exits non-zero — no crash.

- [ ] **Step 6: Commit**

```bash
git add Makefile tests/test_update_flow.sh
git commit -m "$(printf '%s\n' \
  'Add Makefile: update/bin/ext/install/uninstall/watch/help/check' \
  '' \
  'Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 5: README "Updating & Development" section

Document the new flow for future-you and other developers.

**Files:**
- Modify: `README.md`
- Modify: `tests/test_update_flow.sh` (add a docs check)

- [ ] **Step 1: Add a failing docs test**

In `tests/test_update_flow.sh`, insert before the final summary lines:

```bash
echo "== README documents the update flow =="
READ="$(cat "$REPO/README.md")"
assert_contains "README has an Updating section" "$READ" "## Updating"
assert_contains "README documents make update"   "$READ" "make update"
assert_contains "README documents make watch"    "$READ" "make watch"
```

- [ ] **Step 2: Run the suite to verify it fails**

Run: `bash tests/test_update_flow.sh`
Expected: FAIL — the three README assertions fail (section not present yet).

- [ ] **Step 3: Insert the section into `README.md`**

In `README.md`, find the line `## Using It Globally with Claude Code` and insert the following block **immediately before** it:

```markdown
## Updating & Development

Already installed and just want your local changes live? The two artifacts update very differently:

- **The binary** (`src/*.rs`) hot-swaps instantly — the Notification hook spawns a fresh process on every event, so a rebuilt binary is used on the very next notification. No restart.
- **The GNOME extension** (`extension.js`) needs GNOME Shell to reload it. On **Wayland** that means **log out and back in** (`Alt+F2 → r` is X11-only). The tooling only nags you to relogin when the extension actually changed.

A `Makefile` wraps `scripts/install.sh` with granular targets:

| Command | What it does | When |
|---|---|---|
| `make update` | rebuild + reinstall binary **and** extension | everyday "deploy what I changed" |
| `make bin` | rebuild + reinstall the binary only | the common Rust change — live instantly |
| `make ext` | reinstall the extension only | changed `extension.js` (prints relogin notice if needed) |
| `make install` | full install (binary + extension + config + hook + enable) | first-time setup |
| `make watch` | auto-rebuild + reinstall the binary on every source save | tight inner loop (needs `cargo install cargo-watch`) |
| `make uninstall` | remove everything (config preserved) | |
| `make check` | run the dependency-free test suite | before committing |
| `make help` | list all targets | |

`scripts/install.sh` still works directly with no arguments (full install) for anyone not using `make`; it also accepts `--bin` and `--ext`.
```

- [ ] **Step 4: Run the suite to verify it passes**

Run: `bash tests/test_update_flow.sh`
Expected: all pass, exit 0.

- [ ] **Step 5: Commit**

```bash
git add README.md tests/test_update_flow.sh
git commit -m "$(printf '%s\n' \
  'Document the update flow (Updating & Development section)' \
  '' \
  'Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Task 6: Full verification + real-world smoke test

Confirm the whole suite is green and the real (non-sandboxed) commands behave on this machine.

**Files:** none (verification only)

- [ ] **Step 1: Run the full test suite**

Run: `make check`
Expected: every section prints `ok`, final line `… passed, 0 failed`, exit 0.

- [ ] **Step 2: Real binary redeploy (non-destructive — binary only)**

Run: `make bin`
Expected: real `cargo build --release` runs, prints `==> Installing binary to …`, exits 0.
Run: `cmp -s target/release/claude-focus ~/.local/bin/claude-focus && echo "binary in sync"`
Expected: `binary in sync`.

- [ ] **Step 3: Real extension redeploy + smart notice (non-destructive)**

Run: `make ext`
Expected: since the installed extension already matches the repo, prints `Extension unchanged — nothing to reload.`

- [ ] **Step 4: Confirm `make help` and default goal**

Run: `make help` then `make`
Expected: both list the targets (default goal is `help`).

- [ ] **Step 5: Final state check**

Run: `git status --short && git log --oneline -7`
Expected: clean working tree; the Task 1–5 commits plus the spec commit are present on the `update-flow` branch.

---

## Self-Review (completed by plan author)

- **Spec coverage:** install.sh refactor + `--bin`/`--ext` + no-arg backward compat (Tasks 1–2 ✓); smart relogin notice (Task 3 ✓); Makefile `update`/`bin`/`ext`/`install`/`uninstall`/`watch`/`help` + watch scoping + graceful cargo-watch fallback (Task 4 ✓); README Updating/Development section (Task 5 ✓); spec's Verification items mapped to automated tests + Task 6 smoke ✓.
- **Additions beyond the spec's literal "Files touched":** `tests/test_update_flow.sh` and a `make check` target — required to satisfy TDD and to make the spec's Verification section repeatable; dependency-free (no bats), consistent with the project's minimalism ethos and a small down-payment on roadmap Phase 3 (CI/tests).
- **Placeholder scan:** none — every code/command/expected-output is concrete.
- **Type/name consistency:** function names (`do_build`/`do_bin`/`do_ext`/`do_config`/`do_hook`/`do_enable`/`print_full_summary`/`usage`/`main`), env vars (`PROJECT_DIR`/`BIN_DIR`/`EXT_DIR`/`CONFIG_DIR`/`SETTINGS_FILE`), and helper names (`make_sandbox`/`run_install`/`assert_*`) are used identically across all tasks.
- **Known caveat:** Task 1's red state is verified by inspection, not execution, because the pre-refactor script is destructive/unsandboxable — documented in the Testing strategy and Task 1 Step 2.
