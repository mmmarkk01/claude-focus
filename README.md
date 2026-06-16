# Claude Focus

Auto-focus your terminal and play a sound alert when Claude Code needs your attention. Never miss a permission prompt, question, or idle input request again.

## What It Does

When Claude Code needs input — a permission prompt, a question, or it's waiting idle — **claude-focus** will:

1. **Send a desktop notification** with a contextual title (e.g. "Permission Required", "Ready for Input")
2. **Play a sound alert** so you hear it even if your screen is off or you're looking elsewhere
3. **Bring the terminal to the foreground** (auto-focus, requires GNOME Shell extension — see below)

All three behaviors are independently configurable.

## How It Works

Claude Code supports [hooks](https://docs.anthropic.com/en/docs/claude-code/hooks) — shell commands that run in response to lifecycle events. Claude-focus registers itself as a **Notification hook** in `~/.claude/settings.json`. Whenever Claude Code emits a notification event, it pipes a JSON payload to `claude-focus` via stdin.

The flow:

```
Claude Code emits notification
    → pipes JSON to claude-focus via stdin
        → reads config from ~/.config/claude-focus/config.toml
        → checks if notification_type matches configured types
        → attempts auto-focus via GNOME Shell extension (D-Bus)
        → sends desktop notification via notify-send
        → plays sound via pw-play (PipeWire) or paplay (PulseAudio)
        → always exits 0 (never blocks Claude Code)
```

### Architecture

```
┌─────────────────────┐
│     Claude Code     │
│                     │
│  Notification hook  │──stdin JSON──▶┌──────────────┐
│                     │               │ claude-focus  │
└─────────────────────┘               │  (Rust CLI)   │
                                      └──┬───┬───┬───┘
                                         │   │   │
                          ┌──────────────┘   │   └──────────────┐
                          ▼                  ▼                  ▼
                    ┌───────────┐    ┌──────────────┐    ┌─────────────┐
                    │  gdbus →  │    │ notify-send  │    │  pw-play /  │
                    │  GNOME    │    │ (desktop     │    │  paplay     │
                    │  Shell    │    │  notification│    │  (sound)    │
                    │  Extension│    │  )           │    │             │
                    └───────────┘    └──────────────┘    └─────────────┘
                    Auto-focus        Visual alert        Audio alert
```

**Three components:**

- **Rust CLI** (`~/.local/bin/claude-focus`) — the hook binary. Reads JSON from stdin, loads config, dispatches notifications and focus requests. No runtime dependencies beyond standard Linux tools.
- **GNOME Shell Extension** (`focus-by-pid@claude.local`) — runs inside GNOME Shell, exposes a D-Bus method to activate a window by PID. Required for auto-focus on Wayland (direct window activation is not possible from outside the compositor).
- **Config file** (`~/.config/claude-focus/config.toml`) — controls all behavior.

### Process Tree Walking (tmux-aware)

To auto-focus, claude-focus needs to find the terminal window's PID. It walks `/proc/<pid>/status` upward from its own PID, checking each process name against known terminals (gnome-terminal, kitty, alacritty, wezterm, foot, konsole, etc.).

If it encounters a **tmux server** in the process tree, it queries `tmux list-clients` to find the client PID and continues walking from there. A visited-PID set prevents infinite cycles.

## Requirements

- **Linux** with GNOME desktop (tested on GNOME Shell 46, Ubuntu 24.04)
- **Rust toolchain** (for building)
- **notify-send** (usually pre-installed on GNOME)
- **pw-play** (PipeWire) or **paplay** (PulseAudio) for sound alerts
- **Claude Code** with hooks support

## Installation

### Quick Install

```bash
git clone <repo-url> ~/git_repos/claude-focus
cd ~/git_repos/claude-focus
./scripts/install.sh
```

The install script will:

1. Build the Rust binary (`cargo build --release`)
2. Copy it to `~/.local/bin/claude-focus`
3. Install the GNOME Shell extension to `~/.local/share/gnome-shell/extensions/focus-by-pid@claude.local/`
4. Create the default config at `~/.config/claude-focus/config.toml` (skips if it already exists)
5. Merge the Notification hook into `~/.claude/settings.json` (safe merge, won't overwrite existing settings)
6. Enable the GNOME Shell extension

### Post-Install

- **Desktop notifications and sound** work immediately
- **Auto-focus** requires a logout/login (or GNOME Shell restart) to load the extension

### Verify the Extension (after re-login)

```bash
gnome-extensions info focus-by-pid@claude.local

# Test the D-Bus interface
gdbus introspect --session \
    --dest org.gnome.Shell.Extensions.FocusByPid \
    --object-path /org/gnome/Shell/Extensions/FocusByPid
```

## Updating & Development

Already installed and just want your local changes live? The two artifacts update very differently:

- **The binary** (`src/*.rs`) hot-swaps instantly — the Notification hook spawns a fresh process on every event, so a rebuilt binary is used on the very next notification. No restart.
- **The GNOME extension** (`extension.js`) needs GNOME Shell to reload it. On **Wayland** that means **log out and back in** (`Alt+F2 → r` is X11-only). The tooling only nags you to log back in when the extension actually changed.

A `Makefile` wraps `scripts/install.sh` with granular targets:

| Command | What it does | When |
|---|---|---|
| `make update` | rebuild + reinstall binary **and** extension | everyday "deploy what I changed" |
| `make bin` | rebuild + reinstall the binary only | the common Rust change — live instantly |
| `make ext` | reinstall the extension only | changed `extension.js` (prints log-out/in notice if needed) |
| `make install` | full install (binary + extension + config + hook + enable) | first-time setup |
| `make watch` | auto-rebuild + reinstall the binary on every source save | tight inner loop (needs `cargo install cargo-watch`) |
| `make uninstall` | remove everything (config preserved) | |
| `make check` | run the dependency-free test suite | before committing |
| `make help` | list all targets | |

`scripts/install.sh` still works directly with no arguments (full install) for anyone not using `make`; it also accepts `--bin` and `--ext`.

## Using It Globally with Claude Code

The install script configures claude-focus as a **global** hook — it applies to all Claude Code sessions, in every project directory. The hook is registered in your user-level settings file:

**`~/.claude/settings.json`**
```json
{
  "hooks": {
    "Notification": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "/home/<you>/.local/bin/claude-focus"
          }
        ]
      }
    ]
  }
}
```

The `"matcher": "*"` means it triggers for all notification types. Filtering is handled by claude-focus itself via the config file, so you only need one hook entry.

### Manual Setup (if not using install.sh)

If you prefer to set it up manually, add the hook to `~/.claude/settings.json`:

```bash
# Build
cd ~/git_repos/claude-focus
cargo build --release

# Copy binary
cp target/release/claude-focus ~/.local/bin/

# Add to Claude Code settings (edit manually or use jq)
# Add the hooks.Notification entry shown above to ~/.claude/settings.json
```

### Per-Project Override

If you want different behavior for a specific project, create a `.claude/settings.json` in that project's root:

```json
{
  "hooks": {
    "Notification": [
      {
        "matcher": "permission_prompt",
        "hooks": [
          {
            "type": "command",
            "command": "/home/<you>/.local/bin/claude-focus"
          }
        ]
      }
    ]
  }
}
```

## Configuration

Edit `~/.config/claude-focus/config.toml`:

```toml
# Mode: "both", "focus-only", "notify-only"
mode = "both"

# Which notification types trigger action
# Available: permission_prompt, idle_prompt, auth_success, elicitation_dialog
notify_types = ["permission_prompt", "idle_prompt", "elicitation_dialog"]

# Desktop notification timeout (ms)
notification_timeout_ms = 5000

# Sound alert
play_sound = true
sound_file = "/usr/share/sounds/freedesktop/stereo/bell.oga"
```

### Options

| Option | Values | Default | Description |
|---|---|---|---|
| `mode` | `"both"`, `"focus-only"`, `"notify-only"` | `"both"` | What actions to take |
| `notify_types` | Array of type strings | `["permission_prompt", "idle_prompt", "elicitation_dialog"]` | Which notification types to act on |
| `notification_timeout_ms` | Integer (ms) | `5000` | How long the desktop notification stays visible |
| `play_sound` | `true` / `false` | `false` | Whether to play an audio alert |
| `sound_file` | File path | `bell.oga` | Path to the `.oga` sound file |

### Notification Types

| Type | When It Fires |
|---|---|
| `permission_prompt` | Claude Code needs permission to run a tool (file edit, bash command, etc.) |
| `idle_prompt` | Claude Code has finished and is waiting for your next message |
| `elicitation_dialog` | Claude Code is asking you a question |
| `auth_success` | Authentication completed |

### Available Sounds

The default freedesktop sounds are at `/usr/share/sounds/freedesktop/stereo/`. Some good options:

| Sound | File |
|---|---|
| Bell (default) | `bell.oga` |
| Complete | `complete.oga` |
| Warning | `dialog-warning.oga` |
| Window attention | `window-attention.oga` |
| Alarm | `alarm-clock-elapsed.oga` |

Preview them with: `pw-play /usr/share/sounds/freedesktop/stereo/bell.oga`

## Testing

```bash
# Test notification + sound (works immediately)
echo '{"notification_type":"permission_prompt","message":"Test alert"}' | ~/.local/bin/claude-focus

# Test with different notification types
echo '{"notification_type":"idle_prompt","message":"Claude is waiting"}' | ~/.local/bin/claude-focus

# Test that ignored types are silent
echo '{"notification_type":"auth_success","message":"Logged in"}' | ~/.local/bin/claude-focus
```

## Uninstalling

```bash
cd ~/git_repos/claude-focus
./scripts/uninstall.sh
```

This removes the binary, GNOME extension, and hook from Claude Code settings. Your config file at `~/.config/claude-focus/config.toml` is preserved (delete it manually if you want).

## Troubleshooting

**No sound?**
- Check that `play_sound = true` in your config
- Test `pw-play` directly: `pw-play /usr/share/sounds/freedesktop/stereo/bell.oga`
- If `pw-play` isn't found, install PipeWire tools or PulseAudio (`sudo apt install pulseaudio-utils`)
- Some sound files are very quiet — try `bell.oga` or `alarm-clock-elapsed.oga`

**No notification popup?**
- Test `notify-send` directly: `notify-send "Test" "Hello"`
- Check that Do Not Disturb is off in GNOME settings

**Auto-focus not working?**
- Log out and back in after installing (required to load the GNOME Shell extension)
- Verify the extension is enabled: `gnome-extensions info focus-by-pid@claude.local`
- Check extension logs: `journalctl /usr/bin/gnome-shell -f`
- Auto-focus only works on GNOME with Wayland

**Hook not triggering?**
- Verify the hook is in `~/.claude/settings.json`
- Check that `~/.local/bin/claude-focus` exists and is executable
- Run `claude-focus` manually with test JSON to check for errors

## Project Structure

```
claude-focus/
├── Cargo.toml                  # Rust dependencies (serde, serde_json, toml)
├── src/
│   ├── main.rs                 # Entry point — reads stdin JSON, dispatches
│   ├── config.rs               # TOML config loading and defaults
│   ├── process_tree.rs         # /proc walker to find terminal PID (tmux-aware)
│   ├── dbus.rs                 # gdbus call to GNOME Shell extension
│   └── notify.rs               # notify-send + pw-play/paplay
├── extension/
│   ├── metadata.json           # GNOME Shell extension metadata
│   └── extension.js            # D-Bus service for window activation by PID
├── config/
│   └── claude-focus.toml       # Default config template
└── scripts/
    ├── install.sh              # Build + install everything
    └── uninstall.sh            # Remove everything
```
