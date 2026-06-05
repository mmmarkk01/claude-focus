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

main "$@"
