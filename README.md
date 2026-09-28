# BugParcel local core

The implementation workspace for BugParcel's local-first CLI and daemon-era core. The landing page remains at the repository root; this directory intentionally owns product behavior.

## Current vertical slice

- Versioned parcel manifests and append-only status transitions.
- Content-addressed artifact storage using SHA-256.
- Exact Git snapshot capture (HEAD, branch hint, staged and unstaged diffs).
- Generic process reproduction with a stable exit-code assertion.
- CLI workflow: `capture`, `show`, `reproduce`, and `verify`.

This is the Phase 0–1 foundation from the technical specification. It does not yet implement cloud sharing, HTTP capture, MCP, or container isolation.

```bash
cargo run -p bugparcel -- capture --name "failing-auth" -- npm test -- auth
```

## Prove the vertical slice

The integration fixture is an intentionally dirty Git repository. It proves that
BugParcel captures its exact state, replays it in a detached worktree, rejects a
patch that leaves the failure intact, and accepts the patch that repairs it.

```sh
cargo test --workspace
sh tests/integration/dirty_state_e2e.sh
```

## FastAPI sandbox

Use the independent [FastAPI sandbox](../../bugparcel-fastapi-sandbox) to test
against a real application and an uncommitted checkout regression. It is a
separate Git repository so its dirty state can be captured without affecting
this core repository.
