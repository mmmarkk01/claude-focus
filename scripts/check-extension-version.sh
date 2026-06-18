#!/usr/bin/env bash
# Enforce the metadata.json version-bump rule documented in README "Versioning":
# if extension/extension.js changed versus the given base ref, the integer
# `version` in extension/metadata.json must have been incremented.
#
# Usage: check-extension-version.sh <base-ref>
#   <base-ref>  a git ref to diff against (e.g. origin/staging, or FETCH_HEAD in CI)
#
# Fail-safe: if no base ref is given, the ref can't be resolved, or the base
# blob can't be read, the check SKIPS (exit 0) rather than blocking — it only
# ever fails on a clear, real violation.
set -euo pipefail

BASE="${1:-}"
EXT_JS="extension/extension.js"
META="extension/metadata.json"

if [ -z "$BASE" ]; then
    echo "check-extension-version: no base ref given; skipping." >&2
    exit 0
fi

if ! git rev-parse --verify --quiet "$BASE" >/dev/null; then
    echo "check-extension-version: base ref '$BASE' not found; skipping." >&2
    exit 0
fi

# Nothing to enforce unless extension.js actually changed vs the base.
if git diff --quiet "$BASE" -- "$EXT_JS"; then
    echo "check-extension-version: $EXT_JS unchanged vs $BASE — nothing to enforce."
    exit 0
fi

# Extract the integer `version` from a metadata.json source. The grep keys off
# the quoted "version" field, so the "shell-version" array can't false-match.
# (tests/version_truth.rs guarantees this field stays an integer.)
extract_version() {
    local content
    if [ "$1" = "WORKTREE" ]; then
        content="$(cat "$META")"
    else
        content="$(git show "$1:$META" 2>/dev/null || true)"
    fi
    printf '%s' "$content" \
        | grep -oE '"version"[[:space:]]*:[[:space:]]*[0-9]+' \
        | grep -oE '[0-9]+' \
        | head -n1 || true
}

base_v="$(extract_version "$BASE")"
head_v="$(extract_version WORKTREE)"

if [ -z "$base_v" ]; then
    echo "check-extension-version: couldn't read base $META version; skipping." >&2
    exit 0
fi
if [ -z "$head_v" ]; then
    echo "check-extension-version: couldn't read current $META version." >&2
    exit 1
fi

if [ "$head_v" -gt "$base_v" ]; then
    echo "check-extension-version: OK — $EXT_JS changed and $META version bumped $base_v → $head_v."
    exit 0
fi

echo "check-extension-version: FAIL" >&2
echo "  $EXT_JS changed vs $BASE but $META version did not increase ($base_v → $head_v)." >&2
echo "  Bump the integer \"version\" in $META (see README 'Versioning')." >&2
exit 1
