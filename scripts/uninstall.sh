#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="$HOME/.local/bin"
EXT_DIR="$HOME/.local/share/gnome-shell/extensions/focus-by-pid@claude.local"
CONFIG_DIR="$HOME/.config/claude-focus"
SETTINGS_FILE="$HOME/.claude/settings.json"

echo "==> Disabling GNOME Shell extension..."
if command -v gnome-extensions &>/dev/null; then
    gnome-extensions disable focus-by-pid@claude.local 2>/dev/null || true
fi

echo "==> Removing binary..."
rm -f "$BIN_DIR/claude-focus"

echo "==> Removing GNOME Shell extension..."
rm -rf "$EXT_DIR"

echo "==> Removing hook from Claude Code settings..."
if [ -f "$SETTINGS_FILE" ]; then
    python3 -c "
import json

settings_file = '$SETTINGS_FILE'
hook_command = '$BIN_DIR/claude-focus'

with open(settings_file) as f:
    settings = json.load(f)

hooks = settings.get('hooks', {})
notifications = hooks.get('Notification', [])

# Filter out entries containing our hook command
filtered = [
    entry for entry in notifications
    if not any(h.get('command') == hook_command for h in entry.get('hooks', []))
]

if len(filtered) != len(notifications):
    hooks['Notification'] = filtered
    # Clean up empty structures
    if not hooks['Notification']:
        del hooks['Notification']
    if not hooks:
        del settings['hooks']
    with open(settings_file, 'w') as f:
        json.dump(settings, f, indent=2)
    print('    Hook removed from', settings_file)
else:
    print('    Hook not found, skipping')
"
fi

echo ""
echo "Uninstalled. Config left at $CONFIG_DIR/config.toml (remove manually if desired)."
echo "You may need to log out/in to fully unload the GNOME Shell extension."
