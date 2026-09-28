#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
FIXTURE="$ROOT/fixtures/dirty-state-repo"
STORE=$(mktemp -d)
PATCH_DIR=$(mktemp -d)

ensure_fixture_repository() {
  if git -C "$FIXTURE" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    return
  fi

  git -C "$FIXTURE" init -q -b main
  git -C "$FIXTURE" config user.name "BugParcel Test Fixture"
  git -C "$FIXTURE" config user.email "bugparcel-test@example.invalid"
  git -C "$FIXTURE" add README.md check.sh state.env
  git -C "$FIXTURE" commit -qm "fixture: passing baseline"
}

cleanup() {
  rm -rf "$STORE" "$PATCH_DIR"
  git -C "$FIXTURE" worktree prune 2>/dev/null || true
}
trap cleanup EXIT

ensure_fixture_repository
printf 'MODE=broken\n' > "$FIXTURE/state.env"
parcel_output=$(cd "$FIXTURE" && BUGPARCEL_HOME="$STORE" cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel -- capture --name dirty-state -- sh check.sh)
parcel_id=${parcel_output%% *}
cd "$FIXTURE" && BUGPARCEL_HOME="$STORE" cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel -- reproduce "$parcel_id" | grep '"matched": true'

# An empty patch preserves the failure.
: > "$PATCH_DIR/wrong.patch"
cd "$FIXTURE" && BUGPARCEL_HOME="$STORE" cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel -- verify "$parcel_id" --patch "$PATCH_DIR/wrong.patch" | grep '"verify_failed"'

# The correct patch repairs the captured dirty state in the verification worktree.
# Let Git produce the patch so the fixture tests the same format users will submit.
printf 'MODE=broken\n' > "$PATCH_DIR/broken-state.env"
printf 'MODE=ok\n' > "$PATCH_DIR/fixed-state.env"
diff -u -L a/state.env -L b/state.env \
  "$PATCH_DIR/broken-state.env" "$PATCH_DIR/fixed-state.env" > "$PATCH_DIR/correct.patch" || test $? -eq 1

# A new reproducible parcel verifies the correct patch in a fresh worktree.
parcel_output=$(cd "$FIXTURE" && BUGPARCEL_HOME="$STORE" cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel -- capture --name dirty-state-correct -- sh check.sh)
parcel_id=${parcel_output%% *}
cd "$FIXTURE" && BUGPARCEL_HOME="$STORE" cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel -- reproduce "$parcel_id" | grep '"matched": true'
cd "$FIXTURE" && BUGPARCEL_HOME="$STORE" cargo run -q --manifest-path "$ROOT/Cargo.toml" -p bugparcel -- verify "$parcel_id" --patch "$PATCH_DIR/correct.patch" | grep '"verified"'

printf 'MODE=broken\n' > "$FIXTURE/state.env"
echo 'dirty-state e2e passed'
