#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BIN_DIR="$HOME/.local/bin"
EXT_DIR="$HOME/.local/share/gnome-shell/extensions/focus-by-pid@claude.local"
CONFIG_DIR="$HOME/.config/claude-focus"
SETTINGS_FILE="$HOME/.claude/settings.json"

echo "==> Building claude-focus..."
cd "$PROJECT_DIR"
cargo build --release

echo "==> Installing binary to $BIN_DIR..."
mkdir -p "$BIN_DIR"
cp target/release/claude-focus "$BIN_DIR/claude-focus"
chmod +x "$BIN_DIR/claude-focus"

echo "==> Installing GNOME Shell extension..."
mkdir -p "$EXT_DIR"
cp extension/metadata.json "$EXT_DIR/"
cp extension/extension.js "$EXT_DIR/"

echo "==> Installing config..."
mkdir -p "$CONFIG_DIR"
if [ ! -f "$CONFIG_DIR/config.toml" ]; then
    cp config/claude-focus.toml "$CONFIG_DIR/config.toml"
    echo "    Created $CONFIG_DIR/config.toml"
else
    echo "    Config already exists, skipping"
fi

echo "==> Merging hook into Claude Code settings..."
mkdir -p "$(dirname "$SETTINGS_FILE")"
python3 -c "
import json, os, sys

settings_file = '$SETTINGS_FILE'
hook_command = '$BIN_DIR/claude-focus'

# Load existing settings or start fresh
if os.path.exists(settings_file):
    with open(settings_file) as f:
        settings = json.load(f)
else:
    settings = {}

# Build the hook entry
hook_entry = {
    'matcher': '*',
    'hooks': [
        {
            'type': 'command',
            'command': hook_command
        }
    ]
}

# Merge into hooks.Notification
hooks = settings.setdefault('hooks', {})
notifications = hooks.setdefault('Notification', [])

# Check if already present
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
"

echo "==> Enabling GNOME Shell extension..."
if command -v gnome-extensions &>/dev/null; then
    gnome-extensions enable focus-by-pid@claude.local 2>/dev/null || true
    echo "    Extension enabled (may require log out/in to take effect)"
else
    echo "    gnome-extensions not found, skip enabling"
fi

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
