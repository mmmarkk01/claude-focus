# Claude Focus — Local Update Flow

- **Date:** 2026-06-06
- **Status:** Approved (brainstorming) — pending implementation plan
- **Branch:** `update-flow` (off `main`)

## Problem

Getting local changes "live" during development means re-running `./scripts/install.sh`,
which rebuilds and re-copies *everything*, re-runs the settings merge, and always prints
the log-out reminder — even when only the Rust binary changed. There is no fast, granular,
discoverable way to redeploy just what changed. In this project the developer iterates on
the Rust binary and the GNOME extension about equally, so both paths need to be good.

## Key constraint: two artifacts, two deploy costs

The tool ships two deployable artifacts with very different update stories:

| Artifact | Deploy cost | Restart needed? |
|---|---|---|
| Binary (`src/*.rs` → `~/.local/bin/claude-focus`) | rebuild + copy (~1–2s) | **None** — the Notification hook spawns a fresh process per event, so a rebuilt binary is live on the next notification |
| Extension (`extension.js` / `metadata.json`) | copy is trivial | **Log out / log in** — GNOME Shell must reload it; on Wayland (this machine: GNOME Shell 46) `Alt+F2 → r` is X11-only and unavailable |

The extension logout is unavoidable on Wayland. The design **minimizes** the nag (only
prompts when the extension actually changed); it cannot remove the logout itself.

## Goals

- One quick command to deploy local changes: `make update`.
- Instant path for the common Rust case: `make bin`.
- Honest, low-noise extension path: `make ext` — only prompts for a relogin when the
  extension files actually changed.
- Optional hands-off binary redeploy on save: `make watch`.
- Discoverable (`make help`) and documented in the README.
- Zero new **required** dependencies. Fully backward compatible with today's `install.sh`.

## Non-goals (YAGNI)

- **Git post-merge hook** (auto-deploy after `git pull`) — deferred; easy future add.
- **Nested-shell extension testing** target (`gnome-shell --nested`) — deferred; easy future add.
- **Config-template propagation** — intentionally still skipped, to protect the developer's
  customized `~/.config/claude-focus/config.toml`.
- **Binary self-diagnosis features** (`test` / `doctor` / `--version`) — separate roadmap work
  (see `dx-roadmap` branch).

## Design

### 1. `scripts/install.sh` — refactor into functions + flags

Factor the existing inline steps into shell functions: `do_build`, `do_bin`, `do_ext`,
`do_config`, `do_hook`, `do_enable`. Add argument parsing:

- **(no args)** → full install: `do_build; do_bin; do_ext; do_config; do_hook; do_enable`
  followed by the existing summary banner. **Identical to today's behavior** — the README,
  the documented manual flow, and existing muscle memory all keep working.
- `--bin` → `do_build; do_bin`
- `--ext` → `do_ext`
- Flags combine (e.g. `--bin --ext`).
- `--help` / `-h` → usage text.
- Unknown flag → error to stderr + usage, exit 1.

**`do_ext` smart notice:**

1. Before overwriting, `cmp` the repo's `extension.js` and `metadata.json` against the
   installed copies under `~/.local/share/gnome-shell/extensions/focus-by-pid@claude.local/`.
2. Copy the files.
3. If either differed (or an installed copy was absent) → print
   `Log out/in to load the updated extension.`
   Otherwise → print `Extension unchanged — nothing to reload.`

`do_ext` assumes the extension directory already exists and is enabled (first-time enable
lives in `do_enable`, reached only by the full install).

**Output:** partial runs (`--bin` / `--ext`) print concise per-step results; the full,
no-args install keeps the existing summary banner. Preserve `set -euo pipefail`.

### 2. `Makefile` (repo root)

`SHELL := bash`, `.DEFAULT_GOAL := help`, all targets `.PHONY`. Each target carries a
`## description` comment that the `help` target greps to self-document.

| Target | Runs | When to use |
|---|---|---|
| `make help` | prints targets + descriptions | default goal |
| `make update` | `./scripts/install.sh --bin --ext` | everyday "deploy what I changed" (binary live now; extension after relogin) |
| `make bin` | `./scripts/install.sh --bin` | the 90% Rust path — **live instantly** |
| `make ext` | `./scripts/install.sh --ext` | changed `extension.js` (prints logout notice if needed) |
| `make install` | `./scripts/install.sh` | first-time / config + hook + enable |
| `make uninstall` | `./scripts/uninstall.sh` | remove everything |
| `make watch` | guarded `cargo watch -w src -w Cargo.toml -s './scripts/install.sh --bin'` | auto-redeploy the binary on every source save |

`make watch` first checks `command -v cargo-watch`; if missing it prints
`Install with: cargo install cargo-watch` and exits non-zero (cargo-watch is an **optional**
dev dependency, never required). The watch is scoped to `src/` and `Cargo.toml` (`-w`) so
edits to docs, the README, or the extension do not trigger a binary rebuild; `target/` is
git-ignored, so the loop also never re-triggers on its own build output.

### 3. `README.md`

Add an **Updating / Development** subsection under Installation documenting:

- The `make` targets (`update` / `bin` / `ext` / `install` / `watch`) and when to use each.
- The binary-is-instant vs extension-needs-logout distinction.
- That bare `./scripts/install.sh` still works unchanged for anyone not using `make`.

## Verification

- `make help` lists every target with its description.
- `make bin` → installed `~/.local/bin/claude-focus` is byte-identical to a fresh
  `cargo build --release` output (`cmp` matches).
- `make ext` with no change → "Extension unchanged — nothing to reload"; after flipping one
  byte in `extension.js` → the logout notice appears.
- `make update` performs both the binary and extension steps.
- Bare `./scripts/install.sh` (no flags) → full idempotent install, behavior unchanged
  (backward-compatibility check).
- `make watch` without cargo-watch installed → prints the install hint and exits non-zero,
  no crash or stack trace.
- `./scripts/uninstall.sh` → unaffected by the refactor.

## Files touched

- `scripts/install.sh` — refactor into functions + add flag parsing and the smart notice.
- `Makefile` — new, at repo root.
- `README.md` — new Updating / Development section.
