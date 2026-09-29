#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
STORE=$(mktemp -d)
cleanup() { rm -rf "$STORE"; }
trap cleanup EXIT

printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"bugparcel-test","version":"0.1.0"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bugparcel_list_parcels","arguments":{}}}' \
  | BUGPARCEL_HOME="$STORE" cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel-mcp \
  | grep -E '"serverInfo"|"bugparcel_reproduce"|"bugparcel_diagnose"|"bugparcel_propose_fix"|"structuredContent"'

echo 'mcp stdio handshake passed'
