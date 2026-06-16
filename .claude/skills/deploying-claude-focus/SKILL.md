---
name: deploying-claude-focus
description: Use when deploying, installing, or redeploying local claude-focus changes (edits to src/*.rs, the GNOME extension extension.js, or config) so they run in the user's environment, and before telling the user a change is live — e.g. running make update/bin/ext or handling the Wayland log-out reload of the extension.
---

# Deploying claude-focus changes

## Overview

claude-focus has two deployable artifacts with very different deploy costs. Get this wrong and you'll tell the user a change is "live" when it isn't.

- **Binary** (`src/*.rs`): hot-swaps **instantly**. The Notification hook spawns a fresh process on every event, so a rebuilt binary is used on the next notification. No restart.
- **GNOME extension** (`extension/extension.js`, `metadata.json`): copying the file is **not enough**. GNOME Shell already has the old code loaded; on **Wayland** you cannot hot-reload (`Alt+F2 → r` is X11-only), so the user **must log out and back in**.

## Quick reference

Run from the repo root. `make help` lists every target; see the README "Updating & Development" section for detail.

| You changed | Run | Then the user must… |
|---|---|---|
| Rust (`src/*.rs`) | `make bin` | nothing — trigger a Claude Code notification to test |
| Extension (`extension.js`) | `make ext` | **log out / log in** (Wayland) before it's live |
| Both | `make update` | log out / log in (for the extension half) |
| First-time setup | `make install` | log out / log in once for the extension |

## The rule that prevents the silent failure

If you deployed an **extension** change, you MUST tell the user, in your reply, that they have to **log out and back in** for it to take effect — the change is NOT live until they do. Never report an extension change as done / live / ready-to-test without this.

`make ext` helps: it prints `Log out/in to load the updated extension.` when it detects a change, or `Extension unchanged — nothing to reload.` when the files already match.

## Gotchas

- Edit the **repo** file (`extension/extension.js`), not the installed copy under `~/.local/share/gnome-shell/extensions/focus-by-pid@claude.local/` — `make ext` overwrites the installed copy.
- A binary change needs a *fresh* notification to exercise; an already-running Claude session re-invokes the on-disk hook each event, so no Claude restart is needed.
- Run `make check` before committing (the dependency-free update-flow test suite).

## When NOT to use

First-time machine setup is just `make install` (then one log out / in). This skill is for the edit → redeploy loop during development.
