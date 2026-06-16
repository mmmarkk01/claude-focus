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
  # Fake cargo writes an executable stub binary that answers `doctor` (so the
  # end-of-install doctor run is observable) and otherwise exits 0.
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
assert_contains "full install runs doctor"           "$OUT" "DOCTOR RAN"
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
# The merged file must be STRUCTURALLY valid JSON with the hook in place — a
# substring check alone would pass a non-atomic/truncating write. (atomicity gate)
PARSE_RC=0
python3 -c '
import json, sys
s = json.load(open(sys.argv[1]))
ok = s.get("otherKey") == 42 and any(
    h.get("command", "").endswith("claude-focus")
    for e in s["hooks"]["Notification"] for h in e.get("hooks", [])
)
sys.exit(0 if ok else 1)
' "$SETTINGS_FILE" || PARSE_RC=$?
assert_eq "merged settings is valid JSON with hook in place" "$PARSE_RC" "0"
assert_eq "no stray temp file after merge" \
  "$(find "$(dirname "$SETTINGS_FILE")" -name '.settings.*.tmp' | wc -l | tr -d ' ')" "0"
rm -rf "$SB"

echo "== install.sh: malformed settings leaves no stray temp file =="
make_sandbox
mkdir -p "$(dirname "$SETTINGS_FILE")"
printf '{ this is not valid json ' > "$SETTINGS_FILE"
run_install
assert_eq "no stray temp file after malformed abort" \
  "$(find "$(dirname "$SETTINGS_FILE")" -name '.settings.*.tmp' | wc -l | tr -d ' ')" "0"
rm -rf "$SB"

echo "== install.sh: re-install over an existing hook is idempotent =="
make_sandbox
run_install                                  # first install: hook appended
run_install                                  # second install: already present
assert_eq       "reinstall exits 0"               "$RC" "0"
assert_contains "reinstall says already present"  "$OUT" "already present"
assert_eq       "hook present exactly once"       "$(grep -c "$BIN_DIR/claude-focus" "$SETTINGS_FILE")" "1"
rm -rf "$SB"

echo "== install.sh: preflight hard-fails when cargo is missing =="
make_sandbox
# Build a clean PATH with the real tools install.sh needs but deliberately NO
# cargo — independent of where the real cargo lives. type -P bypasses any shell
# function/alias so the symlinks resolve to actual binaries.
CLEAN="$SB/cleanbin"; mkdir -p "$CLEAN"
for t in bash env sh dirname cp mkdir chmod cmp python3 grep cat mktemp rm; do
  p="$(type -P "$t" 2>/dev/null || true)"; [ -n "$p" ] && ln -sf "$p" "$CLEAN/$t"
done
cp "$SB/fakebin/gnome-extensions" "$CLEAN/gnome-extensions"   # present; NB: no cargo
OUT="$(PATH="$CLEAN" PROJECT_DIR="$PROJECT_DIR" BIN_DIR="$BIN_DIR" \
       EXT_DIR="$EXT_DIR" CONFIG_DIR="$CONFIG_DIR" SETTINGS_FILE="$SETTINGS_FILE" \
       bash "$INSTALL" 2>&1)"; RC=$?
assert_eq       "missing cargo exits 1"        "$RC" "1"
assert_contains "missing cargo names rustup"   "$OUT" "rustup"
assert_absent   "missing cargo builds nothing" "$BIN_DIR/claude-focus"
rm -rf "$SB"

echo "== Makefile: targets map to the right commands =="
cd "$REPO" || exit
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

echo "== README documents the update flow =="
READ="$(cat "$REPO/README.md")"
assert_contains "README has an Updating section" "$READ" "## Updating"
assert_contains "README documents make update"   "$READ" "make update"
assert_contains "README documents make watch"    "$READ" "make watch"

echo ""
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
