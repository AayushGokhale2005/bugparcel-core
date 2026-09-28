#!/bin/sh

# The committed state passes. The intentional uncommitted change changes
# MODE=ok to MODE=broken and creates the captured failure.
if grep -q '^MODE=broken$' state.env; then
  echo 'checkout fixture failed: MODE=broken' >&2
  exit 1
fi

echo 'checkout fixture passed'
