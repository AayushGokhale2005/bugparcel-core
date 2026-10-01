#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
TMP=$(mktemp -d)
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

if ! command -v codex >/dev/null 2>&1; then
  echo 'codex unavailable; skipping Codex MCP scope smoke test'
  exit 0
fi

REPO="$TMP/source"
git init -q "$REPO"
git -C "$REPO" config user.email benchmark@example.invalid
git -C "$REPO" config user.name benchmark
touch "$REPO/.keep"
git -C "$REPO" add .keep
git -C "$REPO" commit -qm initial

(cd "$REPO" && cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel -- mcp install --development --store "$TMP/store")
(cd "$REPO" && codex mcp get bugparcel > "$TMP/mcp-get.txt")
grep -q 'bugparcel' "$TMP/mcp-get.txt"
(cd "$REPO" && cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel -- mcp doctor > "$TMP/doctor.txt")
grep -q 'MCP_DISCOVERABLE' "$TMP/doctor.txt"

echo 'codex repository-scoped MCP smoke test passed'
