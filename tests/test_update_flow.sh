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

echo ""
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
