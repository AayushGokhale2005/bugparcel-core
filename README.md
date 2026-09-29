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

## Local MCP server — Phase 1

`bugparcel-mcp` is a local stdio MCP server for coding agents. It exposes four
tools: `bugparcel_list_parcels`, `bugparcel_get_parcel`,
`bugparcel_reproduce`, and `bugparcel_verify`.

```sh
BUGPARCEL_HOME=/path/to/parcel-store \
  cargo run --manifest-path Cargo.toml -p bugparcel-mcp
```

MCP client configuration:

```json
{
  "mcpServers": {
    "bugparcel": {
      "command": "cargo",
      "args": [
        "run", "--quiet", "--manifest-path",
        "/Users/aayushgokhale/Documents/BugParcel/bugparcel-core/Cargo.toml",
        "-p", "bugparcel-mcp"
      ],
      "env": {
        "BUGPARCEL_HOME": "/Users/aayushgokhale/Documents/bugparcel-runs"
      }
    }
  }
}
```

## FastAPI adapter — Phase 2

The Python adapter writes a sanitized request/error event and invokes the local
CLI when FastAPI raises an unhandled exception. It is automatically disabled in
isolated replays through `BUGPARCEL_REPLAY=1`.

```python
from pathlib import Path

from bugparcel_fastapi import BugParcelFastAPIMiddleware, BugParcelSettings

app.add_middleware(
    BugParcelFastAPIMiddleware,
    settings=BugParcelSettings(
        project_root=Path("/path/to/your/fastapi-project"),
        reproduction_command=["/path/to/.venv/bin/python", "-m", "pytest", "-q"],
        parcel_store=Path("/path/to/bugparcel-store"),
        cli_command=[
            "cargo", "run", "--quiet", "--manifest-path",
            "/Users/aayushgokhale/Documents/BugParcel/bugparcel-core/Cargo.toml",
            "-p", "bugparcel", "--",
        ],
    ),
)
```
