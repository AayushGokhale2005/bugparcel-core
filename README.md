# BugParcel local core

The implementation workspace for BugParcel's local-first CLI and daemon-era core. The landing page remains at the repository root; this directory intentionally owns product behavior.

## Install (Homebrew)

```bash
brew tap AayushGokhale2005/bugparcel
brew trust AayushGokhale2005/bugparcel   # Homebrew 7+
brew install bugparcel
bugparcel --help
```

This installs `bugparcel` and `bugparcel-mcp` on your PATH. Point MCP clients at `bugparcel-mcp` and set `BUGPARCEL_HOME` to your parcel store.

### Give coding agents access

Run this from the **source repository an agent will work in** (not from a
parent monorepo):

```sh
bugparcel mcp install
bugparcel mcp doctor
```

The installer creates a repository-scoped `.codex/config.toml` that starts
`bugparcel-mcp` and exposes that repository's `.bugparcel` store. Codex does
not inherit a parent repository's MCP configuration when an agent is started
inside a nested checkout, so this step is required for each source repository.

For local BugParcel development before the binaries are installed, use:

```sh
cargo run -p bugparcel -- mcp install --development
cargo run -p bugparcel -- mcp doctor
```

## Current vertical slice

- Versioned parcel manifests and append-only status transitions.
- Content-addressed artifact storage using SHA-256.
- Exact Git snapshot capture (HEAD, branch hint, staged and unstaged diffs).
- Generic process reproduction with a stable exit-code assertion, independent of framework or language.
- CLI workflow: `capture`, `show`, `reproduce`, and `verify`.

This local-first foundation does not yet implement cloud sharing or container isolation.

```bash
cargo run -p bugparcel -- capture --name "failing-auth" -- npm test -- auth
```

## Enterprise remote

Associate the current workspace with a hosted BugParcel project before sharing
parcels:

```sh
bugparcel add-remote https://bugparcel-enterprise.vercel.app/projects/<project>
```

The project URL is stored locally at `.bugparcel/enterprise/remote.json`.

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

`bugparcel-mcp` is a local stdio MCP server for coding agents. After
`brew install bugparcel`, configure clients to run the binary on PATH:

```json
{
  "mcpServers": {
    "bugparcel": {
      "command": "bugparcel-mcp",
      "env": {
        "BUGPARCEL_HOME": "./.bugparcel"
      }
    }
  }
}
```

From a source checkout you can still run:

```sh
BUGPARCEL_HOME=/path/to/parcel-store \
  cargo run --manifest-path Cargo.toml -p bugparcel-mcp
```

## Framework-neutral server capture

BugParcel's replay contract is not tied to FastAPI: the core CLI captures any
command that can reproduce a server failure. Node, Go, Java, Ruby, PHP, and
custom services can invoke the same portable contract from their error hook:

```sh
bugparcel capture --name checkout-500 \
  --expect-output ZeroDivisionError \
  --contract-file /tmp/bugparcel-event.json \
  --state-file /tmp/bugparcel-event.json \
  --state-json-pointer /request/body \
  --env BUGPARCEL_CAPTURED_FROM=express \
  -- npm test -- checkout-regression
```

The event file is ordinary sanitized JSON with `request`, `error`, and
`reproduction_command` fields. It is a language-neutral boundary: adapters only
need to write that event and invoke the local CLI; replay and verification stay
identical for every backend.

For Python servers, install the dependency-free generic adapter directly from a
source checkout until it is published:

```sh
pip install -e sdks/python/bugparcel-capture
```

It provides manual capture plus ASGI and WSGI wrappers. ASGI covers FastAPI,
Starlette, Django ASGI, Quart, and custom ASGI apps. WSGI covers Flask, Django
WSGI, Bottle, and custom WSGI apps.

```python
from pathlib import Path

from bugparcel_capture import BugParcelASGIMiddleware, BugParcelSettings

app = BugParcelASGIMiddleware(
    app,
    settings=BugParcelSettings(
        project_root=Path("/path/to/service"),
        reproduction_command=["python", "-m", "pytest", "-q"],
        parcel_store=Path("/path/to/bugparcel-store"),
        cli_command=["bugparcel"],
    ),
)
```

Each wrapper redacts sensitive headers, query keys, and JSON body fields, skips
capture during `BUGPARCEL_REPLAY=1`, and never replaces the application error
if capture itself fails.

## FastAPI convenience adapter

The Python adapter writes a sanitized request/error event and invokes the local
CLI when FastAPI raises an unhandled exception. It is automatically disabled in
isolated replays through `BUGPARCEL_REPLAY=1`. Its captured contract requires
both the expected exit code and the original exception type in command output.

## Environment isolation — Phase 4

Every capture now records a Python runtime (when the repro command starts with
Python), SHA-256 lockfile digests, and only explicitly allowlisted environment
variables. Use `--env KEY=VALUE` to include a variable. Docker replay is
optional: declare an image that already contains the runtime dependencies and
BugParcel runs the reproduction in a network-isolated disposable container.

```sh
bugparcel capture --name checkout \
  --env LOG_LEVEL=debug \
  --docker-image ghcr.io/example/checkout-test:sha-abc123 \
  -- python3 -m pytest -q
```

## Stateful request reduction — Phase 5

The FastAPI adapter captures a sanitized JSON request body as parcel state.
After reproduction, reduce it with a contract-preserving search; the test command
receives each candidate in `BUGPARCEL_STATE_JSON`.

```sh
bugparcel reduce <parcel-id> --output minimized-request.json
```

The reducer removes object fields and array items only when the complete failure
contract still matches. Docker-backed reduction is intentionally deferred until
the state-injection protocol is available inside containers.

SQLite and other file-backed fixtures can be made portable with `--fixture-file`.
BugParcel records the file contents and SHA-256 digest, then restores it at the
same repository-relative path in each detached replay worktree.

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
