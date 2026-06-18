# claude-focus — Phase 3 plan & summary (Lock it in)

**Date:** 2026-06-16
**Issue:** #31 (part of #28)
**Branch:** `dx-phase3` → `staging`
**Spec:** §3 of `docs/superpowers/specs/2026-06-05-claude-focus-dx-roadmap-design.md`

## Goal

A regression net (CI + tests) exists *before* the Phase 4 `Focuser` refactor, and the installed build is identifiable.

## Starting state (audited before implementing)

- 32 unit tests already passed; `cargo clippy -D warnings` was clean.
- **`cargo fmt --check` FAILED** on the committed Phase 1/2 code; no `.github/` CI existed.
- **3.2 was partly done**: the `notify_types`/empty-type filter tests (`main.rs`) and config parse-error tests (`config.rs`) already existed. The real gap was `process_tree.rs` (zero tests; the truncation match + tmux detection were inline and not unit-testable).
- **3.3's `--version` already existed** (shipped in Phase 1.5). Only the *documented mapping* + a check remained.

## Changes

### Pre-req — format the codebase

Ran `cargo fmt` to normalize `doctor.rs/main.rs/notify.rs/process_tree.rs`. Mechanical, no behavior change — required so the `fmt --check` gate can ever pass.

### 3.1 — CI (`.github/workflows/ci.yml`)

- **build-test** job (ubuntu-latest): `cargo fmt --check` → `clippy --all-targets -D warnings` → `build --release` → `cargo test` → release-binary smoke (pipes a `permission_prompt` payload with `PATH=` so helper spawns are no-ops, asserts exit 0) → `shellcheck scripts/*.sh`.
- **version-bump** job (PR-only): `scripts/check-extension-version.sh FETCH_HEAD`.
- Triggers: push to `main`/`staging`, all pull requests.

### 3.2 — Tests for the regression-prone logic

- Extracted pure fns `is_known_terminal(&str)` (exact + 15-char `TASK_COMM_LEN` truncation match) and `is_tmux(&str)` from `walk_tree`; behavior identical, now unit-testable.
- Table-driven tests: truncation match (`gnome-terminal-` → server), exact terminals, non-terminals, tmux server/client/bare/`tmuxinator`.
- `tests/smoke.rs`: runs the real binary — valid `permission_prompt`, malformed JSON, and empty stdin all exit 0. Locks the "always exit 0 / never block Claude Code" ethos. Hermetic via empty `PATH` + throwaway `HOME`.

### 3.3 — Single source of version truth

- README **Versioning** section: documents the two *incompatible* schemes (Cargo semver vs GNOME-mandated `metadata.json` integer) and the bump rule.
- `tests/version_truth.rs`: asserts `metadata.json` `version` is a positive integer and `CARGO_PKG_VERSION` is semver.
- `scripts/check-extension-version.sh`: PR guard — if `extension/extension.js` changed vs base, require the `metadata.json` integer to have incremented. Fail-safe (skips when base/blob unavailable).

## Decisions (confirmed with maintainer)

- **Included** the cross-manifest CI guard (the spec's optional piece).
- Commit type **`ci:`** (no release bump) — Phase 3 is infrastructure with no user-facing behavior change. (Phases 1–2 shipped as `feat:`.)
- Formatted the entire codebase (forced by the `fmt --check` gate).

## Verification

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (43 tests: 38 unit + 3 smoke + 2 version-truth), and `shellcheck scripts/*.sh` all green locally. The guard's three branches (unchanged / changed-not-bumped / changed-and-bumped) were exercised manually against the `staging` base.

## DRY pass

Extracted `is_known_terminal`/`is_tmux` from inline `walk_tree` conditionals (removed inline duplication, enabled testing); no new cross-module duplication introduced; integration-test helpers kept local per repo convention.
